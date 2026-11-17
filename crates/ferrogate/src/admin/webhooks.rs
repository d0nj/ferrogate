use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch, post},
    Json, Router,
};
use sea_orm::{ActiveModelTrait, EntityTrait, ModelTrait, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    entities::webhook,
    error::{AppError, AppResult},
    guardrails::webhooks::{self, KNOWN_EVENTS},
    AppState,
};

use super::{mask_secret, now_rfc3339, seal};

#[derive(Debug, Serialize)]
pub struct WebhookDto {
    pub id: String,
    pub name: String,
    pub url: String,
    pub has_secret: bool,
    pub secret_masked: String,
    pub events: Vec<String>,
    pub is_active: bool,
    pub last_status: Option<i64>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<webhook::Model> for WebhookDto {
    fn from(row: webhook::Model) -> Self {
        let secret = row.secret.unwrap_or_default();
        Self {
            id: row.id,
            name: row.name,
            url: row.url,
            has_secret: !secret.trim().is_empty(),
            secret_masked: mask_secret(&secret),
            events: webhooks::parse_events(&row.events_json),
            is_active: row.is_active != 0,
            last_status: row.last_status,
            last_error: row.last_error,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateWebhook {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWebhook {
    pub name: Option<String>,
    pub url: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub clear_secret: Option<bool>,
    pub events: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

fn default_true() -> bool {
    true
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/api/webhooks", get(list).post(create))
        .route("/admin/api/webhooks/{id}", patch(update).delete(remove))
        .route("/admin/api/webhooks/{id}/test", post(test))
}

pub async fn list(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows = webhook::Entity::find()
        .order_by_asc(webhook::Column::Name)
        .all(&state.db)
        .await?;

    let webhooks: Vec<WebhookDto> = rows.into_iter().map(WebhookDto::from).collect();
    Ok(Json(json!({ "webhooks": webhooks })))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateWebhook>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let name = require_name(&body.name)?;
    let url = require_url(&body.url, state.config.allow_private_upstreams).await?;
    let events = normalise_events(&body.events)?;

    let now = now_rfc3339();
    let secret = match body.secret.filter(|value| !value.trim().is_empty()) {
        Some(value) => Some(seal(&state, &value)?),
        None => None,
    };
    let row = webhook::ActiveModel {
        id: Set(format!("webhook-{}", uuid::Uuid::new_v4())),
        name: Set(name),
        url: Set(url),
        secret: Set(secret),
        events_json: Set(encode_events(&events)),
        is_active: Set(i64::from(body.is_active)),
        last_status: Set(None),
        last_error: Set(None),
        created_at: Set(now.clone()),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "webhook": WebhookDto::from(row) })),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateWebhook>,
) -> AppResult<Json<Value>> {
    let existing = find_webhook(&state, &id).await?;

    let mut active: webhook::ActiveModel = existing.into();
    if let Some(name) = body.name.as_deref() {
        active.name = Set(require_name(name)?);
    }
    if let Some(url) = body.url.as_deref() {
        active.url = Set(require_url(url, state.config.allow_private_upstreams).await?);
    }
    if let Some(events) = body.events.as_deref() {
        active.events_json = Set(encode_events(&normalise_events(events)?));
    }
    if let Some(is_active) = body.is_active {
        active.is_active = Set(i64::from(is_active));
    }
    if body.clear_secret.unwrap_or(false) {
        active.secret = Set(None);
    } else if let Some(secret) = body.secret.filter(|value| !value.trim().is_empty()) {
        active.secret = Set(Some(seal(&state, &secret)?));
    }
    active.updated_at = Set(now_rfc3339());

    let row = active.update(&state.db).await?;
    Ok(Json(json!({ "webhook": WebhookDto::from(row) })))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let row = find_webhook(&state, &id).await?;
    row.delete(&state.db).await?;
    Ok(Json(json!({ "deleted": true })))
}

pub async fn test(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let subscriber = find_webhook(&state, &id).await?;
    let event = webhooks::WebhookEvent::new(webhooks::EVENT_PING, json!({ "webhook": id }));

    let outcome = webhooks::deliver(
        &state.db,
        &state.http_client,
        &state.secrets,
        &subscriber,
        &event,
    )
    .await;

    Ok(Json(json!({
        "delivered": outcome.delivered(),
        "status": outcome.status,
        "error": outcome.error,
    })))
}

async fn find_webhook(state: &AppState, id: &str) -> AppResult<webhook::Model> {
    webhook::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("webhook '{id}' not found")))
}

fn require_name(name: &str) -> AppResult<String> {
    let trimmed = name.trim();
    match trimmed.is_empty() {
        true => Err(AppError::BadRequest(
            "webhook name must not be empty".into(),
        )),
        false => Ok(trimmed.to_string()),
    }
}

async fn require_url(url: &str, allow_private: bool) -> AppResult<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("webhook url must not be empty".into()));
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(AppError::BadRequest(
            "webhook url must start with http:// or https://".into(),
        ));
    }
    crate::egress::validate_url(trimmed, allow_private).await
}

fn normalise_events(events: &[String]) -> AppResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for event in events {
        let name = event.trim();
        if name.is_empty() {
            continue;
        }
        if !KNOWN_EVENTS.contains(&name) {
            return Err(AppError::BadRequest(format!(
                "unknown webhook event '{name}'"
            )));
        }
        if !out.iter().any(|known| known == name) {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

fn encode_events(events: &[String]) -> String {
    serde_json::to_string(events).unwrap_or_else(|_| "[]".to_string())
}
