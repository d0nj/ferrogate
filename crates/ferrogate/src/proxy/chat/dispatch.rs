use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use axum::{http::HeaderMap, response::Response};

use crate::{
    error::{AppError, AppResult},
    providers::{adapter_for, ChatCompletionRequest, ChatCompletionResponse, UpstreamCallError},
    routing::{cid, ModelSelector, RoutePreference, RoutingTarget, SessionAffinity},
    telemetry::RoutingAttempt,
    AppState,
};

use super::super::{
    failover::{self, AttemptContext},
    streaming::{open_stream, stream_response, OpenedStream, StreamSetup},
    AuthenticatedKey,
};
use super::{finish, Finish};

pub(crate) async fn run(
    state: &AppState,
    request: ChatCompletionRequest,
    headers: &HeaderMap,
    key: Option<AuthenticatedKey>,
    request_id: &str,
) -> AppResult<Response> {
    let limits = super::super::rate_limits(key.as_ref(), state);

    let mut request = request;

    if let Some(slug) = request.template.clone() {
        let template = crate::prompts::select(state, &slug, request.template_version).await?;
        let values = request.template_variables.clone().unwrap_or_default();
        let rendered = template.render(&values);

        request.messages = template.messages(&rendered);
        if request.model.trim().is_empty() {
            if let Some(model) = template.row.model.clone() {
                request.model = model;
            }
        }
        if request.temperature.is_none() {
            request.temperature = template.row.temperature;
        }
        if request.max_tokens.is_none() && request.max_completion_tokens.is_none() {
            request.max_tokens = template
                .row
                .max_tokens
                .and_then(|value| u32::try_from(value).ok());
        }
    }

    let input_verdict =
        failover::check_guardrails(state, &mut request, key.as_ref(), "/v1/chat/completions")
            .await?;

    let input_tokens = failover::input_tokens(&request);
    if let Some(key) = key.as_ref() {
        failover::check_limits(state, &request.model, Some(key), input_tokens).await?;
    }

    if let Some(response) = finish::cache_hit(
        state,
        &request,
        headers,
        request_id,
        limits,
        key.as_ref(),
        &input_verdict,
    )
    .await
    {
        return Ok(response);
    }
    if let Some(response) = finish::semantic_hit(
        state,
        &request,
        headers,
        request_id,
        limits,
        key.as_ref(),
        &input_verdict,
    )
    .await
    {
        return Ok(response);
    }

    let mut targets = state
        .routing
        .resolve_ordered(&request.model, &state.ewma, &state.pricing)
        .await?;

    let session_id = cid::extract_session_id(headers);
    let affinity_scope =
        SessionAffinity::tenant_scope(&request.model, key.as_ref().map(|key| key.id.as_str()));
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
    let latency_mode = ModelSelector::parse(&request.model).mode == RoutePreference::Nitro;
    if latency_mode {
        state.ewma.sort_by_latency(&mut targets);
    }

    let ctx = AttemptContext {
        state,
        request_model: &request.model,
        scope: &request.model,
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
    let redact = state.redact_prompts.load(Ordering::Relaxed);
    let mut telemetry = failover::telemetry(
        &request,
        streaming,
        "/v1/chat/completions",
        request_id,
        key.as_ref(),
        redact,
    );
    failover::apply_verdict(&mut telemetry, &input_verdict);
    telemetry.input_tokens = input_tokens;

    let cache_eligible = !streaming && crate::cache::is_cache_eligible(&request, headers);
    let cache_key_value = crate::cache::cache_key(
        &request.model,
        &request,
        key.as_ref().map(|key| key.id.as_str()),
    );
    let started = Instant::now();

    let mut attempts: Vec<RoutingAttempt> = Vec::new();
    let policy = state.routing.policy();
    let mut fallback_count: u32 = 0;
    let mut last_error: Option<(UpstreamCallError, RoutingTarget)> = None;

    for target in targets {
        if !policy.allows_attempt(fallback_count) {
            break;
        }
        if ctx.circuit.is_open(&target.provider_id) {
            attempts.push(failover::trace(&target, 0, 0.0, Some("circuit open")));
            continue;
        }

        let rotate = ctx.key_pool.entries(&target.provider_id).await.len() > 1;
        let mut rotations = 0usize;

        loop {
            let outcome = attempt(&ctx, state, &target, &request, streaming).await;
            attempts.push(outcome.trace);

            match outcome.result {
                Ok(AttemptSuccess::Complete(completion)) => {
                    let mut completion = completion;
                    let redacted = failover::redact_completion(
                        state,
                        &mut completion,
                        key.as_ref(),
                        "/v1/chat/completions",
                    )
                    .await;
                    let cache_eligible = cache_eligible && !redacted;
                    let context = Finish {
                        target: &target,
                        attempts,
                        fallback_count,
                        started,
                        cache_key_value: &cache_key_value,
                        cache_eligible,
                        semantic_request: request.clone(),
                        headers,
                        redact,
                        request_id,
                        rate_limits: limits,
                        virtual_key_id: key.as_ref().map(|key| key.id.clone()),
                    };
                    return finish::finish_completion(state, *completion, context, telemetry).await;
                }
                Ok(AttemptSuccess::Stream(opened)) => {
                    let unit_costs =
                        super::prefetch_unit_costs(state, &target.upstream_model).await;

                    let mut setup_telemetry = telemetry.clone();
                    setup_telemetry.routing_trace = attempts;
                    setup_telemetry.fallback_count = fallback_count;

                    return Ok(stream_response(
                        state.clone(),
                        *opened,
                        StreamSetup {
                            telemetry: setup_telemetry,
                            attempts: Vec::new(),
                            fallback_count,
                            estimated_input_tokens: AtomicU32::new(failover::input_tokens(
                                &request,
                            )),
                            redact,
                            unit_costs,
                            request_id: request_id.to_string(),
                            rate_limits: limits,
                            request: request.clone(),
                            circuit: Arc::clone(state.routing.circuit()),
                        },
                        super::chat_sse(),
                    ));
                }
                Err(err) if err.is_retryable_under(&policy) => {
                    fallback_count += 1;
                    let max_rotations =
                        (policy.key_rotations as usize).min(KEY_ROTATION_BACKOFF.len());
                    if rotate && rotations < max_rotations {
                        tokio::time::sleep(KEY_ROTATION_BACKOFF[rotations]).await;
                        rotations += 1;
                        continue;
                    }
                    last_error = Some((err, target));
                    break;
                }
                Err(err) => {
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

const KEY_ROTATION_BACKOFF: [Duration; 2] =
    [Duration::from_millis(100), Duration::from_millis(250)];

enum AttemptSuccess {
    Complete(Box<ChatCompletionResponse>),
    Stream(Box<OpenedStream>),
}

struct AttemptOutcome {
    trace: RoutingAttempt,
    result: Result<AttemptSuccess, UpstreamCallError>,
}

async fn attempt(
    ctx: &AttemptContext<'_>,
    state: &AppState,
    target: &RoutingTarget,
    request: &ChatCompletionRequest,
    streaming: bool,
) -> AttemptOutcome {
    let bound = failover::bind_target(ctx, target).await;
    let key_id = bound.selected_key_id.clone();
    let attempted_at = Instant::now();

    if streaming {
        return match open_stream(state, &bound, request).await {
            Ok(opened) => {
                failover::note_success(ctx, &bound, key_id.as_deref(), opened.ttft_ms);
                AttemptOutcome {
                    trace: failover::trace(&bound, 200, opened.ttft_ms, None),
                    result: Ok(AttemptSuccess::Stream(Box::new(opened))),
                }
            }
            Err(err) => {
                failover::note_failure(ctx, &bound, key_id.as_deref(), &err);
                AttemptOutcome {
                    trace: failover::trace(
                        &bound,
                        err.status_code(),
                        attempted_at.elapsed().as_secs_f64() * 1000.0,
                        Some(&err.to_string()),
                    ),
                    result: Err(err),
                }
            }
        };
    }

    match adapter_for(bound.kind)
        .complete(&state.http_client, &bound.endpoint(), request)
        .await
    {
        Ok(completion) => {
            let latency_ms = attempted_at.elapsed().as_secs_f64() * 1000.0;
            failover::note_success(ctx, &bound, key_id.as_deref(), latency_ms);
            AttemptOutcome {
                trace: failover::trace(&bound, 200, latency_ms, None),
                result: Ok(AttemptSuccess::Complete(Box::new(completion))),
            }
        }
        Err(err) => {
            failover::note_failure(ctx, &bound, key_id.as_deref(), &err);
            AttemptOutcome {
                trace: failover::trace(
                    &bound,
                    err.status_code(),
                    attempted_at.elapsed().as_secs_f64() * 1000.0,
                    Some(&err.to_string()),
                ),
                result: Err(err),
            }
        }
    }
}
