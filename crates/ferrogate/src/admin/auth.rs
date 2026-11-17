use std::{
    net::SocketAddr,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{ConnectInfo, FromRequestParts, Request, State},
    http::{header::SET_COOKIE, request::Parts, Extensions, HeaderMap, Method},
    middleware::Next,
    response::{AppendHeaders, IntoResponse, Response},
    Json,
};
use cookie::{Cookie, SameSite};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, ModelTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::password,
    auth::role::Role,
    auth::session::{self, Principal, Session},
    auth::throttle,
    entities::user,
    error::{AppError, AppResult},
    AppState,
};

use super::{
    audit,
    csrf::{csrf_violation, is_https, session_token, verify_csrf},
    policy::{audit_action, is_mutating, is_session_route, required_capability},
};

pub const SESSION_TTL_SECS: u64 = session::SESSION_TTL_SECS;

pub(super) const LOGIN_PATH: &str = "/admin/api/session/login";

pub const SESSION_COOKIE: &str = "fg_session";

const ZERO_TTL: cookie::time::Duration = cookie::time::Duration::ZERO;

pub(super) const UNAUTHENTICATED_LABEL: &str = "unauthenticated";

pub async fn find_session(state: &AppState, token: &str) -> Option<Session> {
    state.sessions().get(token).await
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub expires_in: u64,
}

pub async fn login(
    State(state): State<AppState>,
    parts: Parts,
    Json(body): Json<LoginRequest>,
) -> AppResult<Response> {
    let headers = parts.headers;
    if let Some(reason) = csrf_violation(&Method::POST, &headers) {
        return Err(AppError::Forbidden(reason.to_string()));
    }

    let peer = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);

    let client = throttle::client_key(&headers, peer, &state.config.trusted_proxies);
    if !state.login_throttle.is_allowed(&client) {
        return Err(AppError::TooManyRequests(
            "too many sign-in attempts; try again shortly".to_string(),
        ));
    }

    let (Some(email), Some(password)) = (body.email.as_deref(), body.password.as_deref()) else {
        return Err(invalid_credentials());
    };

    let result = login_with_password(&state, &headers, email, password, peer).await;
    if result.is_ok() {
        state.login_throttle.clear(&client);
    }
    result
}

async fn login_with_password(
    state: &AppState,
    headers: &HeaderMap,
    email: &str,
    password: &str,
    peer: Option<SocketAddr>,
) -> AppResult<Response> {
    let normalized = email.trim().to_lowercase();
    let row = user::Entity::find()
        .filter(user::Column::Email.eq(&normalized))
        .one(&state.db)
        .await?;

    let Some(row) = row else {
        password::verify_dummy_offloaded(password.to_string()).await;
        return Err(invalid_credentials());
    };

    let verified =
        password::verify_password_offloaded(password.to_string(), row.password_hash.clone()).await;
    if !verified || row.is_active == 0 {
        return Err(invalid_credentials());
    }

    let role = Role::from_str_strict(&row.role).unwrap_or(Role::Viewer);
    let session = state
        .sessions()
        .mint(row.id.clone(), row.email.clone(), role)
        .await;

    let now = super::now_rfc3339();
    let mut active: user::ActiveModel = row.into();
    active.last_login_at = Set(Some(now));
    active.update(&state.db).await?;

    let cookie = session_cookie(
        &session.token,
        state
            .config
            .cookie_secure
            .unwrap_or_else(|| is_https(headers, peer, &state.config.trusted_proxies)),
        session_max_age(),
    );
    Ok((
        AppendHeaders([(SET_COOKIE, cookie)]),
        Json(LoginResponse {
            expires_in: SESSION_TTL_SECS,
        }),
    )
        .into_response())
}

fn invalid_credentials() -> AppError {
    AppError::Unauthorized("invalid email or password".to_string())
}

async fn delete_playground_keys(state: &AppState) {
    use crate::entities::virtual_key;

    let rows = virtual_key::Entity::find()
        .filter(virtual_key::Column::TagsJson.contains("playground"))
        .all(&state.db)
        .await
        .unwrap_or_default();
    for row in rows {
        state.auth.invalidate_key_hash(&row.key_hash).await;
        state.auth.reset_key_state(&row.id, true);
        if let Err(error) = row.delete(&state.db).await {
            tracing::warn!(error = %error, "failed to delete playground key");
        }
    }
}

pub async fn session(CurrentPrincipal(principal): CurrentPrincipal) -> Json<Value> {
    Json(json!({
        "id": principal.user_id,
        "email": principal.email,
        "role": principal.role.as_str(),
    }))
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = session_token(&headers) {
        state.sessions().revoke(&token).await;
    }
    delete_playground_keys(&state).await;

    let cleared = Cookie::build((SESSION_COOKIE, ""))
        .path("/")
        .max_age(ZERO_TTL)
        .build();
    (
        AppendHeaders([(SET_COOKIE, cleared.to_string())]),
        Json(json!({ "logged_out": true })),
    )
        .into_response()
}

fn session_cookie(token: &str, secure: bool, ttl: cookie::time::Duration) -> String {
    Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .max_age(ttl)
        .build()
        .to_string()
}

fn session_max_age() -> cookie::time::Duration {
    cookie::time::Duration::seconds(SESSION_TTL_SECS as i64)
}

#[derive(Debug, Clone)]
pub struct CurrentPrincipal(pub Principal);

impl<S: Send + Sync> FromRequestParts<S> for CurrentPrincipal {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        principal_of(&parts.extensions)
            .map(Self)
            .ok_or_else(missing_credentials)
    }
}

fn principal_of(extensions: &Extensions) -> Option<Principal> {
    extensions.get::<Principal>().cloned()
}

async fn resolve_session_principal(state: &AppState, token: &str) -> Option<Principal> {
    let session = find_session(state, token).await?;

    let found = match user::Entity::find_by_id(&session.user_id)
        .one(&state.db)
        .await
    {
        Ok(found) => found,
        Err(error) => {
            tracing::warn!(%error, "failed to load user for admin session");
            return None;
        }
    };

    let Some(row) = found.filter(|row| row.is_active != 0) else {
        state.sessions().revoke(token).await;
        return None;
    };

    Some(Principal {
        user_id: row.id,
        email: row.email,
        role: Role::from_str_strict(&row.role).unwrap_or(session.role),
        must_change_password: row.must_change_password != 0,
    })
}

pub(super) async fn deny(
    state: &AppState,
    actor: audit::Actor<'_>,
    method: &Method,
    path: &str,
    error: AppError,
    detail: &str,
) -> AppError {
    if let Err(write_error) = audit::record(
        &state.db,
        actor,
        &audit_action(method, path),
        Some(path),
        audit::OUTCOME_DENY,
        Some(detail),
    )
    .await
    {
        tracing::error!(%write_error, action = %audit_action(method, path), "AUDIT WRITE FAILED: denial went unrecorded");
        state.metrics.record_audit_failure();
    }
    error
}

pub async fn require_admin(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> AppResult<Response> {
    let path = request.uri().path().to_string();

    if path == LOGIN_PATH {
        return Ok(next.run(request).await);
    }

    verify_csrf(&state, request.method(), request.headers(), &path).await?;

    let Some(token) = session_token(request.headers()) else {
        return Err(deny(
            &state,
            audit::Actor::Anonymous(UNAUTHENTICATED_LABEL),
            request.method(),
            &path,
            missing_credentials(),
            "unauthenticated",
        )
        .await);
    };

    let Some(principal) = resolve_session_principal(&state, token.as_str()).await else {
        return Err(deny(
            &state,
            audit::Actor::Anonymous(UNAUTHENTICATED_LABEL),
            request.method(),
            &path,
            AppError::Unauthorized("invalid or expired session token".to_string()),
            "unauthenticated",
        )
        .await);
    };

    let method = request.method().clone();
    if principal.must_change_password
        && !is_password_change_request(&method, &path)
        && !is_logout_request(&method, &path)
        && !is_session_route(&path)
    {
        let error = deny(
            &state,
            audit::Actor::Principal(&principal),
            &method,
            &path,
            AppError::Forbidden("password change required".to_string()),
            "must_change_password",
        )
        .await;
        return Err(error);
    }

    if let Some(capability) = required_capability(&method, &path) {
        if !principal.can(capability) {
            let error = deny(
                &state,
                audit::Actor::Principal(&principal),
                &method,
                &path,
                AppError::Forbidden(format!(
                    "role '{}' may not {}",
                    principal.role.as_str(),
                    capability.as_str()
                )),
                capability.as_str(),
            )
            .await;
            return Err(error);
        }
    }

    if is_mutating(&method) && !is_session_route(&path) {
        if let Err(write_error) = audit::record(
            &state.db,
            audit::Actor::Principal(&principal),
            &audit_action(&method, &path),
            Some(&path),
            audit::OUTCOME_ALLOW,
            None,
        )
        .await
        {
            tracing::error!(%write_error, path = %path, "AUDIT WRITE FAILED: refusing unrecorded mutation");
            state.metrics.record_audit_failure();
            return Err(AppError::Internal("audit log unavailable".to_string()));
        }
    }

    request.extensions_mut().insert(principal);
    Ok(next.run(request).await)
}

fn missing_credentials() -> AppError {
    AppError::Unauthorized("missing session cookie; sign in again".to_string())
}

fn is_password_change_request(method: &Method, path: &str) -> bool {
    method == Method::POST && path == "/admin/api/users/me/password"
}

fn is_logout_request(method: &Method, path: &str) -> bool {
    method == Method::DELETE && path == "/admin/api/session"
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderName;

    use crate::admin::csrf::{csrf_violation, is_https, session_token};

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                HeaderName::from_bytes(name.as_bytes()).expect("header name"),
                value.parse().expect("header value"),
            );
        }
        headers
    }

    #[test]
    fn reads_the_session_cookie_among_others() {
        let map = headers(&[("cookie", "theme=dark; fg_session=fgs_abc; locale=en")]);
        assert_eq!(session_token(&map).as_deref(), Some("fgs_abc"));
    }

    #[test]
    fn ignores_missing_and_empty_session_cookies() {
        assert_eq!(session_token(&headers(&[])), None);
        assert_eq!(session_token(&headers(&[("cookie", "theme=dark")])), None);
        assert_eq!(session_token(&headers(&[("cookie", "fg_session=")])), None);
    }

    #[test]
    fn authorization_bearer_is_no_longer_accepted() {
        let map = headers(&[("authorization", "Bearer fgs_abc")]);
        assert_eq!(session_token(&map), None);
    }

    #[test]
    fn session_cookie_is_http_only_lax_and_scoped() {
        let cookie = session_cookie("fgs_abc", false, cookie::time::Duration::seconds(60));
        assert!(cookie.starts_with("fg_session=fgs_abc"), "{cookie}");
        assert!(cookie.contains("HttpOnly"), "{cookie}");
        assert!(cookie.contains("SameSite=Lax"), "{cookie}");
        assert!(cookie.contains("Path=/"), "{cookie}");
        assert!(cookie.contains("Max-Age=60"), "{cookie}");
        assert!(!cookie.contains("Secure"), "{cookie}");
    }

    #[test]
    fn session_cookie_is_secure_over_https() {
        let cookie = session_cookie("fgs_abc", true, session_max_age());
        assert!(cookie.contains("Secure"), "{cookie}");
    }

    #[test]
    fn forwarded_proto_needs_a_trusted_peer() {
        use std::net::SocketAddr;

        let trusted = vec!["10.0.0.0/8".parse().unwrap()];
        let peer = |addr: &str| Some(SocketAddr::new(addr.parse().unwrap(), 4123));

        let https = headers(&[("x-forwarded-proto", "https")]);
        assert!(is_https(&https, peer("10.0.0.5"), &trusted));
        assert!(!is_https(&https, peer("203.0.113.9"), &trusted));
        assert!(!is_https(&https, None, &trusted));
        assert!(!is_https(&headers(&[]), peer("10.0.0.5"), &trusted));
    }

    #[test]
    fn safe_methods_need_no_csrf_proof() {
        for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert_eq!(csrf_violation(&method, &headers(&[])), None, "{method}");
        }
    }

    #[test]
    fn cross_site_state_changes_are_refused() {
        for site in ["cross-site", "same-site"] {
            let map = headers(&[("sec-fetch-site", site)]);
            assert_eq!(
                csrf_violation(&Method::POST, &map),
                Some("cross-site request rejected"),
                "{site}"
            );
        }
    }

    #[test]
    fn same_origin_state_changes_pass() {
        for site in ["same-origin", "none"] {
            let map = headers(&[("sec-fetch-site", site)]);
            assert_eq!(csrf_violation(&Method::POST, &map), None, "{site}");
        }
    }

    #[test]
    fn navigation_requests_are_refused() {
        let map = headers(&[
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-mode", "navigate"),
        ]);
        assert_eq!(
            csrf_violation(&Method::POST, &map),
            Some("cross-site request rejected")
        );
    }

    #[test]
    fn origin_fallback_requires_a_matching_host() {
        let matching = headers(&[
            ("origin", "http://localhost:8080"),
            ("host", "localhost:8080"),
        ]);
        assert_eq!(csrf_violation(&Method::DELETE, &matching), None);

        let mismatched = headers(&[
            ("origin", "http://evil.example"),
            ("host", "localhost:8080"),
        ]);
        assert_eq!(
            csrf_violation(&Method::DELETE, &mismatched),
            Some("missing same-origin proof")
        );

        let bare = headers(&[("origin", "http://localhost:8080")]);
        assert_eq!(
            csrf_violation(&Method::DELETE, &bare),
            Some("missing same-origin proof")
        );

        let opaque = headers(&[("origin", "null"), ("host", "localhost:8080")]);
        assert_eq!(
            csrf_violation(&Method::DELETE, &opaque),
            Some("missing same-origin proof")
        );
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod route_tests;
