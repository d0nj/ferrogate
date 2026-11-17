use axum::{extract::State, routing::put, Json, Router};
use sea_orm::{sea_query::OnConflict, EntityTrait, Set};
use serde::{Deserialize, Serialize};

use crate::{
    entities::app_setting,
    error::{AppError, AppResult},
    semantic::{self, THRESHOLD_KEY, THRESHOLD_RANGE},
    AppState,
};

#[derive(Debug, Serialize)]
pub struct SemanticStatsDto {
    pub entries: u64,
    pub hits: u64,
    pub threshold: f32,
    pub backend: &'static str,
    pub embedding_model: Option<String>,
    pub qdrant_configured: bool,
}

fn require_enabled(state: &AppState) -> AppResult<()> {
    if state.config.qdrant_url.is_none() {
        return Err(AppError::BadRequest(
            "the semantic cache is disabled: QDRANT_URL is not set".to_string(),
        ));
    }
    Ok(())
}

pub async fn stats(State(state): State<AppState>) -> AppResult<Json<SemanticStatsDto>> {
    Ok(Json(SemanticStatsDto {
        entries: state.semantic.entry_count().await?,
        hits: state.semantic.hits(),
        threshold: state.semantic.threshold(),
        backend: state.semantic.backend().as_str(),
        qdrant_configured: state.config.qdrant_url.is_some(),
        embedding_model: state.semantic.embedding_model().map(|m| m.to_string()),
    }))
}

pub async fn clear(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    require_enabled(&state)?;
    state.semantic.clear().await?;
    Ok(Json(serde_json::json!({ "cleared": true })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateThreshold {
    pub threshold: f32,
}

pub async fn put_threshold(
    State(state): State<AppState>,
    Json(body): Json<UpdateThreshold>,
) -> AppResult<Json<SemanticStatsDto>> {
    require_enabled(&state)?;
    if !THRESHOLD_RANGE.contains(&body.threshold) {
        return Err(AppError::BadRequest(format!(
            "threshold must be between {} and {}",
            THRESHOLD_RANGE.start(),
            THRESHOLD_RANGE.end()
        )));
    }

    let row = app_setting::ActiveModel {
        key: Set(THRESHOLD_KEY.to_string()),
        value: Set(body.threshold.to_string()),
        updated_at: Set(super::now_rfc3339()),
    };
    app_setting::Entity::insert(row)
        .on_conflict(
            OnConflict::column(app_setting::Column::Key)
                .update_columns([app_setting::Column::Value, app_setting::Column::UpdatedAt])
                .to_owned(),
        )
        .exec(&state.db)
        .await?;

    state.semantic.set_threshold(body.threshold);
    stats(State(state)).await
}

#[derive(Debug, Deserialize)]
pub struct UpdateEmbeddingModel {
    pub embedding_model: Option<String>,
}

pub async fn put_embedding_model(
    State(state): State<AppState>,
    Json(body): Json<UpdateEmbeddingModel>,
) -> AppResult<Json<SemanticStatsDto>> {
    require_enabled(&state)?;
    let model = body
        .embedding_model
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty());

    if let Some(model) = model.as_deref() {
        if state
            .routing
            .resolve_targets_with_pricing(model, &state.pricing)
            .await
            .is_err()
        {
            return Err(AppError::BadRequest(format!(
                "embedding model '{model}' does not resolve to a provider: it needs an \
                 active provider that lists the model and a routing rule (or alias) pointing at it"
            )));
        }
    }

    store_embedding_model(&state, model.as_deref()).await?;
    state.semantic.set_embedding_model(model).await?;
    stats(State(state)).await
}

async fn store_embedding_model(state: &AppState, model: Option<&str>) -> AppResult<()> {
    app_setting::Entity::insert(app_setting::ActiveModel {
        key: Set(semantic::EMBEDDING_MODEL_KEY.to_string()),
        value: Set(model.unwrap_or_default().to_string()),
        updated_at: Set(super::now_rfc3339()),
    })
    .on_conflict(
        OnConflict::column(app_setting::Column::Key)
            .update_columns([app_setting::Column::Value, app_setting::Column::UpdatedAt])
            .to_owned(),
    )
    .exec(&state.db)
    .await?;
    Ok(())
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/api/semantic/stats", axum::routing::get(stats))
        .route("/admin/api/semantic-cache", axum::routing::delete(clear))
        .route("/admin/api/semantic/threshold", put(put_threshold))
        .route(
            "/admin/api/semantic/embedding-model",
            put(put_embedding_model),
        )
}

pub async fn reload_threshold(state: &AppState) {
    if let Some(value) = semantic::read_threshold(&state.db).await {
        state.semantic.set_threshold(value);
    }
}
