//! Shared fixtures: mock upstream servers, the FerroGate harness and SSE helpers.

mod mock;
mod sse;

pub use mock::{FailureMode, MockServer};
pub use sse::{collect_sse, concat_deltas};

use std::{net::SocketAddr, time::Duration};

use ferrogate::{
    build_app,
    config::AppConfig,
    entities::{provider, request_log, routing_rule, virtual_key},
    AppState,
};
use sea_orm::{
    sea_query::Expr, ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::{json, Value};

/// Seeded demo credential.
pub const DEMO_KEY: &str = "sk-fg-demo-ferrogate-local-key-01";
/// Seeded demo key row id.
const DEMO_KEY_ID: &str = "key-demo-local";

fn ensure_test_key() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::env::set_var(
            "FERROGATE_ENCRYPTION_KEY",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        );
    });
}

/// A running FerroGate instance backed by a throwaway SQLite database.
pub struct Harness {
    pub base_url: String,
    pub db: sea_orm::DatabaseConnection,
    pub state: AppState,
    telemetry_worker: Option<tokio::task::JoinHandle<()>>,
    _dir: std::path::PathBuf,
}

impl Harness {
    /// Start FerroGate with the given `(provider_id, provider_type, base_url)` rows.
    pub async fn start(providers: &[(&str, &str, &str)]) -> Self {
        ensure_test_key();
        let dir = std::env::temp_dir().join(format!("ferrogate-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            db_path.display().to_string().replace('\\', "/")
        );

        let db = ferrogate::db::init_db(&database_url).await.unwrap();

        // Replace seeded provider/routing rows with mock-backed ones.
        routing_rule::Entity::delete_many().exec(&db).await.unwrap();
        provider::Entity::delete_many().exec(&db).await.unwrap();

        // The demo key carries a 120 rpm limit that the whole suite would burn
        // through; unconfigured limits keep these tests focused on proxying.
        // The demo key is seeded inactive; tests opt it in explicitly.
        virtual_key::Entity::update_many()
            .col_expr(virtual_key::Column::IsActive, Expr::value(1))
            .filter(virtual_key::Column::Id.eq(DEMO_KEY_ID))
            .exec(&db)
            .await
            .unwrap();
        virtual_key::Entity::update_many()
            .col_expr(
                virtual_key::Column::RpmLimit,
                Expr::value(Option::<i64>::None),
            )
            .col_expr(
                virtual_key::Column::TpmLimit,
                Expr::value(Option::<i64>::None),
            )
            .col_expr(
                virtual_key::Column::BudgetUsd,
                Expr::value(Option::<f64>::None),
            )
            .filter(virtual_key::Column::Id.eq(DEMO_KEY_ID))
            .exec(&db)
            .await
            .unwrap();

        for (id, kind, base_url) in providers {
            provider::ActiveModel {
                id: Set((*id).to_owned()),
                name: Set(format!("mock-{id}")),
                provider_type: Set((*kind).to_owned()),
                base_url: Set((*base_url).to_owned()),
                api_key: Set("test-key".to_owned()),
                models_json: Set(model_list_for(kind).to_owned()),
                is_active: Set(1),
                timeout_ms: Set(30_000),
                created_at: Set("2026-01-01T00:00:00Z".to_owned()),
                updated_at: Set("2026-01-01T00:00:00Z".to_owned()),
                api_dialect: Set("chat".to_owned()),
                ..Default::default()
            }
            .insert(&db)
            .await
            .unwrap();
        }

        let (state, worker) = AppState::from_db_and_config(db.clone(), AppConfig::default()).await;
        let app = build_app(state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            base_url: format!("http://{addr}"),
            db,
            state,
            telemetry_worker: Some(worker),
            _dir: dir,
        }
    }

    /// Add one routing rule: `alias -> provider/upstream_model` at `position`.
    pub async fn add_rule(
        &self,
        alias: &str,
        provider_id: &str,
        upstream_model: &str,
        position: i64,
    ) {
        routing_rule::ActiveModel {
            id: Set(format!("rule-{alias}-{provider_id}-{position}")),
            alias: Set(alias.to_owned()),
            provider_id: Set(provider_id.to_owned()),
            upstream_model: Set(upstream_model.to_owned()),
            weight: Set(100),
            position: Set(position),
            is_active: Set(1),
            created_at: Set("2026-01-01T00:00:00Z".to_owned()),
            updated_at: Set("2026-01-01T00:00:00Z".to_owned()),
        }
        .insert(&self.db)
        .await
        .unwrap();

        self.state.routing.invalidate().await;
    }

    /// Mark a provider as zero-data-retention capable.
    pub async fn mark_zdr(&self, provider_id: &str) {
        provider::Entity::update_many()
            .col_expr(provider::Column::Zdr, Expr::value(1))
            .filter(provider::Column::Id.eq(provider_id))
            .exec(&self.db)
            .await
            .unwrap();
        self.state.routing.invalidate().await;
    }

    /// Await the telemetry worker's asynchronously persisted log rows.
    ///
    /// Rows are returned oldest-first for `model`; each entry mirrors the
    /// `request_logs` columns the assertions care about as JSON.
    pub async fn await_logs(&self, model: &str) -> Vec<Value> {
        for _ in 0..60 {
            let rows = request_log::Entity::find()
                .filter(request_log::Column::RequestedModel.eq(model))
                .order_by_asc(request_log::Column::CreatedAt)
                .all(&self.db)
                .await;

            if let Ok(rows) = rows {
                if !rows.is_empty() {
                    return rows.into_iter().map(log_json).collect();
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        panic!("no request_logs row for `{model}` within 6s");
    }

    /// Await the first persisted log row for `model` (see [`Self::await_logs`]).
    pub async fn await_log(&self, model: &str) -> Value {
        self.await_logs(model).await.swap_remove(0)
    }

    /// `POST /v1/chat/completions`
    pub async fn post_chat(&self, body: Value) -> reqwest::Response {
        self.post_chat_with_headers(body, &[]).await
    }

    /// `POST /v1/chat/completions` with extra headers.
    pub async fn post_chat_with_headers(
        &self,
        body: Value,
        headers: &[(&str, &str)],
    ) -> reqwest::Response {
        let mut builder = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.base_url))
            .bearer_auth(DEMO_KEY)
            .json(&body);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.send().await.unwrap()
    }

    /// `GET /v1/models` with the demo credential.
    pub async fn get_models(&self) -> reqwest::Response {
        reqwest::Client::new()
            .get(format!("{}/v1/models", self.base_url))
            .bearer_auth(DEMO_KEY)
            .send()
            .await
            .unwrap()
    }

    /// `POST <path>` with the demo credential and optional extra headers.
    pub async fn post_json(&self, path: &str, body: Value) -> reqwest::Response {
        self.post_json_with_headers(path, body, &[]).await
    }

    /// `POST <path>` with extra headers.
    pub async fn post_json_with_headers(
        &self,
        path: &str,
        body: Value,
        headers: &[(&str, &str)],
    ) -> reqwest::Response {
        let mut builder = reqwest::Client::new()
            .post(format!("{}{path}", self.base_url))
            .bearer_auth(DEMO_KEY)
            .json(&body);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.send().await.unwrap()
    }

    /// `GET <path>` with the demo credential.
    pub async fn get(&self, path: &str) -> reqwest::Response {
        reqwest::Client::new()
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(DEMO_KEY)
            .send()
            .await
            .unwrap()
    }
}

/// Project a persisted row onto the subset of columns tests assert on.
fn log_json(row: request_log::Model) -> Value {
    json!({
        "id": row.id,
        "status_code": row.status_code,
        "input_tokens": row.input_tokens,
        "output_tokens": row.output_tokens,
        "ttft_ms": row.ttft_ms,
        "cost_usd": row.cost_usd,
        "latency_ms": row.latency_ms,
        "fallback_count": row.fallback_count,
        "cache_hit": row.cache_hit,
        "response_text": row.response_text,
        "routed_provider_id": row.routed_provider_id,
        "routed_model": row.routed_model,
        "is_streaming": row.is_streaming,
    })
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(worker) = self.telemetry_worker.take() {
            worker.abort();
        }
    }
}

fn model_list_for(kind: &str) -> &'static str {
    match kind {
        "openai" => r#"["gpt-4o-mini","gpt-4o"]"#,
        "anthropic" => r#"["claude-3-5-haiku-latest"]"#,
        _ => r#"["gemini-2.5-flash"]"#,
    }
}

/// Minimal one-message request body.
pub fn chat_body(model: &str, stream: bool, temperature: Option<f64>) -> Value {
    json!({
        "model": model,
        "messages": [{ "role": "user", "content": "Say hello" }],
        "stream": stream,
        "temperature": temperature,
    })
}

#[allow(dead_code)]
fn _probe() {}
