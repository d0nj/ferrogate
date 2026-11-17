use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, ModelTrait, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    entities::{provider, provider_key},
    error::{AppError, AppResult},
    AppState,
};

use super::{encode_string_list, mask_secret, now_rfc3339, parse_string_list, seal};

#[derive(Debug, Serialize)]
pub struct ProviderKeyDto {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub api_key_masked: String,
    pub has_api_key: bool,
    pub weight: i64,
    pub models: Vec<String>,
    pub rpm_limit: Option<i64>,
    pub is_active: bool,
    pub in_cooldown: bool,
    pub cooldown_until: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl ProviderKeyDto {
    fn build(state: &AppState, row: provider_key::Model) -> Self {
        let (in_cooldown, cooldown_until, last_error) = state.key_pool.cooldown_snapshot(&row.id);

        Self {
            api_key_masked: mask_secret(&row.api_key),
            has_api_key: !row.api_key.trim().is_empty(),
            models: parse_string_list(&row.models_json),
            is_active: row.is_active != 0,
            in_cooldown,
            cooldown_until: cooldown_until.or(row.cooldown_until),
            last_error: last_error.or(row.last_error),
            id: row.id,
            provider_id: row.provider_id,
            name: row.name,
            weight: row.weight,
            rpm_limit: row.rpm_limit,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateProviderKey {
    pub name: String,
    pub api_key: String,
    #[serde(default)]
    pub weight: Option<i64>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub rpm_limit: Option<i64>,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProviderKey {
    pub name: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    pub weight: Option<i64>,
    pub models: Option<Vec<String>>,
    pub rpm_limit: Option<i64>,
    #[serde(default)]
    pub clear_rpm_limit: Option<bool>,
    pub is_active: Option<bool>,
    #[serde(default)]
    pub clear_cooldown: Option<bool>,
}

fn default_true() -> bool {
    true
}

fn not_found(id: &str) -> AppError {
    AppError::NotFound(format!("provider key '{id}' not found"))
}

fn require_name(name: &str) -> AppResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "provider key name must not be empty".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn require_secret(key: &str) -> AppResult<String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("api_key must not be empty".into()));
    }
    Ok(trimmed.to_string())
}

async fn find_key(state: &AppState, id: &str) -> AppResult<provider_key::Model> {
    provider_key::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(|| not_found(id))
}

async fn find_scoped_key(
    state: &AppState,
    provider_id: &str,
    key_id: &str,
) -> AppResult<provider_key::Model> {
    let key = find_key(state, key_id).await?;
    if key.provider_id != provider_id {
        return Err(not_found(key_id));
    }
    Ok(key)
}

async fn require_provider(state: &AppState, id: &str) -> AppResult<provider::Model> {
    provider::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("provider '{id}' not found")))
}

async fn refresh(state: &AppState, provider_id: &str) {
    state.routing.invalidate().await;
    state.key_pool.invalidate_provider(provider_id).await;
}

pub async fn list(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    require_provider(&state, &id).await?;

    let rows = provider_key::Entity::find()
        .filter(provider_key::Column::ProviderId.eq(id))
        .order_by_asc(provider_key::Column::Name)
        .all(&state.db)
        .await?;

    let keys: Vec<ProviderKeyDto> = rows
        .into_iter()
        .map(|row| ProviderKeyDto::build(&state, row))
        .collect();
    Ok(Json(json!({ "keys": keys })))
}

pub async fn create(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateProviderKey>,
) -> AppResult<(StatusCode, Json<Value>)> {
    require_provider(&state, &id).await?;
    let name = require_name(&body.name)?;
    let api_key = require_secret(&body.api_key)?;

    let now = now_rfc3339();
    let row = provider_key::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        provider_id: Set(id.clone()),
        name: Set(name),
        api_key: Set(seal(&state, &api_key)?),
        weight: Set(body.weight.unwrap_or(1).max(1)),
        models_json: Set(encode_string_list(&body.models)),
        rpm_limit: Set(body.rpm_limit),
        cooldown_until: Set(None),
        last_error: Set(None),
        is_active: Set(i64::from(body.is_active)),
        created_at: Set(now.clone()),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    refresh(&state, &id).await;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "key": ProviderKeyDto::build(&state, row) })),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    Path((provider_id, key_id)): Path<(String, String)>,
    Json(body): Json<UpdateProviderKey>,
) -> AppResult<Json<Value>> {
    let existing = find_scoped_key(&state, &provider_id, &key_id).await?;
    let mut active: provider_key::ActiveModel = existing.into();

    if let Some(name) = body.name.as_deref() {
        active.name = Set(require_name(name)?);
    }
    if let Some(api_key) = body.api_key.as_deref() {
        if !api_key.trim().is_empty() {
            active.api_key = Set(seal(&state, &require_secret(api_key)?)?);
        }
    }
    if let Some(weight) = body.weight {
        active.weight = Set(weight.max(1));
    }
    if let Some(models) = body.models.as_ref() {
        active.models_json = Set(encode_string_list(models));
    }
    if body.clear_rpm_limit == Some(true) {
        active.rpm_limit = Set(None);
    } else if let Some(limit) = body.rpm_limit {
        active.rpm_limit = Set(Some(limit));
    }
    if let Some(is_active) = body.is_active {
        active.is_active = Set(i64::from(is_active));
    }
    if body.clear_cooldown == Some(true) {
        active.cooldown_until = Set(None);
        active.last_error = Set(None);
    }

    active.updated_at = Set(now_rfc3339());
    let row = active.update(&state.db).await?;

    if body.clear_cooldown == Some(true) {
        state.key_pool.record_success(&key_id);
    }
    refresh(&state, &provider_id).await;
    Ok(Json(json!({ "key": ProviderKeyDto::build(&state, row) })))
}

pub async fn remove(
    State(state): State<AppState>,
    Path((provider_id, key_id)): Path<(String, String)>,
) -> AppResult<Json<Value>> {
    let existing = find_scoped_key(&state, &provider_id, &key_id).await?;
    existing.delete(&state.db).await?;

    state.key_pool.record_success(&key_id);
    refresh(&state, &provider_id).await;
    Ok(Json(json!({ "deleted": true, "id": key_id })))
}
