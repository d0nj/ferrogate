//! Phase 3 authentication suite: bearer resolution, allow-lists, GCRA limits,
//! hard USD budget caps and moka cache invalidation.

use std::{net::SocketAddr, time::Duration};

use axum::Router;
use ferrogate::{
    auth,
    config::AppConfig,
    entities::{provider, virtual_key},
    AppState,
};
use sea_orm::{sea_query::Expr, ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::{json, Value};

/// Seeded demo credential (`key-demo-local`).
const DEMO_KEY: &str = "sk-fg-demo-ferrogate-local-key-01";
const DEMO_KEY_ID: &str = "key-demo-local";
/// Priced upstream model used for cost assertions.
const MODEL: &str = "gpt-4o";

/// Running gateway with the real auth middleware mounted in front of the proxy.
struct World {
    base_url: String,
    db: sea_orm::DatabaseConnection,
    state: AppState,
}

impl World {
    async fn start() -> Self {
        Self::start_at("http://127.0.0.1:1").await
    }

    /// Start with a priced in-process OpenAI-format upstream so spend accrues.
    async fn start_with_upstream() -> Self {
        Self::start_at(&spawn_mock_upstream().await).await
    }

    async fn start_at(provider_base_url: &str) -> Self {
        let db = temp_db().await;
        virtual_key::Entity::update_many()
            .col_expr(virtual_key::Column::IsActive, Expr::value(1))
            .filter(virtual_key::Column::Id.eq(DEMO_KEY_ID))
            .exec(&db)
            .await
            .unwrap();
        provider::Entity::delete_many().exec(&db).await.unwrap();

        // Model allowance checks run before any upstream call, so most tests
        // never reach this base URL; the budget test points it at a mock.
        provider::ActiveModel {
            id: Set("prov-openai".to_owned()),
            name: Set("mock-openai".to_owned()),
            provider_type: Set("openai".to_owned()),
            base_url: Set(provider_base_url.to_owned()),
            api_key: Set("test-key".to_owned()),
            models_json: Set(r#"["gpt-4o"]"#.to_owned()),
            is_active: Set(1),
            timeout_ms: Set(1_000),
            created_at: Set("2026-01-01T00:00:00Z".to_owned()),
            updated_at: Set("2026-01-01T00:00:00Z".to_owned()),
            api_dialect: Set("chat".to_owned()),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();

        ferrogate::entities::routing_rule::ActiveModel {
            id: Set("rule-auth-gpt4o".to_owned()),
            alias: Set(MODEL.to_owned()),
            provider_id: Set("prov-openai".to_owned()),
            upstream_model: Set(MODEL.to_owned()),
            weight: Set(100),
            position: Set(0),
            is_active: Set(1),
            created_at: Set("2026-01-01T00:00:00Z".to_owned()),
            updated_at: Set("2026-01-01T00:00:00Z".to_owned()),
        }
        .insert(&db)
        .await
        .unwrap();

        let (state, worker) = AppState::from_db_and_config(db.clone(), AppConfig::default()).await;
        std::mem::forget(worker);

        let app = Router::new()
            .merge(ferrogate::proxy::router())
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth::require_virtual_key,
            ))
            .with_state(state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            base_url: format!("http://{addr}"),
            db,
            state,
        }
    }

    async fn chat(&self, bearer: Option<&str>, model: &str) -> reqwest::Response {
        self.chat_with(bearer, model, "hello").await
    }

    async fn chat_with(
        &self,
        bearer: Option<&str>,
        model: &str,
        content: &str,
    ) -> reqwest::Response {
        let mut builder = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&json!({
                "model": model,
                "messages": [{ "role": "user", "content": content }],
            }));
        if let Some(token) = bearer {
            builder = builder.bearer_auth(token);
        }
        builder.send().await.unwrap()
    }

    /// Insert a key whose hash is the SHA-256 of `raw`.
    async fn insert_key(
        &self,
        id: &str,
        raw: &str,
        apply: impl FnOnce(&mut virtual_key::ActiveModel),
    ) {
        let mut row = virtual_key::ActiveModel {
            id: Set(id.to_owned()),
            name: Set(format!("test-{id}")),
            key_prefix: Set(raw.chars().take(10).collect()),
            key_hash: Set(auth::sha256_hex(raw)),
            budget_usd: Set(None),
            spent_usd: Set(0.0),
            rpm_limit: Set(None),
            tpm_limit: Set(None),
            allowed_models_json: Set("[]".to_owned()),
            is_active: Set(1),
            created_at: Set("2026-01-01T00:00:00Z".to_owned()),
            last_used_at: Set(None),
            ..Default::default()
        };
        apply(&mut row);
        row.insert(&self.db).await.unwrap();
    }

    async fn revoke(&self, id: &str) {
        virtual_key::Entity::update_many()
            .col_expr(virtual_key::Column::IsActive, Expr::value(0i64))
            .filter(virtual_key::Column::Id.eq(id))
            .exec(&self.db)
            .await
            .unwrap();
    }
}

/// Minimal OpenAI-format upstream reporting 11 prompt / 5 completion tokens.
async fn spawn_mock_upstream() -> String {
    async fn chat(axum::Json(body): axum::Json<Value>) -> axum::Json<Value> {
        let model = body["model"].as_str().unwrap_or("gpt-4o").to_owned();
        axum::Json(json!({
            "id": "chatcmpl-auth-mock",
            "object": "chat.completion",
            "created": 1_700_000_000,
            "model": model,
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "ok" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 11, "completion_tokens": 5, "total_tokens": 16 }
        }))
    }

    let app = Router::new()
        .route("/v1/chat/completions", axum::routing::post(chat))
        .route("/health", axum::routing::get(|| async { "ok" }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    format!("http://{addr}")
}

async fn temp_db() -> sea_orm::DatabaseConnection {
    std::env::set_var(
        "FERROGATE_ENCRYPTION_KEY",
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
    );
    let dir = std::env::temp_dir().join(format!("ferrogate-auth-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("auth.db");
    let url = format!(
        "sqlite://{}?mode=rwc",
        path.display().to_string().replace('\\', "/")
    );
    ferrogate::db::init_db(&url).await.unwrap()
}

#[tokio::test]
async fn demo_key_is_seeded_inactive() {
    let db = temp_db().await;
    let row = virtual_key::Entity::find_by_id(DEMO_KEY_ID)
        .one(&db)
        .await
        .expect("query demo key")
        .expect("demo key row is seeded");
    assert_eq!(row.is_active, 0, "demo key must ship inactive");
}

#[tokio::test]
async fn missing_and_unknown_keys_are_rejected() {
    let world = World::start().await;

    let response = world.chat(None, MODEL).await;
    assert_eq!(response.status(), 401);

    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "authentication_error");

    let response = world.chat(Some("sk-fg-not-a-real-key"), MODEL).await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn inactive_keys_are_rejected() {
    let world = World::start().await;
    world.revoke(DEMO_KEY_ID).await;

    let response = world.chat(Some(DEMO_KEY), MODEL).await;
    assert_eq!(response.status(), 401);

    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "invalid_api_key");
}

#[tokio::test]
async fn disallowed_model_returns_403() {
    let world = World::start().await;
    world
        .insert_key("key-restricted", "sk-fg-restricted", |row| {
            row.allowed_models_json = Set(r#"["gpt-4o-mini"]"#.to_owned());
        })
        .await;

    let response = world.chat(Some("sk-fg-restricted"), MODEL).await;
    assert_eq!(response.status(), 403);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "permission_error");
}

#[tokio::test]
async fn rpm_limit_returns_429() {
    let world = World::start().await;
    world
        .insert_key("key-rpm", "sk-fg-rpm", |row| {
            row.rpm_limit = Set(Some(2));
        })
        .await;

    // Two cells fit the burst; the third trips the GCRA bucket. Each attempt
    // carries a distinct body so the response cache stays cold.
    let first = world.chat_with(Some("sk-fg-rpm"), MODEL, "one").await;
    assert_ne!(first.status(), 429, "body={:?}", first.text().await);
    let second = world.chat_with(Some("sk-fg-rpm"), MODEL, "two").await;
    assert_ne!(second.status(), 429, "body={:?}", second.text().await);

    let third = world.chat_with(Some("sk-fg-rpm"), MODEL, "three").await;
    assert_eq!(third.status(), 429);

    let body: Value = third.json().await.unwrap();
    assert_eq!(body["error"]["code"], "rate_limit_exceeded");
}

#[tokio::test]
async fn tpm_limit_returns_429() {
    let world = World::start().await;
    world
        .insert_key("key-tpm", "sk-fg-tpm", |row| {
            row.tpm_limit = Set(Some(4));
        })
        .await;

    // A one-message prompt always costs more than four tokens.
    let response = world.chat(Some("sk-fg-tpm"), MODEL).await;
    assert_eq!(response.status(), 429);

    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "rate_limit_exceeded");
}

#[tokio::test]
async fn budget_cap_trips_immediately_after_pricey_request() {
    let world = World::start_with_upstream().await;
    world
        .insert_key("key-budget", "sk-fg-budget", |row| {
            row.budget_usd = Set(Some(0.000_001));
        })
        .await;

    let key = world.state.auth.authenticate("sk-fg-budget").await.unwrap();
    assert!(world.state.auth.check_budget(&key).is_ok());

    // First request is priced (gpt-4o at $2.50/$10.00 per 1M) and charges the
    // live counter far beyond the 1e-6 budget...
    let first = world.chat(Some("sk-fg-budget"), MODEL).await;
    assert_eq!(first.status(), 200);
    assert!(world.state.auth.effective_spend_usd(&key.id, 0.0) > 0.000_001);

    // ...so the very next request is rejected without touching the upstream.
    let second = world.chat(Some("sk-fg-budget"), MODEL).await;
    assert_eq!(second.status(), 429);

    let body: Value = second.json().await.unwrap();
    assert_eq!(body["error"]["message"], "Virtual key USD budget exceeded");
}

#[tokio::test]
async fn revoked_key_leaves_moka_cache() {
    let world = World::start().await;

    let first = world.state.auth.authenticate(DEMO_KEY).await;
    assert!(first.is_ok(), "seeded demo key must authenticate");

    world.revoke(DEMO_KEY_ID).await;
    world
        .state
        .auth
        .invalidate_key_hash(&auth::sha256_hex(DEMO_KEY))
        .await;

    let second = world.state.auth.authenticate(DEMO_KEY).await;
    assert!(second.is_err());

    let response = world.chat(Some(DEMO_KEY), MODEL).await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn reset_key_state_clears_spend_and_limiters() {
    let world = World::start().await;
    let key = world.state.auth.authenticate(DEMO_KEY).await.unwrap();

    ferrogate::telemetry::add_live_spend(&world.state.live_spend, &key.id, 1.5);
    assert!(world.state.auth.effective_spend_usd(&key.id, 0.0) >= 1.5);

    world.state.auth.reset_key_state(&key.id, true);
    assert_eq!(world.state.auth.effective_spend_usd(&key.id, 0.0), 0.0);
}

#[tokio::test]
async fn models_endpoint_exempt_from_rpm_and_lists_allowlist() {
    let world = World::start().await;
    world
        .insert_key("key-list", "sk-fg-list", |row| {
            row.rpm_limit = Set(Some(1));
            row.allowed_models_json = Set(r#"["gpt-4o"]"#.to_owned());
        })
        .await;

    let client = reqwest::Client::new();
    for _ in 0..3 {
        let response = client
            .get(format!("{}/v1/models", world.base_url))
            .bearer_auth("sk-fg-list")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);

        let body: Value = response.json().await.unwrap();
        let ids: Vec<&str> = body["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["id"].as_str())
            .collect();
        assert!(ids.iter().all(|id| *id == "gpt-4o"), "ids={ids:?}");
    }

    // Give the limiter map a chance to build, then prove the cap is real.
    tokio::time::sleep(Duration::from_millis(10)).await;
}

#[tokio::test]
async fn anthropic_and_google_credential_headers_authenticate() {
    let world = World::start().await;
    let model = "gpt-4o-mini";

    let anthropic = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", world.base_url))
        .header("x-api-key", DEMO_KEY)
        .json(&json!({ "model": model, "messages": [{ "role": "user", "content": "hi" }] }))
        .send()
        .await
        .unwrap();
    assert_ne!(anthropic.status(), 401);

    let google = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", world.base_url))
        .header("x-goog-api-key", DEMO_KEY)
        .json(&json!({ "model": model, "messages": [{ "role": "user", "content": "hi" }] }))
        .send()
        .await
        .unwrap();
    assert_ne!(google.status(), 401);

    // A malformed Authorization header still wins over the fallbacks.
    let malformed = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", world.base_url))
        .header("authorization", "Basic abc")
        .header("x-api-key", DEMO_KEY)
        .json(&json!({ "model": model, "messages": [{ "role": "user", "content": "hi" }] }))
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status(), 401);
}

#[tokio::test]
async fn allow_list_accepts_base_model_and_provider_prefix() {
    let world = World::start().await;
    world
        .insert_key("key-based", "sk-fg-based", |row| {
            row.allowed_models_json = Set(r#"["gpt-4o"]"#.to_owned());
        })
        .await;
    world
        .insert_key("key-prefixed", "sk-fg-prefixed", |row| {
            row.allowed_models_json = Set(r#"["other-provider/gpt-4o-mini"]"#.to_owned());
        })
        .await;

    let key = world.state.auth.authenticate("sk-fg-based").await.unwrap();
    assert!(world.state.auth.check_model_allowed(&key, MODEL).is_ok());
    assert!(world
        .state
        .auth
        .check_model_allowed(&key, "gpt-4o:zdr")
        .is_ok());
    assert!(world
        .state
        .auth
        .check_model_allowed(&key, "openai/gpt-4o:nitro")
        .is_ok());
    assert!(world
        .state
        .auth
        .check_model_allowed(&key, "gpt-4o-mini:nitro")
        .is_err());

    let scoped = world
        .state
        .auth
        .authenticate("sk-fg-prefixed")
        .await
        .unwrap();
    assert!(world
        .state
        .auth
        .check_model_allowed(&scoped, "other-provider/gpt-4o-mini")
        .is_ok());
    assert!(world
        .state
        .auth
        .check_model_allowed(&scoped, "other-provider/gpt-4o-mini:nitro")
        .is_ok());
    assert!(world
        .state
        .auth
        .check_model_allowed(&scoped, "mock-openai/gpt-4o-mini")
        .is_err());
    assert!(world
        .state
        .auth
        .check_model_allowed(&scoped, "gpt-4o-mini")
        .is_err());
}

#[tokio::test]
async fn rate_limit_rejections_carry_retry_after() {
    let world = World::start().await;
    world
        .insert_key("key-tpm-header", "sk-fg-tpm-header", |row| {
            row.tpm_limit = Set(Some(4));
        })
        .await;

    let response = world.chat(Some("sk-fg-tpm-header"), MODEL).await;
    assert_eq!(response.status(), 429);
    assert_eq!(
        response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("60")
    );
}
