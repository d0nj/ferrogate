use axum::http::{
    header::{COOKIE, HOST, ORIGIN},
    HeaderMap, Method, Uri,
};
use cookie::{Cookie, CookieJar};

use crate::{
    error::{AppError, AppResult},
    AppState,
};

use super::{
    audit,
    auth::{deny, SESSION_COOKIE, UNAUTHENTICATED_LABEL},
};

pub(super) async fn verify_csrf(
    state: &AppState,
    method: &Method,
    headers: &HeaderMap,
    path: &str,
) -> AppResult<()> {
    let Some(reason) = csrf_violation(method, headers) else {
        return Ok(());
    };

    Err(deny(
        state,
        audit::Actor::Anonymous(UNAUTHENTICATED_LABEL),
        method,
        path,
        AppError::Forbidden(reason.to_string()),
        reason,
    )
    .await)
}

pub(super) fn csrf_violation(method: &Method, headers: &HeaderMap) -> Option<&'static str> {
    const SAFE_METHODS: [Method; 3] = [Method::GET, Method::HEAD, Method::OPTIONS];
    if SAFE_METHODS.contains(method) {
        return None;
    }

    if header_str(headers, "sec-fetch-mode") == Some("navigate") {
        return Some("cross-site request rejected");
    }

    match header_str(headers, "sec-fetch-site") {
        Some("same-origin" | "none") => None,
        Some(_) => Some("cross-site request rejected"),
        None => match (header_str(headers, ORIGIN), header_str(headers, HOST)) {
            (Some(origin), Some(host)) if origin_matches_host(origin, host) => None,
            _ => Some("missing same-origin proof"),
        },
    }
}

fn origin_matches_host(origin: &str, host: &str) -> bool {
    let Ok(uri) = origin.parse::<Uri>() else {
        return false;
    };
    uri.scheme().is_some() && uri.authority().map(|authority| authority.as_str()) == Some(host)
}

pub(super) fn is_https(
    headers: &HeaderMap,
    peer: Option<std::net::SocketAddr>,
    trusted: &[ipnet::IpNet],
) -> bool {
    let peer_trusted = peer.is_some_and(|addr| trusted.iter().any(|net| net.contains(&addr.ip())));
    if !peer_trusted {
        return false;
    }
    if header_str(headers, "x-forwarded-proto")
        .is_some_and(|value| value.eq_ignore_ascii_case("https"))
    {
        return true;
    }
    header_str(headers, "forwarded").is_some_and(|value| {
        value
            .split(';')
            .any(|part| part.trim().eq_ignore_ascii_case("proto=https"))
    })
}

pub(super) fn session_token(headers: &HeaderMap) -> Option<String> {
    let raw = header_str(headers, COOKIE)?;
    let mut jar = CookieJar::new();
    for cookie in Cookie::split_parse(raw).flatten() {
        jar.add_original(cookie.into_owned());
    }
    jar.get(SESSION_COOKIE)
        .map(|cookie| cookie.value().trim().to_string())
        .filter(|token| !token.is_empty())
}

fn header_str(headers: &HeaderMap, name: impl axum::http::header::AsHeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}
