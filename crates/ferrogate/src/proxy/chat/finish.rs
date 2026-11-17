use std::sync::atomic::Ordering;

use axum::{
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value;

use crate::{
    auth::limiter::RateLimitSnapshot,
    cache::{cache_hit_body, cache_key, is_cache_eligible, CACHE_HIT_HEADER},
    error::AppResult,
    providers::{ChatCompletionRequest, ChatCompletionResponse, UsageInfo},
    routing::RoutingTarget,
    semantic::SEMANTIC_HIT_HEADER,
    telemetry::{RoutingAttempt, TelemetryEvent},
    AppState,
};

use super::super::{
    failover,
    headers::{attach_gateway_headers, attach_rate_limit_headers},
};

pub(crate) struct Finish<'a> {
    pub target: &'a RoutingTarget,
    pub attempts: Vec<RoutingAttempt>,
    pub fallback_count: u32,
    pub started: std::time::Instant,
    pub cache_key_value: &'a str,
    pub cache_eligible: bool,
    pub semantic_request: ChatCompletionRequest,
    pub headers: &'a HeaderMap,
    pub redact: bool,
    pub request_id: &'a str,
    pub rate_limits: RateLimitSnapshot,
    pub virtual_key_id: Option<String>,
}

pub(crate) async fn finish_completion(
    state: &AppState,
    mut completion: ChatCompletionResponse,
    context: Finish<'_>,
    mut telemetry: TelemetryEvent,
) -> AppResult<Response> {
    let target = context.target;
    completion.model = target.upstream_model.clone();

    let input_tokens = completion
        .usage
        .as_ref()
        .map(|usage| usage.prompt_tokens)
        .unwrap_or(telemetry.input_tokens);

    let output_tokens = completion
        .usage
        .as_ref()
        .map(|usage| usage.completion_tokens)
        .unwrap_or_else(|| {
            crate::tokenizer::count_text_tokens(
                &target.upstream_model,
                &first_message_text(&completion).unwrap_or_default(),
            )
        });

    if completion.usage.is_none() {
        completion.usage = Some(UsageInfo {
            prompt_tokens: input_tokens,
            completion_tokens: output_tokens,
            total_tokens: input_tokens + output_tokens,
        });
    }

    let cost_usd = state
        .pricing
        .compute_cost_usd(&target.upstream_model, input_tokens, output_tokens)
        .await;

    telemetry.status_code = 200;
    telemetry.input_tokens = input_tokens;
    telemetry.output_tokens = output_tokens;
    telemetry.cost_usd = cost_usd;
    telemetry.latency_ms = context.started.elapsed().as_secs_f64() * 1000.0;
    telemetry.fallback_count = context.fallback_count;
    telemetry.routing_trace = context.attempts;
    telemetry.routed_provider_id = Some(target.provider_id.clone());
    telemetry.routed_provider_name = Some(target.provider_name.clone());
    telemetry.routed_model = Some(target.upstream_model.clone());
    telemetry.response_text = if context.redact {
        None
    } else {
        first_message_text(&completion)
    };
    failover::book_spend(state, telemetry.virtual_key_id.as_deref(), cost_usd);
    state.telemetry.record(telemetry);

    if context.cache_eligible && !context.redact {
        state
            .response_cache
            .insert(context.cache_key_value.to_string(), completion.clone())
            .await;
        state
            .semantic
            .store(
                state,
                &context.semantic_request,
                context.headers,
                &completion,
                Some(&target.provider_name),
                context.virtual_key_id.as_deref(),
            )
            .await;
    }

    Ok(completion_response(
        serde_json::to_value(&completion)?,
        context.request_id,
        target,
        context.fallback_count,
        Some(cost_usd),
        context.rate_limits,
    ))
}

pub(crate) fn completion_response(
    body: Value,
    request_id: &str,
    target: &RoutingTarget,
    fallbacks: u32,
    cost_usd: Option<f64>,
    rate_limits: RateLimitSnapshot,
) -> Response {
    let mut response = Json(body).into_response();
    attach_gateway_headers(
        response.headers_mut(),
        request_id,
        &target.provider_name,
        &target.upstream_model,
        fallbacks,
        cost_usd,
    );
    attach_rate_limit_headers(response.headers_mut(), rate_limits);
    response
}

pub(crate) async fn cache_hit(
    state: &AppState,
    request: &ChatCompletionRequest,
    headers: &HeaderMap,
    request_id: &str,
    limits: RateLimitSnapshot,
    key: Option<&super::super::AuthenticatedKey>,
    verdict: &crate::guardrails::InputVerdict,
) -> Option<Response> {
    if request.stream.unwrap_or(false) || !is_cache_eligible(request, headers) {
        return None;
    }

    let cached = state
        .response_cache
        .get(&cache_key(
            &request.model,
            request,
            key.map(|key| key.id.as_str()),
        ))
        .await?;
    let usage = cached.usage.clone().unwrap_or_default();
    let cost_usd = state
        .pricing
        .compute_cost_usd(&cached.model, usage.prompt_tokens, usage.completion_tokens)
        .await;

    let redact = state.redact_prompts.load(Ordering::Relaxed);
    let mut event = TelemetryEvent::new(&request.model, false);
    event.request_id = Some(request_id.to_string());
    if let Some(key) = key {
        event.virtual_key_id = Some(key.id.clone());
        event.virtual_key_name = Some(key.name.clone());
    }
    failover::apply_verdict(&mut event, verdict);
    event.cache_hit = true;
    event.input_tokens = usage.prompt_tokens;
    event.output_tokens = usage.completion_tokens;
    event.cost_usd = cost_usd;
    event.routed_provider_name = Some("cache".to_string());
    event.routed_model = Some(cached.model.clone());
    event.response_text = if redact {
        None
    } else {
        first_message_text(&cached)
    };
    failover::book_spend(state, key.map(|key| key.id.as_str()), cost_usd);
    state.telemetry.record(event);

    let mut response = Json(cache_hit_body(&cached)).into_response();
    response.headers_mut().insert(
        CACHE_HIT_HEADER,
        axum::http::HeaderValue::from_static("HIT"),
    );
    attach_gateway_headers(
        response.headers_mut(),
        request_id,
        "cache",
        &cached.model,
        0,
        Some(cost_usd),
    );
    attach_rate_limit_headers(response.headers_mut(), limits);
    Some(response)
}

pub(crate) async fn semantic_hit(
    state: &AppState,
    request: &ChatCompletionRequest,
    headers: &HeaderMap,
    request_id: &str,
    limits: RateLimitSnapshot,
    key: Option<&super::super::AuthenticatedKey>,
    verdict: &crate::guardrails::InputVerdict,
) -> Option<Response> {
    let resolution = state
        .semantic
        .lookup(
            state,
            request,
            headers,
            request_id,
            key.map(|key| key.id.as_str()),
            verdict,
        )
        .await?;
    failover::book_spend(state, key.map(|key| key.id.as_str()), resolution.cost_usd);
    let mut body = serde_json::to_value(&resolution.response).ok()?;
    body["id"] = Value::String(resolution.response.id.clone());

    let mut http = Json(body).into_response();
    http.headers_mut().insert(
        CACHE_HIT_HEADER,
        axum::http::HeaderValue::from_static(SEMANTIC_HIT_HEADER),
    );
    attach_gateway_headers(
        http.headers_mut(),
        request_id,
        "cache",
        &resolution.response.model,
        0,
        Some(resolution.cost_usd),
    );
    attach_rate_limit_headers(http.headers_mut(), limits);
    Some(http)
}

pub(crate) fn first_message_text(completion: &ChatCompletionResponse) -> Option<String> {
    completion
        .choices
        .first()
        .map(|choice| crate::providers::message_content_to_text(&choice.message.content))
        .filter(|text| !text.is_empty())
}
