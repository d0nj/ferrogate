use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch},
    Json, Router,
};
use sea_orm::{ActiveModelTrait, EntityTrait, ModelTrait, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    entities::guardrail_rule,
    error::{AppError, AppResult},
    AppState,
};

use super::now_rfc3339;

#[derive(Debug, Serialize)]
pub struct GuardrailDto {
    pub id: String,
    pub name: String,
    pub phase: String,
    pub rule_type: String,
    pub pattern: Option<String>,
    pub action: String,
    pub priority: i64,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<guardrail_rule::Model> for GuardrailDto {
    fn from(row: guardrail_rule::Model) -> Self {
        Self {
            id: row.id,
            name: row.name,
            phase: row.phase,
            rule_type: row.rule_type,
            pattern: row.pattern,
            action: row.action,
            priority: row.priority,
            is_active: row.is_active != 0,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateGuardrail {
    pub name: String,
    pub phase: String,
    pub rule_type: String,
    #[serde(default)]
    pub pattern: Option<String>,
    pub action: String,
    #[serde(default)]
    pub priority: i64,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateGuardrail {
    pub name: Option<String>,
    pub phase: Option<String>,
    pub rule_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_nullable")]
    pub pattern: Option<Option<String>>,
    pub action: Option<String>,
    pub priority: Option<i64>,
    pub is_active: Option<bool>,
}

fn default_true() -> bool {
    true
}

fn deserialize_optional_nullable<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/api/guardrails", get(list).post(create))
        .route("/admin/api/guardrails/schema", get(schema))
        .route("/admin/api/guardrails/{id}", patch(update).delete(remove))
}

pub async fn schema() -> Json<Value> {
    Json(json!({
        "phases": PHASES,
        "rule_types": RULE_TYPES,
        "actions": ACTIONS,
    }))
}

pub async fn list(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows = guardrail_rule::Entity::find()
        .order_by_asc(guardrail_rule::Column::Priority)
        .order_by_asc(guardrail_rule::Column::Name)
        .all(&state.db)
        .await?;

    let guardrails: Vec<GuardrailDto> = rows.into_iter().map(GuardrailDto::from).collect();
    Ok(Json(json!({ "guardrails": guardrails })))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateGuardrail>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let name = require_name(&body.name)?;
    let phase = require_phase(&body.phase)?;
    let rule_type = require_rule_type(&body.rule_type)?;
    let action = require_action(&body.action)?;
    let pattern = normalise_pattern(&rule_type, body.pattern)?;

    let now = now_rfc3339();
    let row = guardrail_rule::ActiveModel {
        id: Set(format!("guardrail-{}", uuid::Uuid::new_v4())),
        name: Set(name),
        phase: Set(phase),
        rule_type: Set(rule_type),
        pattern: Set(pattern),
        action: Set(action),
        priority: Set(body.priority),
        is_active: Set(i64::from(body.is_active)),
        created_at: Set(now.clone()),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    state.guardrails.invalidate().await;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "guardrail": GuardrailDto::from(row) })),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateGuardrail>,
) -> AppResult<Json<Value>> {
    let existing = find_rule(&state, &id).await?;

    let mut active: guardrail_rule::ActiveModel = existing.clone().into();
    if let Some(name) = body.name.as_deref() {
        active.name = Set(require_name(name)?);
    }
    if let Some(phase) = body.phase.as_deref() {
        active.phase = Set(require_phase(phase)?);
    }
    if let Some(rule_type) = body.rule_type.as_deref() {
        active.rule_type = Set(require_rule_type(rule_type)?);
    }
    if let Some(action) = body.action.as_deref() {
        active.action = Set(require_action(action)?);
    }
    if let Some(priority) = body.priority {
        active.priority = Set(priority);
    }
    if let Some(is_active) = body.is_active {
        active.is_active = Set(i64::from(is_active));
    }

    let rule_type = match body.rule_type.as_deref() {
        Some(value) => require_rule_type(value)?,
        None => existing.rule_type,
    };
    let pattern = match body.pattern {
        Some(Some(pattern)) => Some(pattern),
        Some(None) => None,
        None if rule_type == "regex" => existing.pattern,
        None => None,
    };
    active.pattern = Set(normalise_pattern(&rule_type, pattern)?);
    active.updated_at = Set(now_rfc3339());

    let row = active.update(&state.db).await?;
    state.guardrails.invalidate().await;
    Ok(Json(json!({ "guardrail": GuardrailDto::from(row) })))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let row = find_rule(&state, &id).await?;
    row.delete(&state.db).await?;
    state.guardrails.invalidate().await;
    Ok(Json(json!({ "deleted": true })))
}

async fn find_rule(state: &AppState, id: &str) -> AppResult<guardrail_rule::Model> {
    guardrail_rule::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("guardrail '{id}' not found")))
}

fn require_name(name: &str) -> AppResult<String> {
    let trimmed = name.trim();
    match trimmed.is_empty() {
        true => Err(AppError::BadRequest(
            "guardrail name must not be empty".into(),
        )),
        false => Ok(trimmed.to_string()),
    }
}

pub const PHASES: &[&str] = &["input", "output"];
pub const RULE_TYPES: &[&str] = &["pii", "secret", "prompt_injection", "regex"];
pub const ACTIONS: &[&str] = &["block", "redact", "flag"];

fn one_of(value: &str, allowed: &[&str], field: &str) -> AppResult<String> {
    if allowed.contains(&value) {
        return Ok(value.to_string());
    }
    Err(AppError::BadRequest(format!(
        "{field} must be one of {}, got '{value}'",
        allowed.join(", ")
    )))
}

fn require_phase(phase: &str) -> AppResult<String> {
    one_of(phase, PHASES, "phase")
}

fn require_rule_type(rule_type: &str) -> AppResult<String> {
    one_of(rule_type, RULE_TYPES, "rule_type")
}

fn require_action(action: &str) -> AppResult<String> {
    one_of(action, ACTIONS, "action")
}

fn normalise_pattern(rule_type: &str, pattern: Option<String>) -> AppResult<Option<String>> {
    if rule_type != "regex" {
        return Ok(None);
    }

    let pattern = pattern.unwrap_or_default();
    let trimmed = pattern.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "regex rules require a non-empty pattern".into(),
        ));
    }
    regex::Regex::new(trimmed)
        .map_err(|err| AppError::BadRequest(format!("invalid regex pattern: {err}")))?;

    Ok(Some(trimmed.to_string()))
}
