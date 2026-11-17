use std::sync::atomic::Ordering;

use axum::{
    extract::{Path, State},
    Json,
};
use sea_orm::{sea_query::OnConflict, ActiveModelTrait, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    entities::{app_setting, model_pricing},
    error::{AppError, AppResult},
    AppState,
};

use super::now_rfc3339;

const REDACT_PROMPTS_KEY: &str = "redact_prompts";

#[derive(Debug, Serialize)]
pub struct PricingDto {
    pub model: String,
    pub provider_type: String,
    pub input_cost_per_1m: f64,
    pub output_cost_per_1m: f64,
    pub updated_at: String,
}

impl From<model_pricing::Model> for PricingDto {
    fn from(row: model_pricing::Model) -> Self {
        Self {
            model: row.model,
            provider_type: row.provider_type,
            input_cost_per_1m: row.input_cost_per_1m,
            output_cost_per_1m: row.output_cost_per_1m,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdatePricing {
    pub provider_type: String,
    pub input_cost_per_1m: f64,
    pub output_cost_per_1m: f64,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettings {
    pub redact_prompts: bool,
}

pub async fn get_pricing(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows = model_pricing::Entity::find().all(&state.db).await?;
    let mut pricing: Vec<PricingDto> = rows.into_iter().map(PricingDto::from).collect();
    pricing.sort_by(|a, b| a.model.cmp(&b.model));

    Ok(Json(json!({ "pricing": pricing })))
}

pub async fn put_pricing(
    State(state): State<AppState>,
    Path(model): Path<String>,
    Json(body): Json<UpdatePricing>,
) -> AppResult<Json<PricingDto>> {
    let model = model.trim().to_string();
    if model.is_empty() {
        return Err(AppError::BadRequest("model must not be empty".into()));
    }

    model_pricing::Entity::insert(model_pricing::ActiveModel {
        model: Set(model.clone()),
        provider_type: Set(body.provider_type.trim().to_lowercase()),
        input_cost_per_1m: Set(body.input_cost_per_1m.max(0.0)),
        output_cost_per_1m: Set(body.output_cost_per_1m.max(0.0)),
        updated_at: Set(now_rfc3339()),
    })
    .on_conflict(
        OnConflict::column(model_pricing::Column::Model)
            .update_columns([
                model_pricing::Column::ProviderType,
                model_pricing::Column::InputCostPer1m,
                model_pricing::Column::OutputCostPer1m,
                model_pricing::Column::UpdatedAt,
            ])
            .to_owned(),
    )
    .exec(&state.db)
    .await?;

    state.pricing.invalidate_all().await;

    let row = model_pricing::Entity::find_by_id(&model)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("pricing row '{model}' vanished")))?;

    Ok(Json(PricingDto::from(row)))
}

pub async fn get_settings(State(state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!({
        "redact_prompts": state.redact_prompts.load(Ordering::Relaxed),
    })))
}

pub async fn put_settings(
    State(state): State<AppState>,
    Json(body): Json<UpdateSettings>,
) -> AppResult<Json<Value>> {
    app_setting::ActiveModel {
        key: Set(REDACT_PROMPTS_KEY.to_string()),
        value: Set(body.redact_prompts.to_string()),
        updated_at: Set(now_rfc3339()),
    }
    .save(&state.db)
    .await?;

    state
        .redact_prompts
        .store(body.redact_prompts, Ordering::Relaxed);

    if body.redact_prompts {
        state.response_cache.clear().await;
        if let Err(error) = state.semantic.clear().await {
            tracing::warn!(%error, "redact enabled but semantic cache clear failed");
        }
    }

    Ok(Json(json!({ "redact_prompts": body.redact_prompts })))
}
