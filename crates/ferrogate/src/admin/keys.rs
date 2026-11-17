use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use rand::Rng;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, ModelTrait, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::sha256_hex,
    entities::virtual_key,
    error::{AppError, AppResult},
    AppState,
};

use super::{encode_string_list, now_rfc3339, parse_string_list};

#[derive(Debug, Serialize)]
pub struct VirtualKeyDto {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub budget_usd: Option<f64>,
    pub spent_usd: f64,
    pub rpm_limit: Option<i64>,
    pub tpm_limit: Option<i64>,
    pub allowed_models: Vec<String>,
    pub allowed_mcp: Vec<String>,
    pub tags: Vec<String>,
    pub team_name: Option<String>,
    pub expires_at: Option<String>,
    pub is_active: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

impl VirtualKeyDto {
    fn build(state: &AppState, row: virtual_key::Model) -> Self {
        Self {
            spent_usd: state.auth.effective_spend_usd(&row.id, row.spent_usd),
            allowed_models: parse_string_list(&row.allowed_models_json),
            allowed_mcp: parse_string_list(&row.allowed_mcp_json),
            tags: parse_string_list(&row.tags_json),
            team_name: row.team_name,
            expires_at: row.expires_at,
            is_active: row.is_active != 0,
            id: row.id,
            name: row.name,
            key_prefix: row.key_prefix,
            budget_usd: row.budget_usd,
            rpm_limit: row.rpm_limit,
            tpm_limit: row.tpm_limit,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct KeyExtras {
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub team_name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub allowed_mcp: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateKey {
    pub name: String,
    #[serde(default)]
    pub budget_usd: Option<f64>,
    #[serde(default)]
    pub rpm_limit: Option<i64>,
    #[serde(default)]
    pub tpm_limit: Option<i64>,
    #[serde(default)]
    pub allowed_models: Vec<String>,
    #[serde(flatten)]
    pub extras: KeyExtras,
}

#[derive(Debug, Deserialize)]
pub struct UpdateKey {
    pub name: Option<String>,
    pub budget_usd: Option<f64>,
    #[serde(default)]
    pub clear_budget: Option<bool>,
    pub rpm_limit: Option<i64>,
    #[serde(default)]
    pub clear_rpm_limit: Option<bool>,
    pub tpm_limit: Option<i64>,
    #[serde(default)]
    pub clear_tpm_limit: Option<bool>,
    pub allowed_models: Option<Vec<String>>,
    pub is_active: Option<bool>,
    #[serde(default)]
    pub reset_spend: Option<bool>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub clear_expires_at: Option<bool>,
    #[serde(default)]
    pub team_name: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub allowed_mcp: Option<Vec<String>>,
}

const KEY_PREFIX: &str = "sk-fg-";
const DISPLAY_PREFIX_LEN: usize = 12;
const PLAYGROUND_BUDGET_USD: f64 = 5.0;
const PLAYGROUND_RPM_LIMIT: i64 = 60;
const PLAYGROUND_TPM_LIMIT: i64 = 20_000;

fn playground_expiry() -> String {
    (Utc::now() + chrono::Duration::hours(12)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn generate_raw_key() -> String {
    let bytes: [u8; 16] = rand::rng().random();
    format!("{KEY_PREFIX}{}", hex::encode(bytes))
}

fn not_found(id: &str) -> AppError {
    AppError::NotFound(format!("virtual key '{id}' not found"))
}

fn normalise_rfc3339(raw: Option<&str>) -> AppResult<Option<String>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    DateTime::parse_from_rfc3339(raw)
        .map(|parsed| Some(parsed.with_timezone(&Utc).to_rfc3339()))
        .map_err(|_| AppError::BadRequest(format!("'{raw}' is not an RFC 3339 timestamp")))
}

fn team_of(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|team| !team.is_empty())
        .map(str::to_string)
}

async fn find_key(state: &AppState, id: &str) -> AppResult<virtual_key::Model> {
    virtual_key::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(|| not_found(id))
}

pub async fn list(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows = virtual_key::Entity::find()
        .order_by_desc(virtual_key::Column::CreatedAt)
        .all(&state.db)
        .await?;

    let keys: Vec<VirtualKeyDto> = rows
        .into_iter()
        .map(|row| VirtualKeyDto::build(&state, row))
        .collect();

    Ok(Json(json!({ "keys": keys })))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateKey>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("key name must not be empty".into()));
    }

    let raw_key = generate_raw_key();
    let key_hash = sha256_hex(&raw_key);
    let key_prefix = format!("{}...", &raw_key[..DISPLAY_PREFIX_LEN.min(raw_key.len())]);

    let is_playground = body
        .extras
        .tags
        .iter()
        .any(|tag| tag.trim() == "playground");
    let budget_usd = match (is_playground, body.budget_usd) {
        (true, Some(budget)) => Some(budget.clamp(0.0, PLAYGROUND_BUDGET_USD)),
        (true, None) => Some(PLAYGROUND_BUDGET_USD),
        (false, budget) => budget,
    };
    let rpm_limit = match (is_playground, body.rpm_limit) {
        (true, None) => Some(PLAYGROUND_RPM_LIMIT),
        _ => body.rpm_limit,
    };
    let tpm_limit = match (is_playground, body.tpm_limit) {
        (true, None) => Some(PLAYGROUND_TPM_LIMIT),
        _ => body.tpm_limit,
    };
    let expires_at = match normalise_rfc3339(body.extras.expires_at.as_deref())? {
        Some(value) => Some(value),
        None if is_playground => Some(playground_expiry()),
        None => None,
    };

    let row = virtual_key::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        name: Set(name.to_string()),
        key_prefix: Set(key_prefix),
        key_hash: Set(key_hash),
        budget_usd: Set(budget_usd),
        spent_usd: Set(0.0),
        rpm_limit: Set(rpm_limit),
        tpm_limit: Set(tpm_limit),
        allowed_models_json: Set(encode_string_list(&body.allowed_models)),
        allowed_mcp_json: Set(encode_string_list(&body.extras.allowed_mcp)),
        tags_json: Set(encode_string_list(&body.extras.tags)),
        team_name: Set(team_of(body.extras.team_name.as_deref())),
        expires_at: Set(expires_at),
        is_active: Set(1),
        created_at: Set(now_rfc3339()),
        last_used_at: Set(None),
    }
    .insert(&state.db)
    .await?;

    state.auth.invalidate_key_hash(&row.key_hash).await;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "key": VirtualKeyDto::build(&state, row),
            "plaintext_key": raw_key,
        })),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateKey>,
) -> AppResult<Json<VirtualKeyDto>> {
    let existing = find_key(&state, &id).await?;
    let old_hash = existing.key_hash.clone();
    let mut active: virtual_key::ActiveModel = existing.into();

    if let Some(name) = body.name.as_deref() {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(AppError::BadRequest("key name must not be empty".into()));
        }
        active.name = Set(trimmed.to_string());
    }

    if body.clear_budget == Some(true) {
        active.budget_usd = Set(None);
    } else if let Some(budget) = body.budget_usd {
        active.budget_usd = Set(Some(budget));
    }

    if body.clear_rpm_limit == Some(true) {
        active.rpm_limit = Set(None);
    } else if let Some(limit) = body.rpm_limit {
        active.rpm_limit = Set(Some(limit));
    }

    if body.clear_tpm_limit == Some(true) {
        active.tpm_limit = Set(None);
    } else if let Some(limit) = body.tpm_limit {
        active.tpm_limit = Set(Some(limit));
    }

    if let Some(models) = body.allowed_models.as_ref() {
        active.allowed_models_json = Set(encode_string_list(models));
    }
    if let Some(mcp) = body.allowed_mcp.as_ref() {
        active.allowed_mcp_json = Set(encode_string_list(mcp));
    }
    if let Some(tags) = body.tags.as_ref() {
        active.tags_json = Set(encode_string_list(tags));
    }
    if let Some(team) = body.team_name.as_deref() {
        active.team_name = Set(team_of(Some(team)));
    }
    if body.clear_expires_at == Some(true) {
        active.expires_at = Set(None);
    } else if let Some(expires_at) = normalise_rfc3339(body.expires_at.as_deref())? {
        active.expires_at = Set(Some(expires_at));
    }
    if let Some(is_active) = body.is_active {
        active.is_active = Set(i64::from(is_active));
    }

    let reset_spend = body.reset_spend == Some(true);
    if reset_spend {
        active.spent_usd = Set(0.0);
    }

    let row = active.update(&state.db).await?;

    state.auth.invalidate_key_hash(&old_hash).await;
    state.auth.reset_key_state(&id, reset_spend);

    Ok(Json(VirtualKeyDto::build(&state, row)))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let existing = find_key(&state, &id).await?;
    let key_hash = existing.key_hash.clone();
    existing.delete(&state.db).await?;

    state.auth.invalidate_key_hash(&key_hash).await;
    state.auth.reset_key_state(&id, true);

    Ok(Json(json!({ "deleted": true, "id": id })))
}

#[derive(Debug, Serialize)]
pub struct TeamDto {
    pub name: String,
    pub keys: usize,
    pub spend_usd: f64,
}

pub async fn teams(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows = virtual_key::Entity::find()
        .filter(virtual_key::Column::TeamName.is_not_null())
        .all(&state.db)
        .await?;

    let mut by_team: std::collections::BTreeMap<String, TeamDto> =
        std::collections::BTreeMap::new();

    for row in rows {
        let Some(team) = row.team_name.as_deref().filter(|team| !team.is_empty()) else {
            continue;
        };

        let entry = by_team.entry(team.to_string()).or_insert_with(|| TeamDto {
            name: team.to_string(),
            keys: 0,
            spend_usd: 0.0,
        });
        entry.keys += 1;
        entry.spend_usd += state.auth.effective_spend_usd(&row.id, row.spent_usd);
    }

    let teams: Vec<TeamDto> = by_team.into_values().collect();
    Ok(Json(json!({ "teams": teams })))
}
