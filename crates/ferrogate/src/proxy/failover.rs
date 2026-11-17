use std::sync::Arc;

use crate::{
    error::{AppError, AppResult},
    guardrails::InputVerdict,
    providers::{ChatCompletionRequest, UpstreamCallError},
    routing::{
        CircuitBreakerRegistry, EwmaRegistry, ProviderKeyPool, RoutingTarget, SessionAffinity,
        SessionBinding,
    },
    telemetry::{RoutingAttempt, TelemetryEvent},
    tokenizer::count_prompt_tokens,
    AppState,
};

use super::AuthenticatedKey;

pub(crate) struct AttemptContext<'a> {
    pub state: &'a AppState,
    pub request_model: &'a str,
    pub scope: &'a str,
    pub session_id: Option<&'a str>,
    pub key_id: Option<&'a str>,
    pub circuit: &'a Arc<CircuitBreakerRegistry>,
    pub ewma: &'a EwmaRegistry,
    pub key_pool: &'a ProviderKeyPool,
    pub affinity: &'a SessionAffinity,
}

pub(crate) async fn bind_target(ctx: &AttemptContext<'_>, base: &RoutingTarget) -> RoutingTarget {
    if ctx.key_pool.entries(&base.provider_id).await.is_empty() {
        return base.clone();
    }

    let sticky_salt = ctx.session_id.unwrap_or("");
    let scope = crate::routing::SessionAffinity::tenant_scope(ctx.scope, ctx.key_id);
    let preferred = ctx
        .affinity
        .get_with_redis(
            ctx.state.redis.as_deref(),
            &scope,
            sticky_salt,
            ctx.request_model,
        )
        .await
        .filter(|binding| binding.provider_id == base.provider_id)
        .and_then(|binding| binding.key_id);

    let selected = ctx
        .key_pool
        .select_key_for(
            sticky_salt,
            &base.provider_id,
            &base.api_key,
            &base.upstream_model,
            preferred.as_deref(),
        )
        .await;

    let mut target = base.clone();
    target.api_key = selected.api_key;
    target.selected_key_id = selected.id.clone();
    target.selected_key_name = selected.name;

    if let (Some(key_id), Some(session_id)) = (selected.id, ctx.session_id) {
        if !session_id.is_empty() {
            let binding = SessionBinding {
                provider_id: target.provider_id.clone(),
                key_id: Some(key_id),
            };
            let scope = crate::routing::SessionAffinity::tenant_scope(ctx.scope, ctx.key_id);
            ctx.affinity
                .set_with_redis(
                    ctx.state.redis.as_deref(),
                    &scope,
                    session_id,
                    ctx.request_model,
                    &binding,
                )
                .await;
        }
    }

    target
}

pub(crate) fn note_success(
    ctx: &AttemptContext<'_>,
    target: &RoutingTarget,
    key_id: Option<&str>,
    latency_ms: f64,
) {
    ctx.circuit.record_success(&target.provider_id);
    if let Some(key_id) = key_id {
        ctx.key_pool.record_success(key_id);
    }
    ctx.ewma
        .record(&target.provider_id, &target.upstream_model, latency_ms);
}

pub(crate) fn note_failure(
    ctx: &AttemptContext<'_>,
    target: &RoutingTarget,
    key_id: Option<&str>,
    err: &UpstreamCallError,
) {
    if err.class() != crate::routing::FailureClass::ClientError {
        ctx.circuit
            .record_failure(&target.provider_id, err.to_string());
    }
    if let Some(key_id) = key_id {
        ctx.key_pool
            .record_failure(key_id, Some(err.status_code()), &err.to_string());
    }
    if err.status_code() == 429 {
        return;
    }
    let Some(session_id) = ctx.session_id.filter(|id| !id.is_empty()) else {
        return;
    };
    let scope = crate::routing::SessionAffinity::tenant_scope(ctx.scope, ctx.key_id);
    let model = ctx.request_model.to_string();
    let session_id = session_id.to_string();
    let affinity = Arc::clone(&ctx.state.affinity);
    let redis = ctx.state.redis.clone();
    tokio::spawn(run_invalidation(affinity, redis, scope, model, session_id));
}

async fn run_invalidation(
    affinity: Arc<SessionAffinity>,
    redis: Option<Arc<crate::infra::redis::RedisStore>>,
    scope: String,
    model: String,
    session_id: String,
) {
    affinity
        .invalidate_with_redis(redis.as_deref(), &scope, &session_id, &model)
        .await;
}

pub(crate) fn telemetry(
    request: &ChatCompletionRequest,
    streaming: bool,
    endpoint: &'static str,
    request_id: &str,
    key: Option<&AuthenticatedKey>,
    redact: bool,
) -> TelemetryEvent {
    telemetry_for(
        &request.model,
        streaming,
        endpoint,
        request_id,
        key,
        redact,
        Some(serde_json::to_string(&request.messages).unwrap_or_default()),
    )
}

pub(crate) fn telemetry_for(
    model: &str,
    streaming: bool,
    endpoint: &'static str,
    request_id: &str,
    key: Option<&AuthenticatedKey>,
    redact: bool,
    transcript: Option<String>,
) -> TelemetryEvent {
    let mut event = super::base_event(model, streaming, endpoint, request_id, key);
    event.request_messages_json = if redact { None } else { transcript };
    event
}

pub(crate) async fn check_guardrails(
    state: &AppState,
    request: &mut ChatCompletionRequest,
    key: Option<&AuthenticatedKey>,
    endpoint: &'static str,
) -> AppResult<InputVerdict> {
    let verdict = state
        .guardrails
        .check_input(request, key.map(|key| key.id.as_str()), endpoint)
        .await;

    if let Err(AppError::Forbidden(message)) = &verdict {
        if let Some(name) = message
            .strip_prefix("blocked by guardrail '")
            .and_then(|rest| rest.strip_suffix('\''))
        {
            state.metrics.record_block(name);
        }
    }

    verdict
}

pub(crate) fn apply_verdict(telemetry: &mut TelemetryEvent, verdict: &InputVerdict) {
    if verdict.is_clean() {
        return;
    }

    telemetry.guardrail_action =
        crate::guardrails::action_label(&verdict.flags, &verdict.redactions);
    if let Some(label) = telemetry.guardrail_action.clone() {
        tracing::info!(guardrail_action = %label, "guardrail verdict");
    }
}

pub(crate) async fn filter_guardrail_output(
    state: &AppState,
    text: &str,
    key: Option<&AuthenticatedKey>,
    endpoint: &'static str,
) -> Option<String> {
    let verdict = state
        .guardrails
        .filter_output(text, key.map(|key| key.id.as_str()), endpoint)
        .await;

    verdict.redacted.then_some(verdict.text)
}

pub(crate) async fn redact_completion(
    state: &AppState,
    completion: &mut crate::providers::ChatCompletionResponse,
    key: Option<&AuthenticatedKey>,
    endpoint: &'static str,
) -> bool {
    let Some(content) = completion.content_mut() else {
        return false;
    };
    let text = crate::providers::message_content_to_text(content);
    if text.is_empty() {
        return false;
    }

    let Some(filtered) = filter_guardrail_output(state, &text, key, endpoint).await else {
        return false;
    };

    crate::providers::rewrite_content(content, &filtered);
    true
}

pub(crate) async fn check_limits(
    state: &AppState,
    model: &str,
    key: Option<&AuthenticatedKey>,
    input_tokens: u32,
) -> AppResult<()> {
    if let Some(key) = key {
        state.auth.check_model_allowed(key, model)?;
        state.auth.check_tpm(key, input_tokens).await?;
    }
    Ok(())
}

pub(crate) fn input_tokens(request: &ChatCompletionRequest) -> u32 {
    count_prompt_tokens(&request.model, &request.messages)
}

pub(crate) fn trace(
    target: &RoutingTarget,
    status_code: u16,
    latency_ms: f64,
    error: Option<&str>,
) -> RoutingAttempt {
    RoutingAttempt {
        provider_id: target.provider_id.clone(),
        provider_name: target.provider_name.clone(),
        model: target.upstream_model.clone(),
        status_code,
        latency_ms,
        error: error.map(str::to_string),
    }
}

pub(crate) fn book_spend(state: &AppState, key_id: Option<&str>, cost_usd: f64) {
    if let (Some(key_id), true) = (key_id, cost_usd > 0.0) {
        crate::telemetry::add_live_spend(&state.live_spend, key_id, cost_usd);
    }
}

pub(crate) fn record_failure(
    state: &AppState,
    mut telemetry: TelemetryEvent,
    attempts: Vec<RoutingAttempt>,
    err: &UpstreamCallError,
    target: Option<&RoutingTarget>,
    fallback_count: u32,
    started: std::time::Instant,
) {
    telemetry.status_code = i64::from(err.status_code());
    telemetry.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    telemetry.fallback_count = fallback_count;
    telemetry.routing_trace = attempts;
    telemetry.routed_provider_id = target.map(|target| target.provider_id.clone());
    telemetry.routed_provider_name = target.map(|target| target.provider_name.clone());
    telemetry.routed_model = target.map(|target| target.upstream_model.clone());
    telemetry.error_message = Some(err.to_string());
    telemetry.response_text = None;
    state.telemetry.record(telemetry);
}

pub(crate) fn exhausted_error(err: &UpstreamCallError) -> crate::error::AppError {
    if err.status_code() == 429 {
        return crate::error::AppError::TooManyRequests(
            "all upstream providers are rate limited".to_string(),
        );
    }
    crate::error::AppError::Upstream(format!(
        "all upstream providers failed (last status {})",
        err.status_code()
    ))
}
