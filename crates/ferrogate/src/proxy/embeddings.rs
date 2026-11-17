use std::{sync::atomic::Ordering, time::Instant};

use axum::{
    extract::{Extension, State},
    http::HeaderMap,
    response::Response,
    Json,
};
use serde_json::Value;

use crate::{
    error::{AppError, AppResult},
    providers::{
        adapter_for, ChatCompletionRequest, EmbeddingRequest, EmbeddingResponse, UpstreamCallError,
    },
    routing::{extract_session_id, RoutingTarget, SelectedKey, SessionAffinity},
    telemetry::RoutingAttempt,
    tokenizer::count_text_tokens,
    AppState,
};

use super::{
    chat::completion_response,
    failover::{self, AttemptContext},
    headers::request_id_from_headers,
    AuthenticatedKey,
};

const ENDPOINT: &str = "/v1/embeddings";

pub(super) async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    authenticated: Option<Extension<AuthenticatedKey>>,
    Json(raw): Json<Value>,
) -> AppResult<Response> {
    let request_id = request_id_from_headers(&headers);
    let key = authenticated.map(|Extension(key)| key);
    let request: EmbeddingRequest = serde_json::from_value(raw)
        .map_err(|err| AppError::BadRequest(format!("invalid request body: {err}")))?;
    validate(&request)?;

    let limits = super::rate_limits(key.as_ref(), &state);
    let input_tokens = estimate_tokens(&request);
    failover::check_limits(&state, &request.model, key.as_ref(), input_tokens).await?;
    let input_verdict = failover::check_guardrails(
        &state,
        &mut scratch_request(&request),
        key.as_ref(),
        ENDPOINT,
    )
    .await?;

    let mut targets = state
        .routing
        .resolve_ordered(&request.model, &state.ewma, &state.pricing)
        .await?;

    let session_id = extract_session_id(&headers);
    let affinity_scope =
        SessionAffinity::tenant_scope(ENDPOINT, key.as_ref().map(|key| key.id.as_str()));
    if let Some(session) = session_id.as_deref() {
        if let Some(binding) = state
            .affinity
            .get_with_redis(
                state.redis.as_deref(),
                &affinity_scope,
                session,
                &request.model,
            )
            .await
        {
            SessionAffinity::promote_sticky(&mut targets, &binding);
        }
    }

    let ctx = AttemptContext {
        state: &state,
        request_model: &request.model,
        scope: ENDPOINT,
        session_id: session_id.as_deref(),
        key_id: key.as_ref().map(|key| key.id.as_str()),
        circuit: state.routing.circuit(),
        ewma: &state.ewma,
        key_pool: &state.key_pool,
        affinity: &state.affinity,
    };

    let mut telemetry = failover::telemetry_for(
        &request.model,
        false,
        ENDPOINT,
        &request_id,
        key.as_ref(),
        state.redact_prompts.load(Ordering::Relaxed),
        Some(request.input.to_string()),
    );
    telemetry.input_tokens = input_tokens;
    failover::apply_verdict(&mut telemetry, &input_verdict);

    let started = Instant::now();
    let mut attempts: Vec<RoutingAttempt> = Vec::new();
    let policy = state.routing.policy();
    let mut fallback_count = 0;
    let mut last_error: Option<(UpstreamCallError, RoutingTarget)> = None;

    for base in targets {
        if !policy.allows_attempt(fallback_count) {
            break;
        }
        if state.routing.circuit().is_open(&base.provider_id) {
            attempts.push(failover::trace(&base, 0, 0.0, Some("circuit open")));
            continue;
        }

        let rotations = if state.key_pool.entries(&base.provider_id).await.len() > 1 {
            policy.key_rotations as usize
        } else {
            0
        };
        let mut rotation = 0;
        let mut next_key: Option<SelectedKey> = None;

        loop {
            let mut target = failover::bind_target(&ctx, &base).await;
            if let Some(rotated) = next_key.take() {
                target.api_key = rotated.api_key;
                target.selected_key_id = rotated.id;
                target.selected_key_name = rotated.name;
            }
            let key_id = target.selected_key_id.clone();
            let attempted_at = Instant::now();

            match adapter_for(target.kind)
                .embed(&state.http_client, &target.endpoint(), &request)
                .await
            {
                Ok(response) => {
                    let latency_ms = attempted_at.elapsed().as_secs_f64() * 1000.0;
                    failover::note_success(&ctx, &target, key_id.as_deref(), latency_ms);
                    attempts.push(failover::trace(&target, 200, latency_ms, None));
                    return finish(
                        &state,
                        response,
                        request_id,
                        target,
                        attempts,
                        fallback_count,
                        started,
                        telemetry,
                        limits,
                    )
                    .await;
                }
                Err(err) => {
                    failover::note_failure(&ctx, &target, key_id.as_deref(), &err);
                    if rotation < rotations && err.is_retryable_under(&policy) {
                        rotation += 1;
                        tokio::time::sleep(key_backoff(rotation)).await;
                        next_key = rotated_key(&state, &base, key_id.as_deref()).await;
                        continue;
                    }
                    attempts.push(failover::trace(
                        &target,
                        err.status_code(),
                        attempted_at.elapsed().as_secs_f64() * 1000.0,
                        Some(&err.to_string()),
                    ));
                    if !err.is_retryable_under(&policy) {
                        failover::record_failure(
                            &state,
                            telemetry,
                            attempts,
                            &err,
                            Some(&target),
                            fallback_count,
                            started,
                        );
                        return Err(AppError::Upstream(format!(
                            "upstream provider '{}' failed: {}",
                            target.provider_name, err
                        )));
                    }
                    fallback_count += 1;
                    last_error = Some((err, target));
                    break;
                }
            }
        }
    }

    let (err, target) = last_error.ok_or_else(|| {
        AppError::NotFound(format!(
            "no routing target found for model '{}'",
            request.model
        ))
    })?;
    failover::record_failure(
        &state,
        telemetry,
        attempts,
        &err,
        Some(&target),
        fallback_count,
        started,
    );
    Err(failover::exhausted_error(&err))
}

async fn rotated_key(
    state: &AppState,
    base: &RoutingTarget,
    failed: Option<&str>,
) -> Option<SelectedKey> {
    state
        .key_pool
        .entries(&base.provider_id)
        .await
        .iter()
        .find(|entry| {
            Some(entry.id.as_str()) != failed
                && !entry.api_key.trim().is_empty()
                && entry.serves(&base.upstream_model)
                && !state.key_pool.in_cooldown(&entry.id)
        })
        .map(|entry| SelectedKey {
            id: Some(entry.id.clone()),
            name: Some(entry.name.clone()),
            api_key: entry.api_key.clone(),
        })
}

fn key_backoff(rotation: usize) -> std::time::Duration {
    match rotation {
        1 => std::time::Duration::from_millis(100),
        _ => std::time::Duration::from_millis(250),
    }
}

#[allow(clippy::too_many_arguments)]
async fn finish(
    state: &AppState,
    mut response: EmbeddingResponse,
    request_id: String,
    target: RoutingTarget,
    attempts: Vec<RoutingAttempt>,
    fallback_count: u32,
    started: Instant,
    mut telemetry: crate::telemetry::TelemetryEvent,
    limits: crate::auth::limiter::RateLimitSnapshot,
) -> AppResult<Response> {
    response.model = target.upstream_model.clone();
    let input_tokens = response.usage.prompt_tokens;
    let cost_usd = state
        .pricing
        .compute_cost_usd(&target.upstream_model, input_tokens, 0)
        .await;

    telemetry.status_code = 200;
    telemetry.input_tokens = input_tokens;
    telemetry.output_tokens = 0;
    telemetry.cost_usd = cost_usd;
    telemetry.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    telemetry.fallback_count = fallback_count;
    telemetry.routing_trace = attempts;
    telemetry.routed_provider_id = Some(target.provider_id.clone());
    telemetry.routed_provider_name = Some(target.provider_name.clone());
    telemetry.routed_model = Some(target.upstream_model.clone());
    telemetry.response_text = None;
    failover::book_spend(state, telemetry.virtual_key_id.as_deref(), cost_usd);
    state.telemetry.record(telemetry);

    Ok(completion_response(
        serde_json::to_value(&response)?,
        &request_id,
        &target,
        fallback_count,
        Some(cost_usd),
        limits,
    ))
}

fn validate(request: &EmbeddingRequest) -> AppResult<()> {
    if request.model.trim().is_empty() {
        return Err(AppError::BadRequest(
            "field 'model' must be a non-empty string".to_string(),
        ));
    }
    let empty = match &request.input {
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => true,
    };
    if empty {
        return Err(AppError::BadRequest(
            "field 'input' must be a non-empty string or array".to_string(),
        ));
    }
    Ok(())
}

fn scratch_request(request: &EmbeddingRequest) -> ChatCompletionRequest {
    let content = match &request.input {
        Value::Array(items) => Value::String(
            items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| item.to_string())
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        other => other.clone(),
    };

    ChatCompletionRequest {
        model: request.model.clone(),
        messages: vec![crate::providers::ChatMessage {
            role: "user".to_string(),
            content,
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }],
        temperature: None,
        top_p: None,
        max_tokens: None,
        max_completion_tokens: None,
        stream: None,
        stream_options: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        user: None,
        tools: None,
        tool_choice: None,
        response_format: None,
        seed: None,
        reasoning_effort: None,
        template: None,
        template_version: None,
        template_variables: None,
    }
}

fn estimate_tokens(request: &EmbeddingRequest) -> u32 {
    let text = match &request.input {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        other => other.to_string(),
    };
    count_text_tokens(&request.model, &text).max(1)
}
