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

use std::collections::BTreeMap;

use crate::{
    entities::{provider, provider_key},
    error::{AppError, AppResult},
    providers::{ApiDialect, ProviderKind},
    routing::{parse_extra_headers, parse_models_json},
    AppState,
};

use super::{encode_string_list, mask_secret, now_rfc3339, seal};

#[derive(Debug, Serialize)]
pub struct ProviderDto {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key_masked: String,
    pub has_api_key: bool,
    pub pool_key_count: i64,
    pub models: Vec<String>,
    pub is_active: bool,
    pub timeout_ms: i64,
    pub region: Option<String>,
    pub zdr: bool,
    pub extra_headers: BTreeMap<String, String>,
    pub api_dialect: String,
    pub created_at: String,
    pub updated_at: String,
}

impl ProviderDto {
    async fn from_model(db: &sea_orm::DatabaseConnection, row: provider::Model) -> Self {
        let pool_keys = provider_key::Entity::find()
            .filter(provider_key::Column::ProviderId.eq(row.id.clone()))
            .filter(provider_key::Column::IsActive.eq(1))
            .all(db)
            .await
            .unwrap_or_default();
        let pool_key = pool_keys.iter().find(|key| !key.api_key.trim().is_empty());
        let inline = !row.api_key.trim().is_empty();
        let masked = if inline {
            mask_secret(&row.api_key)
        } else {
            pool_key.map_or_else(String::new, |key| mask_secret(&key.api_key))
        };
        Self {
            api_dialect: row.api_dialect,
            api_key_masked: masked,
            has_api_key: inline || pool_key.is_some(),
            pool_key_count: pool_keys.len() as i64,
            models: parse_models_json(&row.models_json),
            is_active: row.is_active != 0,
            region: row.region,
            zdr: row.zdr != 0,
            extra_headers: parse_extra_headers(&row.extra_headers_json),
            id: row.id,
            name: row.name,
            provider_type: row.provider_type,
            base_url: row.base_url,
            timeout_ms: row.timeout_ms,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateProvider {
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default = "default_true")]
    pub is_active: bool,
    #[serde(default = "default_timeout")]
    pub timeout_ms: i64,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub zdr: Option<bool>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub api_dialect: ApiDialect,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProvider {
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: Option<bool>,
    #[serde(default)]
    pub api_dialect: Option<ApiDialect>,
    pub models: Option<Vec<String>>,
    pub is_active: Option<bool>,
    pub timeout_ms: Option<i64>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub zdr: Option<bool>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
}

fn default_true() -> bool {
    true
}

fn default_timeout() -> i64 {
    60_000
}

fn clean_region(region: Option<String>) -> Option<String> {
    region
        .map(|region| region.trim().to_string())
        .filter(|region| !region.is_empty())
}

fn encode_headers(headers: Option<BTreeMap<String, String>>) -> String {
    match headers {
        Some(headers) => {
            serde_json::to_string(&headers).unwrap_or_else(|_| DEFAULT_HEADERS_JSON.to_string())
        }
        None => DEFAULT_HEADERS_JSON.to_string(),
    }
}

const DEFAULT_HEADERS_JSON: &str = "{}";

fn normalise_kind(raw: &str) -> AppResult<String> {
    ProviderKind::parse(raw)
        .map(|kind| kind.as_str().to_string())
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "unsupported provider_type '{raw}'; expected one of openai, anthropic, gemini, typesafe"
            ))
        })
}

fn require_name(name: &str) -> AppResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "provider name must not be empty".into(),
        ));
    }
    Ok(trimmed.to_string())
}

async fn require_base_url(url: &str, allow_private: bool) -> AppResult<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("base_url must not be empty".into()));
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(AppError::BadRequest(
            "base_url must start with http:// or https://".into(),
        ));
    }
    crate::egress::validate_url(trimmed, allow_private).await
}

fn not_found(id: &str) -> AppError {
    AppError::NotFound(format!("provider '{id}' not found"))
}

async fn find_provider(state: &AppState, id: &str) -> AppResult<provider::Model> {
    provider::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(|| not_found(id))
}

pub async fn list(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows = provider::Entity::find()
        .order_by_asc(provider::Column::Name)
        .all(&state.db)
        .await?;

    let mut providers: Vec<ProviderDto> = Vec::with_capacity(rows.len());
    for row in rows {
        providers.push(ProviderDto::from_model(&state.db, row).await);
    }
    Ok(Json(json!({ "providers": providers })))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateProvider>,
) -> AppResult<(StatusCode, Json<ProviderDto>)> {
    let name = require_name(&body.name)?;
    let provider_type = normalise_kind(&body.provider_type)?;
    let base_url = require_base_url(&body.base_url, state.config.allow_private_upstreams).await?;

    let duplicate = provider::Entity::find()
        .filter(provider::Column::Name.eq(name.clone()))
        .one(&state.db)
        .await?;
    if duplicate.is_some() {
        return Err(AppError::BadRequest(format!(
            "provider name '{name}' already exists"
        )));
    }

    let now = now_rfc3339();
    let row = provider::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        name: Set(name),
        provider_type: Set(provider_type),
        base_url: Set(base_url),
        api_key: Set(seal(&state, body.api_key.as_deref().unwrap_or_default())?),
        models_json: Set(encode_string_list(&body.models)),
        api_dialect: Set(body.api_dialect.as_str().to_string()),
        is_active: Set(i64::from(body.is_active)),
        timeout_ms: Set(body.timeout_ms.max(0)),
        region: Set(clean_region(body.region)),
        zdr: Set(i64::from(body.zdr.unwrap_or(false))),
        extra_headers_json: Set(encode_headers(body.extra_headers)),
        created_at: Set(now.clone()),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    state.routing.invalidate().await;
    let dto = ProviderDto::from_model(&state.db, row).await;
    Ok((StatusCode::CREATED, Json(dto)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateProvider>,
) -> AppResult<Json<ProviderDto>> {
    let existing = find_provider(&state, &id).await?;

    let mut active: provider::ActiveModel = existing.clone().into();
    if let Some(name) = body.name.as_deref() {
        active.name = Set(require_name(name)?);
    }
    if let Some(dialect) = body.api_dialect {
        active.api_dialect = Set(dialect.as_str().to_string());
    }
    if let Some(provider_type) = body.provider_type.as_deref() {
        active.provider_type = Set(normalise_kind(provider_type)?);
    }
    if let Some(base_url) = body.base_url.as_deref() {
        active.base_url =
            Set(require_base_url(base_url, state.config.allow_private_upstreams).await?);
    }
    if let Some(models) = body.models.as_ref() {
        active.models_json = Set(encode_string_list(models));
    }
    if let Some(is_active) = body.is_active {
        active.is_active = Set(i64::from(is_active));
    }
    if let Some(timeout_ms) = body.timeout_ms {
        active.timeout_ms = Set(timeout_ms.max(0));
    }
    if let Some(region) = body.region {
        active.region = Set(clean_region(Some(region)));
    }
    if let Some(zdr) = body.zdr {
        active.zdr = Set(i64::from(zdr));
    }
    if let Some(headers) = body.extra_headers {
        active.extra_headers_json = Set(encode_headers(Some(headers)));
    }

    let replacement = body
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string);
    if body.clear_api_key == Some(true) {
        active.api_key = Set(String::new());
    } else if let Some(key) = replacement {
        active.api_key = Set(seal(&state, &key)?);
    }

    active.updated_at = Set(now_rfc3339());
    let row = active.update(&state.db).await?;

    state.routing.invalidate().await;
    Ok(Json(ProviderDto::from_model(&state.db, row).await))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let existing = find_provider(&state, &id).await?;
    existing.delete(&state.db).await?;

    state.routing.invalidate().await;
    Ok(Json(json!({ "deleted": true, "id": id })))
}
