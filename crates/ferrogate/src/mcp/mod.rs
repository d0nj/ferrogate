pub mod jsonrpc;
pub mod registry;

use axum::{
    body::Bytes,
    extract::{Extension, State},
    http::{header::CONTENT_TYPE, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{json, Value};

use crate::{entities::virtual_key, proxy::AuthenticatedKey, AppState};

use self::{
    jsonrpc::{
        Envelope, Reply, Request, RpcError, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST,
        METHOD_NOT_FOUND,
    },
    registry::{acl_allows, ServerEntry},
};

pub const PROTOCOL_VERSION: &str = "2024-11-05";

pub const MAX_BATCH_ELEMENTS: usize = 16;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new().route("/mcp", axum::routing::post(handle))
}

pub async fn handle(
    State(state): State<AppState>,
    authenticated: Option<Extension<AuthenticatedKey>>,
    body: Bytes,
) -> Response {
    let key = authenticated.map(|Extension(key)| key);
    let acl = key_allowed_mcp_or_deny(&state, key.as_ref()).await;
    let is_batch = is_batch_body(&body);

    let envelope = match jsonrpc::parse_body(&body) {
        Ok(envelope) => envelope,
        Err(reply) => return respond(&state, &acl, reply.into_value()),
    };

    if let Envelope::Batch(requests) = &envelope {
        if requests.len() > MAX_BATCH_ELEMENTS {
            let reply = Reply::error(
                Value::Null,
                RpcError::new(
                    INVALID_REQUEST,
                    format!("batch too large: max {MAX_BATCH_ELEMENTS} requests"),
                ),
            );
            return respond(&state, &acl, reply.into_value());
        }
    }

    let mut replies = Vec::new();
    match envelope {
        Envelope::Single(request) => {
            if let Some(reply) = dispatch(&state, &key, &acl, request).await {
                replies.push(reply);
            }
        }
        Envelope::Batch(requests) => {
            for request in requests {
                if let Some(reply) = dispatch(&state, &key, &acl, request).await {
                    replies.push(reply);
                }
            }
        }
    }

    if replies.is_empty() {
        return notification_response();
    }

    let payload = match is_batch {
        true => Value::Array(replies.into_iter().map(Reply::into_value).collect()),
        false => replies
            .into_iter()
            .next()
            .map(Reply::into_value)
            .unwrap_or(Value::Null),
    };

    respond(&state, &acl, payload)
}

fn is_batch_body(body: &[u8]) -> bool {
    body.iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| *byte == b'[')
}

async fn dispatch(
    state: &AppState,
    key: &Option<AuthenticatedKey>,
    acl: &Option<Vec<String>>,
    request: Request,
) -> Option<Reply> {
    let id = request.id.clone();
    let outcome = match request.method.as_str() {
        "initialize" => Ok(initialize_result()),
        "ping" => Ok(json!({})),
        "notifications/initialized" => return None,
        "tools/list" => gated_tools_list(state, key, acl).await,
        "tools/call" => gated_tools_call(state, key, acl, request.params).await,
        other => Err(RpcError::new(
            METHOD_NOT_FOUND,
            format!("unknown method '{other}'"),
        )),
    };

    id.map(|id| Reply { id, outcome })
}

async fn admit_mcp_call(state: &AppState, key: &Option<AuthenticatedKey>) -> Result<(), RpcError> {
    let Some(key) = key else {
        return Err(RpcError::with_data(
            INTERNAL_ERROR,
            "missing virtual key".to_string(),
            json!({ "reason": "tool_not_allowed" }),
        ));
    };
    state.auth.admit_rpm(key).await.map_err(|err| {
        RpcError::with_data(
            INTERNAL_ERROR,
            format!("rate limited: {err}"),
            json!({ "reason": "rate_limited" }),
        )
    })
}

async fn gated_tools_list(
    state: &AppState,
    key: &Option<AuthenticatedKey>,
    acl: &Option<Vec<String>>,
) -> Result<Value, RpcError> {
    admit_mcp_call(state, key).await?;
    tools_list(state, acl).await
}

async fn gated_tools_call(
    state: &AppState,
    key: &Option<AuthenticatedKey>,
    acl: &Option<Vec<String>>,
    params: Value,
) -> Result<Value, RpcError> {
    admit_mcp_call(state, key).await?;
    tools_call(state, acl, params).await
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "ferrogate", "version": env!("CARGO_PKG_VERSION") },
    })
}

async fn tools_list(state: &AppState, acl: &Option<Vec<String>>) -> Result<Value, RpcError> {
    let Some(acl) = acl else {
        return Ok(json!({ "tools": [] }));
    };
    let servers = state
        .mcp
        .active_servers()
        .await
        .map_err(|err| RpcError::new(INTERNAL_ERROR, err.to_string()))?;

    let mut tools = Vec::new();
    for server in servers
        .iter()
        .filter(|server| acl_allows(acl, &server.name, &server.name))
    {
        for tool in server.tools.iter() {
            tools.push(json!({
                "name": ServerEntry::qualified(&server.name, &tool.name),
                "description": tool.description,
                "inputSchema": { "type": "object" },
            }));
        }
    }

    Ok(json!({ "tools": tools }))
}

async fn tools_call(
    state: &AppState,
    acl: &Option<Vec<String>>,
    params: Value,
) -> Result<Value, RpcError> {
    let Some(acl) = acl else {
        return Err(RpcError::with_data(
            INTERNAL_ERROR,
            "tool is not permitted for this key".to_string(),
            json!({ "reason": "tool_not_allowed" }),
        ));
    };
    let Some(name) = jsonrpc::string_param(&params, "name") else {
        return Err(RpcError::new(
            INVALID_PARAMS,
            "params.name must be a string",
        ));
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let (server, tool) = state
        .mcp
        .resolve(&name)
        .await
        .map_err(|err| RpcError::new(INTERNAL_ERROR, err.to_string()))?;

    if !acl_allows(acl, &server.name, &name) {
        return Err(RpcError::with_data(
            INTERNAL_ERROR,
            format!("tool '{name}' is not permitted for this key"),
            json!({ "reason": "tool_not_allowed" }),
        ));
    }

    state
        .mcp
        .call_tool(&server, &tool, arguments)
        .await
        .map_err(|err| RpcError::new(INTERNAL_ERROR, err.to_string()))
}

async fn key_allowed_mcp_or_deny(
    state: &AppState,
    key: Option<&AuthenticatedKey>,
) -> Option<Vec<String>> {
    let key = key?;
    let row = virtual_key::Entity::find()
        .filter(virtual_key::Column::Id.eq(key.id.as_str()))
        .one(&state.db)
        .await
        .ok()??;
    serde_json::from_str::<Vec<String>>(&row.allowed_mcp_json).ok()
}

fn respond(_state: &AppState, acl: &Option<Vec<String>>, payload: Value) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let allow = acl.as_deref().unwrap_or_default().join(",");
    if let Ok(value) = HeaderValue::from_str(&allow) {
        headers.insert("x-mcp-allow", value);
    } else {
        headers.insert("x-mcp-allow", HeaderValue::from_static(""));
    }

    (headers, Json(payload)).into_response()
}

fn notification_response() -> Response {
    StatusCode::NO_CONTENT.into_response()
}
