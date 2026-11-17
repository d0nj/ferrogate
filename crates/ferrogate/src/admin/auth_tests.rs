use axum::{
    body::Body,
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
};

use super::{require_admin, LOGIN_PATH};
use crate::admin::policy::required_capability;
use crate::auth::role::{self, Capability, Role};
use crate::auth::session::Principal;
use crate::AppState;

fn ensure_test_key() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::env::set_var(
            "FERROGATE_ENCRYPTION_KEY",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        );
    });
}

fn cap(method: Method, path: &str) -> Option<Capability> {
    required_capability(&method, path)
}

#[test]
fn overview_routes_read_overview() {
    assert_eq!(
        cap(Method::GET, "/admin/api/overview"),
        Some(Capability::ReadOverview)
    );
    assert_eq!(
        cap(Method::GET, "/admin/api/overview/series"),
        Some(Capability::ReadOverview)
    );
    assert_eq!(
        cap(Method::GET, "/admin/api/overview/providers/health"),
        Some(Capability::ReadOverview)
    );
}

#[test]
fn log_routes_read_logs() {
    assert_eq!(
        cap(Method::GET, "/admin/api/logs"),
        Some(Capability::ReadLogs)
    );
    assert_eq!(
        cap(Method::GET, "/admin/api/logs/req-1"),
        Some(Capability::ReadLogs)
    );
    assert_eq!(
        cap(Method::GET, "/admin/api/audit"),
        Some(Capability::ReadLogs)
    );
}

#[test]
fn config_reads_require_read_config() {
    assert_eq!(
        cap(Method::GET, "/admin/api/response-cache/stats"),
        Some(Capability::ReadConfig)
    );
    assert_eq!(
        cap(Method::GET, "/admin/api/settings"),
        Some(Capability::ReadConfig)
    );
    assert_eq!(
        cap(Method::GET, "/admin/api/pricing"),
        Some(Capability::ReadConfig)
    );
}

#[test]
fn key_routes_manage_keys() {
    assert_eq!(
        cap(Method::GET, "/admin/api/keys"),
        Some(Capability::ManageKeys)
    );
    assert_eq!(
        cap(Method::POST, "/admin/api/keys"),
        Some(Capability::ManageKeys)
    );
    assert_eq!(
        cap(Method::PATCH, "/admin/api/keys/key-1"),
        Some(Capability::ManageKeys)
    );
    assert_eq!(
        cap(Method::DELETE, "/admin/api/keys/key-1"),
        Some(Capability::ManageKeys)
    );
    assert_eq!(
        cap(Method::GET, "/admin/api/teams"),
        Some(Capability::ManageKeys)
    );
}

#[test]
fn provider_routes_manage_providers() {
    for (method, path) in [
        (Method::GET, "/admin/api/providers"),
        (Method::POST, "/admin/api/providers"),
        (Method::PUT, "/admin/api/providers/prov-1"),
        (Method::DELETE, "/admin/api/providers/prov-1"),
        (Method::GET, "/admin/api/providers/prov-1/keys"),
        (Method::POST, "/admin/api/providers/prov-1/keys"),
        (Method::POST, "/admin/api/providers/prov-1/test"),
        (Method::POST, "/admin/api/providers/prov-1/sync-models"),
        (Method::PATCH, "/admin/api/providers/prov-1/keys/pk-1"),
        (Method::DELETE, "/admin/api/providers/prov-1/keys/pk-1"),
    ] {
        assert_eq!(
            cap(method.clone(), path),
            Some(Capability::ManageProviders),
            "{method} {path}"
        );
    }
}

#[test]
fn routing_routes_manage_routing() {
    for (method, path) in [
        (Method::GET, "/admin/api/routing"),
        (Method::POST, "/admin/api/routing"),
        (Method::PUT, "/admin/api/routing/rule-1"),
        (Method::DELETE, "/admin/api/routing/rule-1"),
    ] {
        assert_eq!(
            cap(method.clone(), path),
            Some(Capability::ManageRouting),
            "{method} {path}"
        );
    }
}

#[test]
fn resource_prefixed_routes_map_to_their_capability() {
    for (method, path, expected) in [
        (
            Method::GET,
            "/admin/api/guardrails",
            Capability::ManageGuardrails,
        ),
        (
            Method::POST,
            "/admin/api/guardrails",
            Capability::ManageGuardrails,
        ),
        (
            Method::PUT,
            "/admin/api/guardrails/g-1",
            Capability::ManageGuardrails,
        ),
        (
            Method::DELETE,
            "/admin/api/guardrails/g-1",
            Capability::ManageGuardrails,
        ),
        (
            Method::GET,
            "/admin/api/webhooks",
            Capability::ManageWebhooks,
        ),
        (
            Method::POST,
            "/admin/api/webhooks",
            Capability::ManageWebhooks,
        ),
        (
            Method::PUT,
            "/admin/api/webhooks/w-1",
            Capability::ManageWebhooks,
        ),
        (
            Method::DELETE,
            "/admin/api/webhooks/w-1",
            Capability::ManageWebhooks,
        ),
        (
            Method::POST,
            "/admin/api/webhooks/w-1/test",
            Capability::ManageWebhooks,
        ),
        (Method::GET, "/admin/api/prompts", Capability::ManagePrompts),
        (
            Method::POST,
            "/admin/api/prompts",
            Capability::ManagePrompts,
        ),
        (
            Method::PUT,
            "/admin/api/prompts/p-1",
            Capability::ManagePrompts,
        ),
        (
            Method::DELETE,
            "/admin/api/prompts/p-1",
            Capability::ManagePrompts,
        ),
        (Method::GET, "/admin/api/mcp/servers", Capability::ManageMcp),
        (
            Method::POST,
            "/admin/api/mcp/servers",
            Capability::ManageMcp,
        ),
        (
            Method::PUT,
            "/admin/api/mcp/servers/m-1",
            Capability::ManageMcp,
        ),
        (
            Method::DELETE,
            "/admin/api/mcp/servers/m-1",
            Capability::ManageMcp,
        ),
        (Method::GET, "/admin/api/mcp/tools", Capability::ManageMcp),
        (
            Method::GET,
            "/admin/api/semantic/stats",
            Capability::ClearCache,
        ),
        (
            Method::PUT,
            "/admin/api/semantic/threshold",
            Capability::ClearCache,
        ),
        (
            Method::PUT,
            "/admin/api/semantic/embedding-model",
            Capability::ClearCache,
        ),
        (
            Method::DELETE,
            "/admin/api/semantic-cache",
            Capability::ClearCache,
        ),
        (Method::GET, "/admin/api/users", Capability::ManageUsers),
        (Method::GET, "/admin/api/users/me", Capability::ReadConfig),
        (
            Method::POST,
            "/admin/api/users/me/password",
            Capability::ReadConfig,
        ),
        (Method::POST, "/admin/api/users", Capability::ManageUsers),
        (
            Method::PATCH,
            "/admin/api/users/u-1",
            Capability::ManageUsers,
        ),
        (
            Method::DELETE,
            "/admin/api/users/u-1",
            Capability::ManageUsers,
        ),
    ] {
        assert_eq!(cap(method.clone(), path), Some(expected), "{method} {path}");
    }
}

#[test]
fn write_routes_map_to_their_write_capability() {
    assert_eq!(
        cap(Method::PUT, "/admin/api/pricing/gpt-4o"),
        Some(Capability::ManagePricing)
    );
    assert_eq!(
        cap(Method::PUT, "/admin/api/settings"),
        Some(Capability::ChangeSettings)
    );
    assert_eq!(
        cap(Method::DELETE, "/admin/api/response-cache"),
        Some(Capability::ClearCache)
    );
}

#[test]
fn session_routes_need_read_config() {
    for (method, path) in [
        (Method::GET, "/admin/api/session"),
        (Method::DELETE, "/admin/api/session"),
    ] {
        assert_eq!(
            cap(method.clone(), path),
            Some(Capability::ReadConfig),
            "{method} {path}"
        );
    }
}

#[test]
fn unknown_admin_paths_default_to_owner_only() {
    for path in [
        "/admin/api/mystery",
        "/admin/api/mystery/deep/path",
        "/admin/api/session/extra",
    ] {
        assert_eq!(
            cap(Method::GET, path),
            Some(Capability::ManageUsers),
            "{path}"
        );
    }
    for path in [
        "/admin/api",
        "/admin/api/",
        "/health",
        "/v1/chat/completions",
    ] {
        assert_eq!(cap(Method::GET, path), None, "{path}");
    }

    assert_eq!(cap(Method::POST, LOGIN_PATH), None);
}

#[test]
fn every_admin_route_has_an_explicit_capability() {
    use Capability::*;
    let routes = [
        (Method::GET, "/admin/api/overview", ReadOverview),
        (Method::GET, "/admin/api/logs", ReadLogs),
        (Method::DELETE, "/admin/api/logs/req-1", ClearLogs),
        (Method::GET, "/admin/api/audit", ReadLogs),
        (Method::GET, "/admin/api/session", ReadConfig),
        (Method::DELETE, "/admin/api/session", ReadConfig),
        (Method::GET, "/admin/api/response-cache/stats", ReadConfig),
        (Method::DELETE, "/admin/api/response-cache", ClearCache),
        (Method::GET, "/admin/api/keys", ManageKeys),
        (Method::POST, "/admin/api/keys", ManageKeys),
        (Method::GET, "/admin/api/teams", ManageKeys),
        (Method::PATCH, "/admin/api/keys/key-1", ManageKeys),
        (Method::DELETE, "/admin/api/keys/key-1", ManageKeys),
        (Method::GET, "/admin/api/providers", ManageProviders),
        (Method::POST, "/admin/api/providers", ManageProviders),
        (
            Method::PATCH,
            "/admin/api/providers/prov-1",
            ManageProviders,
        ),
        (
            Method::DELETE,
            "/admin/api/providers/prov-1",
            ManageProviders,
        ),
        (
            Method::POST,
            "/admin/api/providers/prov-1/test",
            ManageProviders,
        ),
        (
            Method::POST,
            "/admin/api/providers/prov-1/sync-models",
            ManageProviders,
        ),
        (
            Method::GET,
            "/admin/api/providers/prov-1/keys",
            ManageProviders,
        ),
        (
            Method::POST,
            "/admin/api/providers/prov-1/keys",
            ManageProviders,
        ),
        (
            Method::PATCH,
            "/admin/api/providers/prov-1/keys/pk-1",
            ManageProviders,
        ),
        (
            Method::DELETE,
            "/admin/api/providers/prov-1/keys/pk-1",
            ManageProviders,
        ),
        (Method::GET, "/admin/api/routing/policy", ManageRouting),
        (Method::PUT, "/admin/api/routing/policy", ManageRouting),
        (Method::GET, "/admin/api/routes", ManageRouting),
        (Method::GET, "/admin/api/routes/default", ManageRouting),
        (Method::PUT, "/admin/api/routes/default", ManageRouting),
        (Method::DELETE, "/admin/api/routes/default", ManageRouting),
        (Method::GET, "/admin/api/guardrails", ManageGuardrails),
        (Method::GET, "/admin/api/webhooks", ManageWebhooks),
        (Method::GET, "/admin/api/prompts", ManagePrompts),
        (Method::GET, "/admin/api/mcp/servers", ManageMcp),
        (Method::GET, "/admin/api/mcp/tools", ManageMcp),
        (Method::GET, "/admin/api/semantic/stats", ClearCache),
        (Method::DELETE, "/admin/api/semantic-cache", ClearCache),
        (Method::GET, "/admin/api/pricing", ReadConfig),
        (Method::PUT, "/admin/api/pricing/gpt-4o", ManagePricing),
        (Method::GET, "/admin/api/settings", ReadConfig),
        (Method::PUT, "/admin/api/settings", ChangeSettings),
        (Method::GET, "/admin/api/users", ManageUsers),
        (Method::GET, "/admin/api/users/me", ReadConfig),
        (Method::POST, "/admin/api/users/me/password", ReadConfig),
        (Method::PATCH, "/admin/api/users/u-1", ManageUsers),
        (Method::DELETE, "/admin/api/users/u-1", ManageUsers),
        (Method::POST, "/admin/api/users/u-1/password", ManageUsers),
        (Method::POST, "/admin/api/users", ManageUsers),
    ];
    for (method, path, expected) in routes {
        assert_eq!(cap(method.clone(), path), Some(expected), "{method} {path}");
    }
}

fn mapped_capabilities() -> Vec<Capability> {
    let probes = [
        (Method::GET, "/admin/api/overview"),
        (Method::GET, "/admin/api/logs"),
        (Method::DELETE, "/admin/api/logs/req-1"),
        (Method::GET, "/admin/api/settings"),
        (Method::GET, "/admin/api/keys"),
        (Method::GET, "/admin/api/providers"),
        (Method::GET, "/admin/api/routing"),
        (Method::GET, "/admin/api/guardrails"),
        (Method::GET, "/admin/api/webhooks"),
        (Method::GET, "/admin/api/prompts"),
        (Method::GET, "/admin/api/mcp"),
        (Method::PUT, "/admin/api/pricing/model"),
        (Method::PUT, "/admin/api/settings"),
        (Method::DELETE, "/admin/api/response-cache"),
        (Method::GET, "/admin/api/users"),
    ];

    let mut found: Vec<Capability> = probes
        .iter()
        .filter_map(|(method, path)| cap(method.clone(), path))
        .collect();
    found.sort_by_key(|capability| capability.as_str());
    found.dedup();
    found
}

#[test]
fn every_capability_is_reachable_through_the_map() {
    let mut all = vec![
        Capability::ReadOverview,
        Capability::ReadLogs,
        Capability::ReadConfig,
        Capability::ManageProviders,
        Capability::ManageRouting,
        Capability::ManageKeys,
        Capability::ManageGuardrails,
        Capability::ManageWebhooks,
        Capability::ManagePrompts,
        Capability::ManageMcp,
        Capability::ManagePricing,
        Capability::ManageUsers,
        Capability::ClearCache,
        Capability::ClearLogs,
        Capability::ChangeSettings,
    ];
    all.sort_by_key(|capability| capability.as_str());

    assert_eq!(mapped_capabilities(), all);
}

#[test]
fn principal_agrees_with_role_table() {
    for role in Role::ALL {
        let principal = Principal {
            user_id: format!("user-{}", role.as_str()),
            email: format!("{}@example.com", role.as_str()),
            role,
            must_change_password: false,
        };

        for capability in mapped_capabilities() {
            assert_eq!(
                principal.can(capability),
                role::grants(role, capability),
                "{} / {}",
                role.as_str(),
                capability.as_str()
            );
        }
    }
}

async fn memory_db() -> sea_orm::DatabaseConnection {
    ensure_test_key();
    use sea_orm::{ConnectOptions, Database};
    use sea_orm_migration::MigratorTrait;

    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1);
    let db = Database::connect(options).await.expect("in-memory sqlite");
    crate::migration::Migrator::up(&db, None)
        .await
        .expect("migrate");
    db
}

async fn state() -> AppState {
    let db = memory_db().await;
    let (state, _worker) = AppState::new(db, crate::config::AppConfig::default());
    state
}

struct Guarded {
    status: StatusCode,
    principal_seen: bool,
}

async fn guard(state: &AppState, method: Method, path: &str, headers: &[(&str, &str)]) -> Guarded {
    use axum::{middleware, response::IntoResponse, routing::any, Router};
    use tower::util::ServiceExt;

    async fn probe(request: Request) -> impl IntoResponse {
        (
            StatusCode::OK,
            if request.extensions().get::<Principal>().is_some() {
                "principal"
            } else {
                "anonymous"
            },
        )
    }

    let app: Router = Router::new()
        .route("/admin/api", any(probe))
        .route("/admin/api/{*rest}", any(probe))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_admin))
        .with_state(state.clone());

    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .expect("request");
    for (name, value) in headers {
        request.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes()).expect("header name"),
            HeaderValue::from_str(value).expect("header value"),
        );
    }

    let response = app.oneshot(request).await.expect("router is infallible");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");

    Guarded {
        status,
        principal_seen: body.as_ref() == b"principal",
    }
}

fn cookie(token: &str) -> String {
    format!("{}={token}", super::SESSION_COOKIE)
}

#[tokio::test]
async fn missing_session_token_is_rejected() {
    let state = state().await;

    let guarded = guard(&state, Method::GET, "/admin/api/overview", &[]).await;
    assert_eq!(guarded.status, StatusCode::UNAUTHORIZED);
    assert!(!guarded.principal_seen);
}

#[tokio::test]
async fn unknown_session_token_is_rejected() {
    let state = state().await;

    let guarded = guard(
        &state,
        Method::GET,
        "/admin/api/overview",
        &[("cookie", &cookie("fgs_deadbeef"))],
    )
    .await;
    assert_eq!(guarded.status, StatusCode::UNAUTHORIZED);
    assert!(!guarded.principal_seen);
}

#[tokio::test]
async fn unknown_admin_paths_deny_non_owners() {
    let state = state().await;
    seed_user(&state, "viewer", "viewer@example.com", Role::Viewer).await;
    let session = state
        .sessions()
        .mint("viewer".into(), "viewer@example.com".into(), Role::Viewer)
        .await;

    let guarded = guard(
        &state,
        Method::GET,
        "/admin/api/mystery",
        &[("cookie", &cookie(&session.token))],
    )
    .await;
    assert_eq!(guarded.status, StatusCode::FORBIDDEN);
    assert!(!guarded.principal_seen);
}

#[tokio::test]
async fn bearer_header_no_longer_authenticates() {
    let state = state().await;
    let session = state
        .sessions()
        .mint("owner".into(), "owner@example.com".into(), Role::Owner)
        .await;
    seed_user(&state, "owner", "owner@example.com", Role::Owner).await;

    let guarded = guard(
        &state,
        Method::GET,
        "/admin/api/overview",
        &[("authorization", &format!("Bearer {}", session.token))],
    )
    .await;
    assert_eq!(guarded.status, StatusCode::UNAUTHORIZED);
    assert!(!guarded.principal_seen);
}

#[tokio::test]
async fn session_cookie_authenticates() {
    let state = state().await;
    seed_user(&state, "owner", "owner@example.com", Role::Owner).await;
    let session = state
        .sessions()
        .mint("owner".into(), "owner@example.com".into(), Role::Owner)
        .await;

    let guarded = guard(
        &state,
        Method::GET,
        "/admin/api/overview",
        &[("cookie", &cookie(&session.token))],
    )
    .await;
    assert_eq!(guarded.status, StatusCode::OK);
    assert!(guarded.principal_seen);
}

#[tokio::test]
async fn session_of_a_missing_user_is_revoked() {
    let state = state().await;
    let session = state
        .sessions()
        .mint("ghost".into(), "ghost@example.com".into(), Role::Owner)
        .await;

    let guarded = guard(
        &state,
        Method::GET,
        "/admin/api/overview",
        &[("cookie", &cookie(&session.token))],
    )
    .await;
    assert_eq!(guarded.status, StatusCode::UNAUTHORIZED);
    assert!(!guarded.principal_seen);
    assert!(state.sessions().get(&session.token).await.is_none());
}

#[tokio::test]
async fn login_path_is_exempt_and_gets_no_principal() {
    let state = state().await;

    let guarded = guard(
        &state,
        Method::POST,
        LOGIN_PATH,
        &[("sec-fetch-site", "same-origin")],
    )
    .await;
    assert_eq!(guarded.status, StatusCode::OK);
    assert!(
        !guarded.principal_seen,
        "the login request has no principal yet"
    );
}

#[tokio::test]
async fn cross_site_state_change_is_refused() {
    let state = state().await;
    seed_user(&state, "owner", "owner@example.com", Role::Owner).await;
    let session = state
        .sessions()
        .mint("owner".into(), "owner@example.com".into(), Role::Owner)
        .await;

    let guarded = guard(
        &state,
        Method::POST,
        "/admin/api/providers",
        &[
            ("cookie", &cookie(&session.token)),
            ("sec-fetch-site", "cross-site"),
        ],
    )
    .await;
    assert_eq!(guarded.status, StatusCode::FORBIDDEN);
    assert!(!guarded.principal_seen);
}

#[tokio::test]
async fn state_change_without_origin_proof_is_refused() {
    let state = state().await;
    seed_user(&state, "owner", "owner@example.com", Role::Owner).await;
    let session = state
        .sessions()
        .mint("owner".into(), "owner@example.com".into(), Role::Owner)
        .await;

    let guarded = guard(
        &state,
        Method::DELETE,
        "/admin/api/providers/p1",
        &[("cookie", &cookie(&session.token))],
    )
    .await;
    assert_eq!(guarded.status, StatusCode::FORBIDDEN);
    assert!(!guarded.principal_seen);
}

#[tokio::test]
async fn same_origin_state_change_succeeds() {
    let state = state().await;
    seed_user(&state, "owner", "owner@example.com", Role::Owner).await;
    let session = state
        .sessions()
        .mint("owner".into(), "owner@example.com".into(), Role::Owner)
        .await;

    let guarded = guard(
        &state,
        Method::POST,
        "/admin/api/providers",
        &[
            ("cookie", &cookie(&session.token)),
            ("sec-fetch-site", "same-origin"),
        ],
    )
    .await;
    assert_eq!(guarded.status, StatusCode::OK);
    assert!(guarded.principal_seen);
}

#[tokio::test]
async fn origin_header_fallback_authenticates_a_same_host_request() {
    let state = state().await;
    seed_user(&state, "owner", "owner@example.com", Role::Owner).await;
    let session = state
        .sessions()
        .mint("owner".into(), "owner@example.com".into(), Role::Owner)
        .await;

    let guarded = guard(
        &state,
        Method::POST,
        "/admin/api/providers",
        &[
            ("cookie", &cookie(&session.token)),
            ("origin", "http://localhost:8080"),
            ("host", "localhost:8080"),
        ],
    )
    .await;
    assert_eq!(guarded.status, StatusCode::OK);
    assert!(guarded.principal_seen);
}

async fn seed_user(state: &AppState, id: &str, email: &str, role: Role) {
    use crate::entities::user;
    use sea_orm::{ActiveModelTrait, Set};

    user::ActiveModel {
        id: Set(id.to_string()),
        email: Set(email.to_string()),
        name: Set(id.to_string()),
        password_hash: Set(
            crate::auth::password::hash_password("seeded-Password-9").expect("hash"),
        ),
        role: Set(role.as_str().to_string()),
        is_active: Set(1),
        must_change_password: Set(0),
        created_at: Set("2026-01-01T00:00:00.000Z".to_string()),
        last_login_at: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("seed user");
}

async fn call(
    state: &AppState,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<serde_json::Value>,
) -> (StatusCode, HeaderMap, serde_json::Value) {
    use axum::Router;
    use tower::util::ServiceExt;

    let app: Router = crate::build_app(state.clone());
    let payload = body
        .map(|value| Body::from(value.to_string()))
        .unwrap_or_else(Body::empty);

    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(payload)
        .expect("request");
    for (name, value) in headers {
        request.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes()).expect("header name"),
            HeaderValue::from_str(value).expect("header value"),
        );
    }

    let response = app.oneshot(request).await.expect("router is infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, headers, json)
}

fn session_cookie_header(headers: &HeaderMap) -> String {
    headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(super::SESSION_COOKIE))
        .map(str::to_string)
        .unwrap_or_default()
}

#[tokio::test]
async fn login_sets_an_http_only_cookie_and_no_token_in_the_body() {
    let state = state().await;
    seed_user(&state, "owner", "owner@example.com", Role::Owner).await;

    let (status, headers, body) = call(
        &state,
        Method::POST,
        LOGIN_PATH,
        &[("sec-fetch-site", "same-origin")],
        Some(serde_json::json!({
            "email": "owner@example.com",
            "password": "seeded-Password-9",
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["expires_in"], super::SESSION_TTL_SECS);
    assert!(body.get("token").is_none(), "token must not be in the body");

    let cookie = session_cookie_header(&headers);
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("SameSite=Lax"), "{cookie}");
    assert!(cookie.contains("Path=/"), "{cookie}");
    assert!(
        cookie.contains(&format!("Max-Age={}", super::SESSION_TTL_SECS)),
        "{cookie}"
    );
    assert!(
        !cookie.contains("Secure"),
        "plain-HTTP login stays usable: {cookie}"
    );
}

#[tokio::test]
async fn logout_clears_the_cookie_and_revokes_the_token() {
    let state = state().await;
    seed_user(&state, "owner", "owner@example.com", Role::Owner).await;

    let (_, headers, _) = call(
        &state,
        Method::POST,
        LOGIN_PATH,
        &[("sec-fetch-site", "same-origin")],
        Some(serde_json::json!({
            "email": "owner@example.com",
            "password": "seeded-Password-9",
        })),
    )
    .await;
    let cookie = session_cookie_header(&headers);
    let token = cookie
        .split(';')
        .next()
        .and_then(|pair| pair.split_once('='))
        .map(|(_, value)| value.to_string())
        .expect("token in cookie");
    assert!(state.sessions().get(&token).await.is_some());

    let (status, headers, body) = call(
        &state,
        Method::DELETE,
        "/admin/api/session",
        &[
            ("cookie", &format!("{}={token}", super::SESSION_COOKIE)),
            ("sec-fetch-site", "same-origin"),
        ],
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["logged_out"], true);
    let cleared = session_cookie_header(&headers);
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    assert!(state.sessions().get(&token).await.is_none());
}

#[tokio::test]
async fn session_endpoint_returns_the_principal() {
    let state = state().await;
    seed_user(&state, "owner", "owner@example.com", Role::Owner).await;
    let session = state
        .sessions()
        .mint("owner".into(), "owner@example.com".into(), Role::Owner)
        .await;

    let (status, _, body) = call(
        &state,
        Method::GET,
        "/admin/api/session",
        &[("cookie", &cookie(&session.token))],
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["id"], "owner");
    assert_eq!(body["email"], "owner@example.com");
    assert_eq!(body["role"], "owner");
}

#[tokio::test]
async fn session_endpoint_requires_a_cookie() {
    let state = state().await;

    let (status, _, _) = call(&state, Method::GET, "/admin/api/session", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn must_change_password_blocks_admin_but_allows_rotation_and_logout() {
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let state = state().await;
    seed_user(&state, "owner", "owner@example.com", Role::Owner).await;
    crate::entities::user::Entity::update_many()
        .col_expr(
            crate::entities::user::Column::MustChangePassword,
            Expr::value(1),
        )
        .filter(crate::entities::user::Column::Id.eq("owner"))
        .exec(&state.db)
        .await
        .expect("flag rotation");
    let session = state
        .sessions()
        .mint("owner".into(), "owner@example.com".into(), Role::Owner)
        .await;
    let cookie_value = cookie(&session.token);
    let headers: Vec<(&str, &str)> = vec![
        ("cookie", cookie_value.as_str()),
        ("sec-fetch-site", "same-origin"),
    ];

    let guarded = guard(&state, Method::GET, "/admin/api/overview", &headers).await;
    assert_eq!(guarded.status, StatusCode::FORBIDDEN);
    assert!(!guarded.principal_seen);

    let guarded = guard(
        &state,
        Method::POST,
        "/admin/api/users/me/password",
        &headers,
    )
    .await;
    assert_eq!(guarded.status, StatusCode::OK);
    assert!(guarded.principal_seen);

    let guarded = guard(&state, Method::DELETE, "/admin/api/session", &headers).await;
    assert_eq!(guarded.status, StatusCode::OK);
    assert!(guarded.principal_seen);
}
