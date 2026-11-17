use axum::http::{HeaderMap, HeaderName, HeaderValue};
use uuid::Uuid;

use crate::auth::limiter::RateLimitSnapshot;

pub(crate) fn request_id_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.is_ascii())
        .map(str::to_string)
        .unwrap_or_else(|| format!("req-{}", Uuid::new_v4()))
}

pub(crate) fn attach_gateway_headers(
    headers: &mut HeaderMap,
    request_id: &str,
    provider: &str,
    model: &str,
    fallbacks: u32,
    cost_usd: Option<f64>,
) {
    insert(headers, "x-request-id", request_id);
    insert(headers, "x-ferrogate-provider", provider);
    insert(headers, "x-ferrogate-model", model);
    insert(headers, "x-ferrogate-fallbacks", &fallbacks.to_string());

    if let Some(cost) = cost_usd {
        insert(headers, "x-ferrogate-cost-usd", &format!("{cost:.6}"));
    }
}

pub(crate) fn attach_rate_limit_headers(headers: &mut HeaderMap, snapshot: RateLimitSnapshot) {
    insert_number(
        headers,
        "x-ratelimit-limit-requests",
        snapshot.limit_requests,
    );
    insert_number(
        headers,
        "x-ratelimit-remaining-requests",
        snapshot.remaining_requests,
    );
    insert_number(headers, "x-ratelimit-limit-tokens", snapshot.limit_tokens);
    insert_number(
        headers,
        "x-ratelimit-remaining-tokens",
        snapshot.remaining_tokens,
    );
}

fn insert(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(HeaderName::from_static(name), value);
    }
}

fn insert_number<T: ToString + Copy>(
    headers: &mut HeaderMap,
    name: &'static str,
    value: Option<T>,
) {
    if let Some(value) = value {
        insert(headers, name, &value.to_string());
    }
}
