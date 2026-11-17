mod events;
mod translate;

use std::time::Instant;

use axum::{
    extract::{Extension, State},
    http::HeaderMap,
    response::Response,
    Json,
};
use serde_json::Value;

use crate::{
    error::{AppError, AppResult},
    providers::{adapter_for, ChatCompletionRequest, UpstreamCallError},
    routing::{extract_session_id, RoutingTarget, SelectedKey},
    telemetry::RoutingAttempt,
    AppState,
};

use super::{
    chat::{self, completion_response},
    failover::{self, AttemptContext},
    headers::request_id_from_headers,
    streaming::{open_stream, stream_response, StreamSetup},
    AuthenticatedKey,
};

const ENDPOINT: &str = "/v1/responses";

pub(super) async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    authenticated: Option<Extension<AuthenticatedKey>>,
    Json(raw): Json<Value>,
) -> AppResult<Response> {
    let request_id = request_id_from_headers(&headers);
    let key = authenticated.map(|Extension(key)| key);
    let request = translate::from_responses(&raw)?;
    chat::validate_request(&request)?;
    run(&state, request, &headers, key, &request_id).await
}

async fn run(
    state: &AppState,
    request: ChatCompletionRequest,
    headers: &HeaderMap,
    key: Option<AuthenticatedKey>,
    request_id: &str,
) -> AppResult<Response> {
    let limits = super::rate_limits(key.as_ref(), state);
    let mut request = request;
    let input_verdict =
        failover::check_guardrails(state, &mut request, key.as_ref(), ENDPOINT).await?;
    let mut targets = state
        .routing
        .resolve_ordered(&request.model, &state.ewma, &state.pricing)
        .await?;

    let session_id = extract_session_id(headers);
    let affinity_scope = crate::routing::SessionAffinity::tenant_scope(
        ENDPOINT,
        key.as_ref().map(|key| key.id.as_str()),
    );
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
            crate::routing::SessionAffinity::promote_sticky(&mut targets, &binding);
        }
    }

    let ctx = AttemptContext {
        state,
        request_model: &request.model,
        scope: ENDPOINT,
        session_id: session_id.as_deref(),
        key_id: key.as_ref().map(|key| key.id.as_str()),
        circuit: state.routing.circuit(),
        ewma: &state.ewma,
        key_pool: &state.key_pool,
        affinity: &state.affinity,
    };

    let mut streaming = request.stream.unwrap_or(false);
    if streaming && state.guardrails.has_output_rules().await {
        request.stream = None;
        streaming = false;
    }
    let redact = state
        .redact_prompts
        .load(std::sync::atomic::Ordering::Relaxed);
    let mut telemetry = failover::telemetry(
        &request,
        streaming,
        ENDPOINT,
        request_id,
        key.as_ref(),
        redact,
    );
    failover::apply_verdict(&mut telemetry, &input_verdict);
    telemetry.input_tokens = failover::input_tokens(&request);
    failover::check_limits(state, &request.model, key.as_ref(), telemetry.input_tokens).await?;

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

            if streaming {
                match open_stream(state, &target, &request).await {
                    Ok(opened) => {
                        let unit_costs =
                            chat::prefetch_unit_costs(state, &target.upstream_model).await;
                        failover::note_success(&ctx, &target, key_id.as_deref(), opened.ttft_ms);
                        attempts.push(failover::trace(&target, 200, opened.ttft_ms, None));
                        let mut setup_telemetry = telemetry.clone();
                        setup_telemetry.routing_trace = attempts;
                        setup_telemetry.fallback_count = fallback_count;

                        return Ok(stream_response(
                            state.clone(),
                            opened,
                            StreamSetup {
                                telemetry: setup_telemetry,
                                attempts: Vec::new(),
                                fallback_count,
                                estimated_input_tokens: std::sync::atomic::AtomicU32::new(
                                    failover::input_tokens(&request),
                                ),
                                redact,
                                unit_costs,
                                request_id: request_id.to_string(),
                                rate_limits: limits,
                                request: request.clone(),
                                circuit: std::sync::Arc::clone(state.routing.circuit()),
                            },
                            events::format(),
                        ));
                    }
                    Err(err) => {
                        failover::note_failure(&ctx, &target, key_id.as_deref(), &err);
                        if rotation < rotations && err.is_retryable_under(&policy) {
                            rotation += 1;
                            tokio::time::sleep(key_backoff(rotation)).await;
                            next_key = rotated_key(state, &base, key_id.as_deref()).await;
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
                                state,
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

            match adapter_for(target.kind)
                .complete(&state.http_client, &target.endpoint(), &request)
                .await
            {
                Ok(completion) => {
                    let latency_ms = attempted_at.elapsed().as_secs_f64() * 1000.0;
                    failover::note_success(&ctx, &target, key_id.as_deref(), latency_ms);
                    attempts.push(failover::trace(&target, 200, latency_ms, None));
                    let mut completion = completion;
                    failover::redact_completion(state, &mut completion, key.as_ref(), ENDPOINT)
                        .await;
                    return finish(
                        state,
                        completion,
                        target,
                        request_id,
                        limits,
                        telemetry,
                        attempts,
                        started,
                        fallback_count,
                    )
                    .await;
                }
                Err(err) => {
                    failover::note_failure(&ctx, &target, key_id.as_deref(), &err);
                    if rotation < rotations && err.is_retryable_under(&policy) {
                        rotation += 1;
                        tokio::time::sleep(key_backoff(rotation)).await;
                        next_key = rotated_key(state, &base, key_id.as_deref()).await;
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
                            state,
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
            "no usable upstream provider for '{}'",
            request.model
        ))
    })?;
    failover::record_failure(
        state,
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
    completion: crate::providers::ChatCompletionResponse,
    target: RoutingTarget,
    request_id: &str,
    limits: crate::auth::limiter::RateLimitSnapshot,
    mut telemetry: crate::telemetry::TelemetryEvent,
    attempts: Vec<RoutingAttempt>,
    started: Instant,
    fallbacks: u32,
) -> AppResult<Response> {
    let usage = completion.usage.clone().unwrap_or_default();
    let cost_usd = state
        .pricing
        .compute_cost_usd(
            &target.upstream_model,
            usage.prompt_tokens,
            usage.completion_tokens,
        )
        .await;

    telemetry.status_code = 200;
    telemetry.input_tokens = usage.prompt_tokens;
    telemetry.output_tokens = usage.completion_tokens;
    telemetry.cost_usd = cost_usd;
    telemetry.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    telemetry.routing_trace = attempts;
    telemetry.routed_provider_id = Some(target.provider_id.clone());
    telemetry.routed_provider_name = Some(target.provider_name.clone());
    telemetry.routed_model = Some(target.upstream_model.clone());
    telemetry.response_text = super::chat::first_message_text(&completion);
    failover::book_spend(state, telemetry.virtual_key_id.as_deref(), cost_usd);
    state.telemetry.record(telemetry);

    let body = events::completed_body(&completion, &target.upstream_model);
    Ok(completion_response(
        body,
        request_id,
        &target,
        fallbacks,
        Some(cost_usd),
        limits,
    ))
}
