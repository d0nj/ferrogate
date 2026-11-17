use std::time::Instant;

use axum::{
    extract::{Extension, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value;

use crate::{
    error::{AppError, AppResult},
    providers::{adapter_for, ChatCompletionRequest, ChatMessage, UpstreamCallError},
    routing::{extract_session_id, RoutingTarget, SelectedKey, SessionAffinity},
    telemetry::RoutingAttempt,
    AppState,
};

use super::super::{
    chat::completion_response,
    failover::{self, AttemptContext},
    headers::request_id_from_headers,
    AuthenticatedKey,
};
use super::translate::{typesafe_models, Dialect};

pub(crate) async fn handle_openai(
    State(state): State<AppState>,
    headers: HeaderMap,
    authenticated: Option<Extension<AuthenticatedKey>>,
    Json(raw): Json<Value>,
) -> AppResult<Response> {
    handle(state, headers, authenticated, raw, Dialect::OpenAi).await
}

pub(crate) async fn handle_typesafe(
    State(state): State<AppState>,
    headers: HeaderMap,
    authenticated: Option<Extension<AuthenticatedKey>>,
    Json(raw): Json<Value>,
) -> AppResult<Response> {
    handle(state, headers, authenticated, raw, Dialect::TypeSafe).await
}

pub(crate) async fn handle_gemini(
    State(state): State<AppState>,
    headers: HeaderMap,
    authenticated: Option<Extension<AuthenticatedKey>>,
    Json(raw): Json<Value>,
) -> AppResult<Response> {
    handle(state, headers, authenticated, raw, Dialect::Gemini).await
}

async fn handle(
    state: AppState,
    headers: HeaderMap,
    authenticated: Option<Extension<AuthenticatedKey>>,
    raw: Value,
    dialect: Dialect,
) -> AppResult<Response> {
    let request_id = request_id_from_headers(&headers);
    let key = authenticated.map(|Extension(key)| key);
    let request = dialect.decode(&raw)?;
    if request.model.trim().is_empty() {
        return Err(AppError::BadRequest(
            "field 'model' must be a non-empty string".to_string(),
        ));
    }

    let limits = super::super::rate_limits(key.as_ref(), &state);
    let input_tokens = dialect.input_tokens(&request);
    failover::check_limits(&state, &request.model, key.as_ref(), input_tokens).await?;

    let mut probe = ChatCompletionRequest {
        model: request.model.clone(),
        messages: vec![ChatMessage::text("user", decision_text(&request))],
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
    };
    let input_verdict =
        failover::check_guardrails(&state, &mut probe, key.as_ref(), dialect.endpoint()).await?;

    let mut targets = state
        .routing
        .resolve_ordered(&request.model, &state.ewma, &state.pricing)
        .await?;

    let session_id = extract_session_id(&headers);
    let affinity_scope =
        SessionAffinity::tenant_scope(dialect.endpoint(), key.as_ref().map(|key| key.id.as_str()));
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
        scope: dialect.endpoint(),
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
        dialect.endpoint(),
        &request_id,
        key.as_ref(),
        state
            .redact_prompts
            .load(std::sync::atomic::Ordering::Relaxed),
        Some(serde_json::to_string(&raw).unwrap_or_default()),
    );
    failover::apply_verdict(&mut telemetry, &input_verdict);
    telemetry.input_tokens = input_tokens;

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
                .decide(&state.http_client, &target.endpoint(), &request)
                .await
            {
                Ok(response) => {
                    let latency_ms = attempted_at.elapsed().as_secs_f64() * 1000.0;
                    failover::note_success(&ctx, &target, key_id.as_deref(), latency_ms);
                    attempts.push(failover::trace(&target, 200, latency_ms, None));
                    return finish(
                        &state,
                        dialect,
                        response,
                        target,
                        &request_id,
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

fn decision_text(request: &crate::providers::decisions::UnifiedDecisionRequest) -> String {
    let mut parts = vec![request.text_input()];
    for question in &request.questions {
        parts.push(question.resolved_instructions().to_string());
        let (when_true, when_false) = question.resolved_descriptions();
        parts.push(when_true);
        parts.push(when_false);
    }
    parts
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
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
    dialect: Dialect,
    mut response: crate::providers::decisions::UnifiedDecisionResponse,
    target: RoutingTarget,
    request_id: &str,
    limits: crate::auth::limiter::RateLimitSnapshot,
    mut telemetry: crate::telemetry::TelemetryEvent,
    attempts: Vec<RoutingAttempt>,
    started: Instant,
    fallbacks: u32,
) -> AppResult<Response> {
    response.model = target.upstream_model.clone();
    let usage = response.usage.clone().unwrap_or_default();
    let cost_usd = state
        .pricing
        .compute_cost_usd(
            &target.upstream_model,
            usage.input_tokens,
            usage.output_tokens,
        )
        .await;

    telemetry.status_code = 200;
    telemetry.input_tokens = usage.input_tokens;
    telemetry.output_tokens = usage.output_tokens;
    telemetry.cost_usd = cost_usd;
    telemetry.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    telemetry.fallback_count = fallbacks;
    telemetry.routing_trace = attempts;
    telemetry.routed_provider_id = Some(target.provider_id.clone());
    telemetry.routed_provider_name = Some(target.provider_name.clone());
    telemetry.routed_model = Some(target.upstream_model.clone());
    let answer_json = serde_json::to_string(&response.answer_map()).unwrap_or_default();
    let verdict = state
        .guardrails
        .filter_output(
            &answer_json,
            telemetry.virtual_key_id.as_deref(),
            dialect.endpoint(),
        )
        .await;
    telemetry.response_text = Some(verdict.text);
    failover::book_spend(state, telemetry.virtual_key_id.as_deref(), cost_usd);
    state.telemetry.record(telemetry);

    Ok(completion_response(
        dialect.encode(&response),
        request_id,
        &target,
        fallbacks,
        Some(cost_usd),
        limits,
    ))
}

pub(crate) async fn list_typesafe_models(
    State(state): State<AppState>,
    authenticated: Option<Extension<AuthenticatedKey>>,
) -> AppResult<Response> {
    let models = state.routing.list_available_models().await?;

    let allowed = authenticated
        .map(|Extension(key)| key.allowed_models)
        .unwrap_or_default();

    let models: Vec<String> = models
        .into_iter()
        .filter(|id| allowed.is_empty() || allowed.iter().any(|allowed| allowed == id))
        .collect();
    Ok(Json(typesafe_models(models)).into_response())
}
