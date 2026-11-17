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
    entities::mcp_server,
    error::{AppError, AppResult},
    mcp::registry::{
        self, decode_cached_tools, McpRegistry, ServerEntry, ToolInfo, STATUS_NEVER_SYNCED,
    },
    AppState,
};

use super::now_rfc3339;

#[derive(Debug, Serialize)]
pub struct ServerDto {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub url: String,
    pub headers: Value,
    pub allowed_tools: Vec<String>,
    pub tools: Vec<ToolDto>,
    pub tool_count: usize,
    pub last_status: Option<String>,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ToolDto {
    pub name: String,
    pub description: String,
}

impl From<ToolInfo> for ToolDto {
    fn from(tool: ToolInfo) -> Self {
        Self {
            name: tool.name,
            description: tool.description,
        }
    }
}

impl ServerDto {
    pub fn from_row(row: mcp_server::Model) -> Self {
        let tools = decode_cached_tools(row.tools_cache_json.as_deref());
        let headers: Value = serde_json::from_str(&row.headers_json).unwrap_or_else(|_| json!({}));

        Self {
            id: row.id,
            name: row.name,
            transport: row.transport,
            url: row.url,
            headers,
            allowed_tools: registry::parse_string_list(&row.allowed_tools_json),
            tool_count: tools.len(),
            tools: tools.into_iter().map(ToolDto::from).collect(),
            last_status: row
                .last_status
                .or_else(|| Some(STATUS_NEVER_SYNCED.to_string())),
            is_active: row.is_active != 0,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateServer {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub headers: Option<Value>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateServer {
    pub name: Option<String>,
    pub url: Option<String>,
    pub transport: Option<String>,
    pub headers: Option<Value>,
    pub allowed_tools: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

fn default_true() -> bool {
    true
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/api/mcp/servers", get(list).post(create))
        .route("/admin/api/mcp/servers/{id}", patch(update).delete(remove))
        .route("/admin/api/mcp/servers/{id}/refresh", post(refresh))
        .route("/admin/api/mcp/tools", get(tools))
}

pub async fn list(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows = mcp_server::Entity::find()
        .order_by_asc(mcp_server::Column::Name)
        .all(&state.db)
        .await?;

    let servers: Vec<ServerDto> = rows.into_iter().map(ServerDto::from_row).collect();
    Ok(Json(json!({ "servers": servers })))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateServer>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let name = require_name(&body.name)?;
    let url = require_url(&body.url, state.config.allow_private_upstreams).await?;
    let transport = registry::parse_transport(body.transport.as_deref())?;
    let headers = encode_headers(body.headers)?;
    let allowed_tools = body.allowed_tools.unwrap_or_default();

    let now = now_rfc3339();
    let row = mcp_server::ActiveModel {
        id: Set(format!("mcp-{}", uuid::Uuid::new_v4())),
        name: Set(name),
        transport: Set(transport),
        url: Set(url),
        headers_json: Set(headers),
        allowed_tools_json: Set(serde_json::to_string(&allowed_tools)?),
        tools_cache_json: Set(None),
        last_status: Set(None),
        is_active: Set(i64::from(body.is_active)),
        created_at: Set(now.clone()),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    let tools = discover(&state.mcp, &row).await;
    let row = find_server(&state, &row.id).await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "server": ServerDto::from_row(row), "discovered": tools })),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateServer>,
) -> AppResult<Json<Value>> {
    let existing = find_server(&state, &id).await?;

    let mut active: mcp_server::ActiveModel = existing.clone().into();
    if let Some(name) = body.name.as_deref() {
        active.name = Set(require_name(name)?);
    }
    if let Some(url) = body.url.as_deref() {
        active.url = Set(require_url(url, state.config.allow_private_upstreams).await?);
    }
    if body.transport.is_some() {
        active.transport = Set(registry::parse_transport(body.transport.as_deref())?);
    }
    if body.headers.is_some() {
        active.headers_json = Set(encode_headers(body.headers)?);
    }
    if let Some(allowed_tools) = body.allowed_tools {
        active.allowed_tools_json = Set(serde_json::to_string(&allowed_tools)?);
    }
    if let Some(is_active) = body.is_active {
        active.is_active = Set(i64::from(is_active));
    }
    active.updated_at = Set(now_rfc3339());

    active.update(&state.db).await?;
    state.mcp.invalidate(&id).await;

    let row = find_server(&state, &id).await?;
    Ok(Json(json!({ "server": ServerDto::from_row(row) })))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let row = find_server(&state, &id).await?;
    row.delete(&state.db).await?;
    state.mcp.invalidate(&id).await;
    Ok(Json(json!({ "deleted": true })))
}

pub async fn refresh(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let row = find_server(&state, &id).await?;
    let discovered = state.mcp.refresh(&row).await?;

    let row = find_server(&state, &id).await?;
    Ok(Json(json!({
        "server": ServerDto::from_row(row),
        "discovered": discovered.len(),
    })))
}

pub async fn tools(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let servers = state.mcp.active_servers().await?;

    let mut merged = Vec::new();
    for server in servers {
        for tool in server.tools.iter() {
            merged.push(json!({
                "server": server.name,
                "name": ServerEntry::qualified(&server.name, &tool.name),
                "description": tool.description,
            }));
        }
    }

    Ok(Json(json!({ "tools": merged })))
}

async fn discover(registry: &McpRegistry, row: &mcp_server::Model) -> usize {
    match registry.refresh(row).await {
        Ok(tools) => tools.len(),
        Err(err) => {
            tracing::warn!(server = %row.name, error = %err, "mcp tool discovery failed");
            0
        }
    }
}

async fn find_server(state: &AppState, id: &str) -> AppResult<mcp_server::Model> {
    mcp_server::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("mcp server '{id}' not found")))
}

fn require_name(name: &str) -> AppResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("server name must not be empty".into()));
    }
    if trimmed.contains("__") {
        return Err(AppError::BadRequest(
            "server name must not contain '__'".into(),
        ));
    }

    match trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        true => Ok(trimmed.to_string()),
        false => Err(AppError::BadRequest(
            "server name may only contain letters, digits and '-'".into(),
        )),
    }
}

async fn require_url(url: &str, allow_private: bool) -> AppResult<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("server url must not be empty".into()));
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(AppError::BadRequest(
            "server url must start with http:// or https://".into(),
        ));
    }

    crate::egress::validate_url(trimmed, allow_private).await
}

fn encode_headers(headers: Option<Value>) -> AppResult<String> {
    let Some(headers) = headers else {
        return Ok("{}".to_string());
    };

    let object = headers
        .as_object()
        .ok_or_else(|| AppError::BadRequest("headers must be a JSON object".into()))?;

    let mut encoded = serde_json::Map::with_capacity(object.len());
    for (name, value) in object {
        let value = value
            .as_str()
            .ok_or_else(|| AppError::BadRequest(format!("header '{name}' must be a string")))?;
        encoded.insert(name.clone(), Value::String(value.to_string()));
    }

    Ok(Value::Object(encoded).to_string())
}
