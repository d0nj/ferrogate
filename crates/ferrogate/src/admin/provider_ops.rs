use std::{collections::BTreeSet, time::Duration, time::Instant};

use axum::{
    extract::{Path, State},
    Json,
};
use reqwest::RequestBuilder;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde_json::{json, Value};

use crate::{
    entities::provider,
    error::{AppError, AppResult},
    providers::{join_url, ProviderKind},
    routing::parse_models_json,
    AppState,
};

use super::{encode_string_list, now_rfc3339};

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_ERROR_BODY: usize = 500;

fn api_base(base_url: &str, version: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    trimmed
        .strip_suffix(&format!("/{version}"))
        .unwrap_or(trimmed)
        .to_string()
}

fn models_url(kind: ProviderKind, base_url: &str) -> String {
    match kind {
        ProviderKind::Gemini => join_url(&api_base(base_url, "v1beta"), "/v1beta/models"),
        _ => join_url(&api_base(base_url, "v1"), "/v1/models"),
    }
}

fn with_auth(request: RequestBuilder, kind: ProviderKind, api_key: &str) -> RequestBuilder {
    match kind {
        ProviderKind::Anthropic => request
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION),
        ProviderKind::Gemini => request.header("x-goog-api-key", api_key),
        _ => request.bearer_auth(api_key),
    }
}

async fn find_provider(state: &AppState, id: &str) -> AppResult<provider::Model> {
    provider::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("provider '{id}' not found")))
}

fn kind_of(row: &provider::Model) -> AppResult<ProviderKind> {
    ProviderKind::parse(&row.provider_type).ok_or_else(|| {
        AppError::BadRequest(format!(
            "unsupported provider_type '{}'; connectivity probes need openai, anthropic, gemini or typesafe",
            row.provider_type
        ))
    })
}

struct Probe {
    status: Option<u16>,
    latency_ms: f64,
    body: Option<Value>,
    error: Option<String>,
}

fn snippet(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    match trimmed.char_indices().nth(limit) {
        Some((cut, _)) => format!("{}…", &trimmed[..cut]),
        None => trimmed.to_string(),
    }
}

async fn probe_credential(state: &AppState, row: &provider::Model) -> String {
    let row_key = state.secrets.open(&row.api_key).unwrap_or_default();
    if !row_key.trim().is_empty() {
        return row_key.trim().to_string();
    }

    state
        .key_pool
        .entries(&row.id)
        .await
        .iter()
        .find(|entry| !entry.api_key.trim().is_empty() && !state.key_pool.in_cooldown(&entry.id))
        .map(|entry| entry.api_key.trim().to_string())
        .unwrap_or_default()
}

async fn probe(state: &AppState, row: &provider::Model, kind: ProviderKind) -> Probe {
    let url = models_url(kind, &row.base_url);
    let credential = probe_credential(state, row).await;

    if credential.is_empty() {
        return Probe {
            status: None,
            latency_ms: 0.0,
            body: None,
            error: Some(
                "no API key on this provider and no usable credential in its pool".to_string(),
            ),
        };
    }

    let request = with_auth(state.http_client.get(&url), kind, &credential);
    let started = Instant::now();

    match request.timeout(PROBE_TIMEOUT).send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
            let text = response.text().await.unwrap_or_default();
            let parsed = serde_json::from_str::<Value>(&text).ok();
            let error = (status >= 400).then(|| {
                format!(
                    "upstream returned HTTP {status}: {}",
                    snippet(&text, MAX_ERROR_BODY)
                )
            });
            Probe {
                status: Some(status),
                latency_ms,
                body: parsed,
                error,
            }
        }
        Err(error) => Probe {
            status: error.status().map(|status| status.as_u16()),
            latency_ms: started.elapsed().as_secs_f64() * 1000.0,
            body: None,
            error: Some(if error.is_timeout() {
                format!("upstream timed out after {}s", PROBE_TIMEOUT.as_secs())
            } else {
                error.to_string()
            }),
        },
    }
}

fn extract_models(kind: ProviderKind, body: &Value) -> Vec<String> {
    let named_under = |key: &str| -> Vec<String> {
        body.get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|model| model.get("name").and_then(Value::as_str))
            .map(str::to_string)
            .collect()
    };

    let mut found: BTreeSet<String> = BTreeSet::new();
    match kind {
        ProviderKind::Gemini => {
            found.extend(
                named_under("models")
                    .into_iter()
                    .map(|name| name.trim_start_matches("models/").to_string()),
            );
        }
        ProviderKind::Typesafe => found.extend(named_under("models")),
        _ => {
            if let Some(models) = body.get("data").and_then(Value::as_array) {
                for model in models {
                    if let Some(id) = model.get("id").and_then(Value::as_str) {
                        found.insert(id.to_string());
                    }
                }
            }
        }
    }

    found
        .into_iter()
        .filter(|model| !model.is_empty())
        .collect()
}

pub async fn test(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let row = find_provider(&state, &id).await?;
    let kind = kind_of(&row)?;
    let result = probe(&state, &row, kind).await;
    let models = result
        .body
        .as_ref()
        .map(|body| extract_models(kind, body))
        .unwrap_or_default();
    let ok = result.status.is_some_and(|status| status < 400);

    Ok(Json(json!({
        "ok": ok,
        "status_code": result.status,
        "latency_ms": result.latency_ms,
        "models_found": models.len(),
        "error": result.error,
    })))
}

pub async fn sync_models(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let row = find_provider(&state, &id).await?;
    let kind = kind_of(&row)?;
    let result = probe(&state, &row, kind).await;

    if let Some(error) = result.error.as_deref() {
        return Err(AppError::Upstream(format!(
            "cannot read models from provider '{id}': {error}"
        )));
    }

    let discovered = result
        .body
        .as_ref()
        .map(|body| extract_models(kind, body))
        .unwrap_or_default();
    if discovered.is_empty() {
        return Err(AppError::Upstream(format!(
            "provider '{id}' returned no models"
        )));
    }

    let existing: BTreeSet<String> = parse_models_json(&row.models_json).into_iter().collect();
    let fresh: BTreeSet<String> = discovered.into_iter().collect();

    let added_count = fresh.difference(&existing).count();
    let removed: Vec<String> = existing.difference(&fresh).cloned().collect();
    let models: Vec<String> = fresh.into_iter().collect();

    let now = now_rfc3339();
    provider::ActiveModel {
        id: Set(row.id),
        models_json: Set(encode_string_list(&models)),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(&state.db)
    .await?;

    state.routing.invalidate().await;

    Ok(Json(json!({
        "ok": true,
        "added_count": added_count,
        "removed_count": removed.len(),
        "removed": removed,
        "models": models,
    })))
}

#[cfg(test)]
mod extract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_compatible_reads_data_ids() {
        let body = json!({"data": [{"id": "gpt-4o"}, {"id": "gpt-4o-mini"}]});
        assert_eq!(
            extract_models(ProviderKind::OpenAi, &body),
            vec!["gpt-4o", "gpt-4o-mini"]
        );
    }

    #[test]
    fn typesafe_reads_names_under_models() {
        let body = json!({
            "models": [
                {"name": "jev-latest", "description": "latest", "release_date": "2026-10-22T02:00:00+00:00"},
                {"name": "jev-preview", "description": "preview", "release_date": "2026-10-14T20:14:00+00:00"}
            ]
        });
        assert_eq!(
            extract_models(ProviderKind::Typesafe, &body),
            vec!["jev-latest", "jev-preview"]
        );
    }

    #[test]
    fn gemini_strips_the_models_prefix() {
        let body = json!({"models": [{"name": "models/gemini-2.5-pro"}, {"name": "models/gemini-2.5-flash"}]});
        assert_eq!(
            extract_models(ProviderKind::Gemini, &body),
            vec!["gemini-2.5-flash", "gemini-2.5-pro"]
        );
    }

    #[test]
    fn an_unexpected_shape_yields_nothing_rather_than_garbage() {
        assert!(extract_models(ProviderKind::Typesafe, &json!({"detail": "nope"})).is_empty());
        assert!(
            extract_models(ProviderKind::OpenAi, &json!({"models": [{"name": "x"}]})).is_empty()
        );
        assert!(
            extract_models(ProviderKind::Typesafe, &json!({"models": [{"name": ""}]})).is_empty()
        );
    }
}
