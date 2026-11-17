mod chat;
mod decisions;
mod embeddings;
mod failover;
mod headers;
mod messages;
mod responses;
mod streaming;

use axum::{
    extract::{Extension, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use crate::{
    auth::limiter::RateLimitSnapshot, error::AppResult, telemetry::TelemetryEvent, AppState,
};

use self::headers::request_id_from_headers;

#[derive(Debug, Clone)]
pub struct AuthenticatedKey {
    pub id: String,
    pub name: String,
    pub rpm_limit: Option<i64>,
    pub tpm_limit: Option<i64>,
    pub budget_usd: Option<f64>,
    pub allowed_models: Vec<String>,
    pub is_active: bool,
}

pub fn router() -> axum::Router<AppState> {
    use axum::routing::{get, post};

    axum::Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(responses::handle))
        .route("/v1/messages", post(messages::handle))
        .route("/v1/embeddings", post(embeddings::handle))
        .route("/v1/decisions", post(decisions::handle_openai))
        .route("/decisions", post(decisions::handle_openai))
        .route("/openai/v1/decisions", post(decisions::handle_openai))
        .route("/v1/systemone", post(decisions::handle_typesafe))
        .route("/systemone", post(decisions::handle_typesafe))
        .route("/typesafe/v1/systemone", post(decisions::handle_typesafe))
        .route("/v1beta/decisions", post(decisions::handle_gemini))
        .route("/genai/v1beta/decisions", post(decisions::handle_gemini))
        .route("/v1/models", get(list_models))
        .route("/typesafe/v1/models", get(decisions::list_typesafe_models))
}

async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    authenticated: Option<Extension<AuthenticatedKey>>,
    Json(raw): Json<Value>,
) -> AppResult<Response> {
    let request_id = request_id_from_headers(&headers);
    let key = authenticated.map(|Extension(key)| key);
    let request = chat::parse_request(raw)?;
    chat::validate_request(&request)?;
    chat::run(&state, request, &headers, key, &request_id).await
}

pub(crate) fn rate_limits(key: Option<&AuthenticatedKey>, state: &AppState) -> RateLimitSnapshot {
    match key {
        Some(key) => state
            .auth
            .limiters
            .snapshot(&key.id, key.rpm_limit, key.tpm_limit),
        None => RateLimitSnapshot::default(),
    }
}

pub(crate) fn base_event(
    model: &str,
    streaming: bool,
    endpoint: &str,
    request_id: &str,
    key: Option<&AuthenticatedKey>,
) -> TelemetryEvent {
    let mut event = TelemetryEvent::new(model, streaming);
    event.request_id = Some(request_id.to_string());
    event.endpoint = endpoint.to_string();
    if let Some(key) = key {
        event.virtual_key_id = Some(key.id.clone());
        event.virtual_key_name = Some(key.name.clone());
    }
    event
}

async fn list_models(
    State(state): State<AppState>,
    authenticated: Option<Extension<AuthenticatedKey>>,
) -> AppResult<Response> {
    let models = state.routing.list_available_models().await?;

    let allowed = authenticated
        .map(|Extension(key)| key.allowed_models)
        .unwrap_or_default();

    let data: Vec<Value> = models
        .into_iter()
        .filter(|id| allowed.is_empty() || crate::auth::model_is_allowed(&allowed, id))
        .map(|id| {
            json!({
                "id": id,
                "object": "model",
                "created": 1_700_000_000,
                "owned_by": "ferrogate",
            })
        })
        .collect();

    Ok(Json(json!({ "object": "list", "data": data })).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_event_keeps_a_server_generated_id() {
        let event = base_event(
            "gpt-4o",
            false,
            "/v1/chat/completions",
            "caller-supplied-id",
            None,
        );
        assert_ne!(event.id, "caller-supplied-id");
        assert_eq!(event.request_id.as_deref(), Some("caller-supplied-id"));
    }

    #[test]
    fn base_event_attributes_the_calling_key() {
        let key = AuthenticatedKey {
            id: "key-1".to_string(),
            name: "Primary".to_string(),
            rpm_limit: None,
            tpm_limit: None,
            budget_usd: None,
            allowed_models: Vec::new(),
            is_active: true,
        };
        let event = base_event(
            "gpt-4o",
            false,
            "/v1/chat/completions",
            "caller-supplied-id",
            Some(&key),
        );
        assert_ne!(event.id, "caller-supplied-id");
        assert_eq!(event.virtual_key_id.as_deref(), Some("key-1"));
        assert_eq!(event.virtual_key_name.as_deref(), Some("Primary"));
    }
}
