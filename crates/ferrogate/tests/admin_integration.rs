//! End-to-end coverage of the `/admin/api/*` surface.
//!
//! The harness assembles the admin router directly over an AppState backed by a
//! throwaway SQLite file (migrations applied), so no proxy middleware interferes.

use std::{net::SocketAddr, path::PathBuf};

use axum::Router;
use ferrogate::{
    auth::sha256_hex,
    config::AppConfig,
    db,
    entities::{
        app_setting, model_pricing, provider, request_log, routing_rule, virtual_key, webhook,
    },
    AppState,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::{json, Value};

/// A running admin API over a fresh database.
struct AdminHarness {
    base_url: String,
    db: sea_orm::DatabaseConnection,
    state: AppState,
    _dir: PathBuf,
}

impl Drop for AdminHarness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self._dir);
    }
}

impl AdminHarness {
    async fn start() -> Self {
        std::env::set_var(
            "FERROGATE_ENCRYPTION_KEY",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        );
        let dir = std::env::temp_dir().join(format!("ferrogate-admin-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("admin.db");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            db_path.display().to_string().replace('\\', "/")
        );

        let db = db::init_db(&database_url)
            .await
            .expect("bootstrap database");
        let (state, worker) = AppState::from_db_and_config(db.clone(), AppConfig::default()).await;
        drop(worker);

        // Start from an empty provider/routing slate (drops the demo seed rows).
        routing_rule::Entity::delete_many().exec(&db).await.unwrap();
        provider::Entity::delete_many().exec(&db).await.unwrap();

        let app: Router = ferrogate::admin::router().with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            base_url: format!("http://{addr}"),
            db,
            state,
            _dir: dir,
        }
    }

    fn client(&self) -> reqwest::Client {
        reqwest::Client::new()
    }

    async fn get(&self, path: &str) -> (reqwest::StatusCode, Value) {
        let response = self
            .client()
            .get(format!("{}{path}", self.base_url))
            .send()
            .await
            .unwrap();
        let status = response.status();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    async fn post(&self, path: &str, body: Value) -> (reqwest::StatusCode, Value) {
        let response = self
            .client()
            .post(format!("{}{path}", self.base_url))
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    async fn patch(&self, path: &str, body: Value) -> (reqwest::StatusCode, Value) {
        let response = self
            .client()
            .patch(format!("{}{path}", self.base_url))
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    async fn put(&self, path: &str, body: Value) -> (reqwest::StatusCode, Value) {
        let response = self
            .client()
            .put(format!("{}{path}", self.base_url))
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    async fn delete(&self, path: &str) -> (reqwest::StatusCode, Value) {
        let response = self
            .client()
            .delete(format!("{}{path}", self.base_url))
            .send()
            .await
            .unwrap();
        let status = response.status();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    /// Insert a provider row directly, bypassing the API.
    async fn seed_provider(&self, id: &str, name: &str, api_key: &str) {
        provider::ActiveModel {
            id: Set(id.to_owned()),
            name: Set(name.to_owned()),
            provider_type: Set("openai".to_owned()),
            base_url: Set("https://api.example.com".to_owned()),
            api_key: Set(api_key.to_owned()),
            models_json: Set(r#"["gpt-4o-mini"]"#.to_owned()),
            is_active: Set(1),
            timeout_ms: Set(30_000),
            created_at: Set("2026-01-01T00:00:00.000Z".to_owned()),
            updated_at: Set("2026-01-01T00:00:00.000Z".to_owned()),
            api_dialect: Set("chat".to_owned()),
            ..Default::default()
        }
        .insert(&self.db)
        .await
        .unwrap();
    }

    /// Insert one request log row at `created_at`.
    async fn seed_log(
        &self,
        id: &str,
        created_at: &str,
        status_code: i64,
        cost_usd: f64,
        ttft_ms: Option<f64>,
    ) {
        request_log::ActiveModel {
            id: Set(id.to_owned()),
            created_at: Set(created_at.to_owned()),
            virtual_key_id: Set(Some("key-1".to_owned())),
            virtual_key_name: Set(Some("Primary".to_owned())),
            requested_model: Set("gpt-4o-mini".to_owned()),
            routed_provider_id: Set(Some("prov-1".to_owned())),
            routed_provider_name: Set(Some("OpenAI".to_owned())),
            routed_model: Set(Some("gpt-4o-mini".to_owned())),
            status_code: Set(status_code),
            is_streaming: Set(0),
            input_tokens: Set(100),
            output_tokens: Set(50),
            cost_usd: Set(cost_usd),
            ttft_ms: Set(ttft_ms),
            latency_ms: Set(420.0),
            fallback_count: Set(0),
            cache_hit: Set(0),
            routing_trace_json: Set("[]".to_owned()),
            request_messages_json: Set(Some(r#"[{"role":"user","content":"hi"}]"#.to_owned())),
            response_text: Set(Some("hello".to_owned())),
            error_message: Set(None),
            ..Default::default()
        }
        .insert(&self.db)
        .await
        .unwrap();
    }
}

fn rfc3339_minutes_ago(minutes: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::minutes(minutes))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[tokio::test]
async fn overview_reports_kpis_timeseries_and_provider_health() {
    let harness = AdminHarness::start().await;
    harness
        .seed_provider("prov-1", "Test OpenAI", "sk-secret-value-1234")
        .await;

    // Three successes and one failure spread across the 24h window.
    harness
        .seed_log("log-1", &rfc3339_minutes_ago(60), 200, 0.01, Some(100.0))
        .await;
    harness
        .seed_log("log-2", &rfc3339_minutes_ago(50), 200, 0.02, Some(200.0))
        .await;
    harness
        .seed_log("log-3", &rfc3339_minutes_ago(40), 200, 0.03, Some(400.0))
        .await;
    harness
        .seed_log("log-4", &rfc3339_minutes_ago(30), 500, 0.04, None)
        .await;

    let (status, body) = harness.get("/admin/api/overview?window=24h").await;
    assert_eq!(status, reqwest::StatusCode::OK);

    let kpis = &body["kpis"];
    assert_eq!(kpis["total_requests"], 4);
    assert!((kpis["total_spend_usd"].as_f64().unwrap() - 0.1).abs() < 1e-9);
    assert_eq!(kpis["error_rate_pct"], 25.0);
    // Sorted TTFTs: [100, 200, 400] -> p50 = 200, p95 = 400.
    assert_eq!(kpis["ttft_p50_ms"].as_f64().unwrap(), 200.0);
    assert_eq!(kpis["ttft_p95_ms"].as_f64().unwrap(), 400.0);
    assert!(kpis["rpm"].as_f64().unwrap() > 0.0);

    let series = body["timeseries"].as_array().unwrap();
    assert_eq!(series.len(), 24, "24h window yields 24 hourly buckets");

    let populated: Vec<&Value> = series
        .iter()
        .filter(|bucket| bucket["requests"].as_i64().unwrap() > 0)
        .collect();
    assert!(!populated.is_empty());
    // Every populated bucket carries our model/provider attribution and token counts.
    for bucket in &populated {
        assert!(bucket["input_tokens"].as_i64().unwrap() > 0);
        assert_eq!(bucket["by_model"][0]["model"], "gpt-4o-mini");
        assert_eq!(bucket["by_model"][0]["provider"], "OpenAI");
    }
    let bucketed_tokens: i64 = populated
        .iter()
        .map(|bucket| bucket["input_tokens"].as_i64().unwrap())
        .sum();
    assert_eq!(bucketed_tokens, 400, "4 rows x 100 input tokens");

    let health = body["providers_health"].as_array().unwrap();
    assert_eq!(health.len(), 1);
    assert_eq!(health[0]["provider_id"], "prov-1");
    assert_eq!(health[0]["name"], "Test OpenAI");
    assert_eq!(health[0]["circuit_state"], "closed");
    assert_eq!(health[0]["is_active"], true);
}

#[tokio::test]
async fn overview_window_changes_bucket_width() {
    let harness = AdminHarness::start().await;

    let (_, body) = harness.get("/admin/api/overview?window=1h").await;
    assert_eq!(body["window"], "1h");
    assert_eq!(body["timeseries"].as_array().unwrap().len(), 12);

    let (_, body) = harness.get("/admin/api/overview?window=7d").await;
    assert_eq!(body["window"], "7d");
    assert_eq!(body["timeseries"].as_array().unwrap().len(), 7);

    let (_, body) = harness.get("/admin/api/overview").await;
    assert_eq!(body["window"], "24h", "default window is 24h");
}

#[tokio::test]
async fn providers_accept_typesafe_and_routing_extras() {
    let harness = AdminHarness::start().await;

    let (status, created) = harness
        .post(
            "/admin/api/providers",
            json!({
                "name": "TypeSafe Prod",
                "provider_type": "typesafe",
                "base_url": "https://api.typesafe.example",
                "region": " eu-west ",
                "zdr": true,
                "extra_headers": { "x-team": "edge" },
            }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::CREATED, "body={created}");
    assert_eq!(created["provider_type"], "typesafe");
    assert_eq!(created["region"], "eu-west");
    assert_eq!(created["zdr"], true);
    assert_eq!(created["extra_headers"]["x-team"], "edge");

    let id = created["id"].as_str().unwrap().to_string();
    let (status, updated) = harness
        .patch(
            &format!("/admin/api/providers/{id}"),
            json!({ "region": "", "zdr": false, "extra_headers": {} }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(
        updated["region"].is_null(),
        "blank region clears: {updated}"
    );
    assert_eq!(updated["zdr"], false);
    assert_eq!(updated["extra_headers"], json!({}));
}

#[tokio::test]
async fn providers_crud_masks_secrets_and_preserves_them_on_edit() {
    let harness = AdminHarness::start().await;

    let (status, created) = harness
        .post(
            "/admin/api/providers",
            json!({
                "name": "Anthropic Prod",
                "provider_type": "Anthropic",
                "base_url": "https://api.anthropic.com",
                "api_key": "sk-ant-super-secret-key",
                "models": ["claude-3-5-haiku-latest"],
            }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::CREATED);
    assert_eq!(created["provider_type"], "anthropic");
    assert_eq!(created["has_api_key"], true);
    assert!(!created["api_key_masked"]
        .as_str()
        .unwrap()
        .contains("super-secret"));
    assert_eq!(created["models"][0], "claude-3-5-haiku-latest");
    assert!(
        created.get("api_key").is_none(),
        "raw key must never be serialised"
    );

    let id = created["id"].as_str().unwrap().to_string();

    let (status, body) = harness
        .post(
            "/admin/api/providers",
            json!({
                "name": "Anthropic Prod",
                "provider_type": "anthropic",
                "base_url": "https://dup.example.com",
            }),
        )
        .await;
    assert_eq!(
        status,
        reqwest::StatusCode::BAD_REQUEST,
        "duplicate name rejected: {body}"
    );

    let (status, body) = harness
        .post(
            "/admin/api/providers",
            json!({"name": "Bogus", "provider_type": "cohere", "base_url": "https://x"}),
        )
        .await;
    assert_eq!(
        status,
        reqwest::StatusCode::BAD_REQUEST,
        "unknown type rejected: {body}"
    );

    let (status, listed) = harness.get("/admin/api/providers").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(listed["providers"].as_array().unwrap().len(), 1);

    // Editing other fields leaves the stored secret untouched.
    let (status, updated) = harness
        .patch(
            &format!("/admin/api/providers/{id}"),
            json!({"name": "Anthropic Prod v2", "timeout_ms": 120_000}),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(updated["name"], "Anthropic Prod v2");
    assert_eq!(updated["timeout_ms"], 120_000);
    assert_eq!(updated["has_api_key"], true);

    let stored = provider::Entity::find_by_id(&id)
        .one(&harness.db)
        .await
        .unwrap()
        .unwrap();
    // The column is sealed: reading it back must not reveal the credential.
    assert!(
        stored.api_key.starts_with("enc:v1:"),
        "credential stored in the clear: {}",
        stored.api_key
    );
    assert_ne!(stored.api_key, "sk-ant-super-secret-key");
    assert_eq!(
        harness
            .state
            .secrets
            .open(&stored.api_key)
            .expect("stored credential decrypts"),
        "sk-ant-super-secret-key"
    );

    // Explicit clear wipes it.
    let (status, cleared) = harness
        .patch(
            &format!("/admin/api/providers/{id}"),
            json!({"clear_api_key": true}),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(cleared["has_api_key"], false);
    assert_eq!(cleared["api_key_masked"], "");

    let (status, _) = harness
        .patch("/admin/api/providers/does-not-exist", json!({"name": "x"}))
        .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);

    let (status, deleted) = harness.delete(&format!("/admin/api/providers/{id}")).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(deleted["deleted"], true);
    assert_eq!(deleted["id"], id);

    let (status, _) = harness.delete(&format!("/admin/api/providers/{id}")).await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn routes_replace_targets_in_order_and_prune_fallbacks() {
    let harness = AdminHarness::start().await;
    harness.seed_provider("prov-1", "Test OpenAI", "sk-1").await;
    harness
        .seed_provider("prov-2", "Test Anthropic", "sk-2")
        .await;

    let (status, created) = harness
        .put(
            "/admin/api/routes/fast",
            json!({
                "targets": [
                    {"provider_id": "prov-1", "upstream_model": "gpt-4o-mini"},
                    {"provider_id": "prov-2", "upstream_model": "claude-haiku"},
                ],
                "fallbacks": [],
                "strategy": null,
            }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(created["targets"][0]["provider_name"], "Test OpenAI");
    assert_eq!(created["targets"][1]["provider_name"], "Test Anthropic");

    // Array order is attempt order: it must survive the round trip verbatim.
    let (_, fetched) = harness.get("/admin/api/routes/fast").await;
    assert_eq!(fetched["targets"][0]["upstream_model"], "gpt-4o-mini");
    assert_eq!(fetched["targets"][1]["upstream_model"], "claude-haiku");

    // A second PUT is a full replace, not an append.
    let (status, replaced) = harness
        .put(
            "/admin/api/routes/fast",
            json!({
                "targets": [{"provider_id": "prov-2", "upstream_model": "claude-sonnet"}],
                "fallbacks": [],
                "strategy": "latency",
            }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(replaced["targets"].as_array().unwrap().len(), 1);
    assert_eq!(replaced["strategy"], "latency");

    let rows = routing_rule::Entity::find().all(&harness.db).await.unwrap();
    assert_eq!(rows.len(), 1, "replaced targets must not leave stale rows");
    assert_eq!(rows[0].position, 0, "single target sits at position 0");

    // Deleting an alias prunes it from every other alias's chain.
    harness
        .put(
            "/admin/api/routes/smart",
            json!({
                "targets": [{"provider_id": "prov-1", "upstream_model": "gpt-4o"}],
                "fallbacks": ["fast"],
                "strategy": null,
            }),
        )
        .await;
    let (status, _) = harness.delete("/admin/api/routes/fast").await;
    assert_eq!(status, reqwest::StatusCode::NO_CONTENT);

    let (_, smart) = harness.get("/admin/api/routes/smart").await;
    assert_eq!(
        smart["fallbacks"].as_array().unwrap().len(),
        0,
        "deleted alias must not linger as a fallback"
    );

    // Deleting an absent alias is a no-op, not an error: the end state is the
    // same, and retries are safe.
    let (status, _) = harness.delete("/admin/api/routes/fast").await;
    assert_eq!(status, reqwest::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn routes_reject_malformed_payloads() {
    let harness = AdminHarness::start().await;
    harness.seed_provider("prov-1", "Test OpenAI", "sk-1").await;

    for body in [
        json!({"targets": [], "fallbacks": [], "strategy": null}),
        json!({"targets": [{"provider_id": "nope", "upstream_model": "x"}], "fallbacks": [], "strategy": null}),
        json!({"targets": [{"provider_id": "prov-1", "upstream_model": "x", "weight": 0}], "fallbacks": [], "strategy": null}),
        json!({"targets": [{"provider_id": "prov-1", "upstream_model": "x"}], "fallbacks": ["fast"], "strategy": null}),
    ] {
        let (status, _) = harness.put("/admin/api/routes/fast", body).await;
        assert!(
            status.is_client_error(),
            "malformed route should have been rejected, got {status}"
        );
    }

    let (status, _) = harness.get("/admin/api/routes/fast").await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn keys_issue_plaintext_once_and_store_only_the_digest() {
    let harness = AdminHarness::start().await;

    let (status, created) = harness
        .post(
            "/admin/api/keys",
            json!({
                "name": "CI Runner",
                "budget_usd": 12.5,
                "rpm_limit": 60,
                "tpm_limit": 90_000,
                "allowed_models": ["gpt-4o-mini"],
            }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::CREATED);

    let plaintext = created["plaintext_key"].as_str().unwrap().to_string();
    assert!(plaintext.starts_with("sk-fg-"), "plaintext={plaintext}");
    assert_eq!(plaintext.len(), "sk-fg-".len() + 32);

    let key_id = created["key"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["key"]["budget_usd"], 12.5);
    assert_eq!(created["key"]["allowed_models"][0], "gpt-4o-mini");
    assert_eq!(created["key"]["is_active"], true);

    // The database stores only the SHA-256 digest, never the plaintext.
    let row = virtual_key::Entity::find_by_id(&key_id)
        .one(&harness.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.key_hash, sha256_hex(&plaintext));
    assert_ne!(row.key_hash, plaintext);
    assert!(row.key_prefix.starts_with("sk-fg-"));
    assert!(row.key_prefix.ends_with("..."));

    let (status, listed) = harness.get("/admin/api/keys").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let keys = listed["keys"].as_array().unwrap();
    assert!(keys.iter().any(|key| key["id"] == key_id.as_str()));
    assert!(
        !listed.to_string().contains(&plaintext),
        "listing must never leak the plaintext key"
    );
}

#[tokio::test]
async fn keys_patch_updates_limits_and_resets_spend() {
    let harness = AdminHarness::start().await;

    let (_, created) = harness
        .post(
            "/admin/api/keys",
            json!({"name": "Budgets", "budget_usd": 5.0}),
        )
        .await;
    let key_id = created["key"]["id"].as_str().unwrap().to_string();

    // Persist some spend, then mirror it into the live counter.
    let mut active: virtual_key::ActiveModel = virtual_key::Entity::find_by_id(&key_id)
        .one(&harness.db)
        .await
        .unwrap()
        .unwrap()
        .into();
    active.spent_usd = Set(4.25);
    active.update(&harness.db).await.unwrap();

    let (status, listed) = harness.get("/admin/api/keys").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let spent = listed["keys"][0]["spent_usd"].as_f64().unwrap();
    assert!((spent - 4.25).abs() < 1e-9);

    let (status, patched) = harness
        .patch(
            &format!("/admin/api/keys/{key_id}"),
            json!({"name": "Budgets v2", "rpm_limit": 30, "reset_spend": true}),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(patched["name"], "Budgets v2");
    assert_eq!(patched["rpm_limit"], 30);
    assert_eq!(patched["spent_usd"].as_f64().unwrap(), 0.0);

    let (status, cleared) = harness
        .patch(
            &format!("/admin/api/keys/{key_id}"),
            json!({"clear_budget": true, "clear_rpm_limit": true, "is_active": false}),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(cleared["budget_usd"].is_null());
    assert!(cleared["rpm_limit"].is_null());
    assert_eq!(cleared["is_active"], false);

    let (status, deleted) = harness.delete(&format!("/admin/api/keys/{key_id}")).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(deleted["deleted"], true);

    let (status, _) = harness.delete(&format!("/admin/api/keys/{key_id}")).await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);

    let (status, _) = harness
        .patch("/admin/api/keys/missing", json!({"name": "x"}))
        .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn logs_paginate_filter_and_expose_detail_payload() {
    let harness = AdminHarness::start().await;

    for index in 0..5 {
        harness
            .seed_log(
                &format!("log-{index}"),
                &rfc3339_minutes_ago(index as i64),
                200,
                0.01,
                Some(120.0),
            )
            .await;
    }
    harness
        .seed_log("log-error", &rfc3339_minutes_ago(0), 500, 0.0, None)
        .await;

    let (status, first_page) = harness.get("/admin/api/logs?page=1&page_size=2").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(first_page["total"], 6);
    assert_eq!(first_page["page"], 1);
    assert_eq!(first_page["page_size"], 2);
    assert_eq!(first_page["items"].as_array().unwrap().len(), 2);
    assert_eq!(first_page["items"][0]["id"], "log-error", "newest first");

    let (_, second_page) = harness.get("/admin/api/logs?page=2&page_size=2").await;
    assert_eq!(second_page["items"].as_array().unwrap().len(), 2);
    assert_ne!(first_page["items"][0]["id"], second_page["items"][0]["id"]);

    let (status, errors) = harness.get("/admin/api/logs?status=500").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(errors["total"], 1);
    assert_eq!(errors["items"][0]["id"], "log-error");

    let (_, by_key) = harness.get("/admin/api/logs?key_id=key-1").await;
    assert_eq!(by_key["total"], 6);

    let (_, searched) = harness.get("/admin/api/logs?search=log-3").await;
    assert_eq!(searched["total"], 1);
    assert_eq!(searched["items"][0]["id"], "log-3");

    let (_, by_model) = harness.get("/admin/api/logs?model=gpt-4o-mini").await;
    assert_eq!(by_model["total"], 6);

    let (status, detail) = harness.get("/admin/api/logs/log-1").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(detail["id"], "log-1");
    assert_eq!(detail["routed_provider_name"], "OpenAI");
    assert_eq!(detail["request_messages"][0]["role"], "user");
    assert_eq!(detail["response_text"], "hello");
    assert_eq!(detail["routing_trace"], json!([]));
    assert_eq!(detail["is_streaming"], false);

    let (status, _) = harness.get("/admin/api/logs/nope").await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn pricing_upsert_updates_and_invalidates_cache() {
    let harness = AdminHarness::start().await;

    // Warm the pricing cache with the seeded value.
    let before = harness.state.pricing.unit_costs("gpt-4o-mini").await;

    let (status, created) = harness
        .put(
            "/admin/api/pricing/gpt-4o-mini",
            json!({"provider_type": "openai", "input_cost_per_1m": 1.5, "output_cost_per_1m": 6.0}),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(created["model"], "gpt-4o-mini");
    assert_eq!(created["input_cost_per_1m"], 1.5);

    let after = harness.state.pricing.unit_costs("gpt-4o-mini").await;
    assert_eq!(after, (1.5, 6.0));
    assert_ne!(before, after);

    // Second PUT takes the update path, not an insert.
    let (status, updated) = harness
        .put(
            "/admin/api/pricing/gpt-4o-mini",
            json!({"provider_type": "openai", "input_cost_per_1m": 2.0, "output_cost_per_1m": 8.0}),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(updated["output_cost_per_1m"], 8.0);

    let rows = model_pricing::Entity::find()
        .filter(model_pricing::Column::Model.eq("gpt-4o-mini"))
        .all(&harness.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "upsert must not duplicate the row");
    assert_eq!(rows[0].input_cost_per_1m, 2.0);

    let (status, listed) = harness.get("/admin/api/pricing").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let pricing = listed["pricing"].as_array().unwrap();
    assert!(pricing.iter().any(|row| row["model"] == "gpt-4o-mini"));
}

#[tokio::test]
async fn pricing_put_inserts_a_model_that_does_not_exist_yet() {
    let harness = AdminHarness::start().await;

    // Not seeded: the PUT must insert. ActiveModel::save() would emit an UPDATE
    // that matches nothing and fail, so this is the path a fresh model takes.
    let (status, created) = harness
        .put(
            "/admin/api/pricing/brand-new-model",
            json!({"provider_type": "openai", "input_cost_per_1m": 3.0, "output_cost_per_1m": 15.0}),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(created["model"], "brand-new-model");

    let rows = model_pricing::Entity::find()
        .filter(model_pricing::Column::Model.eq("brand-new-model"))
        .all(&harness.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "the row must exist after the first PUT");
    assert_eq!(rows[0].output_cost_per_1m, 15.0);

    // A second PUT updates in place rather than duplicating.
    let (status, _) = harness
        .put(
            "/admin/api/pricing/brand-new-model",
            json!({"provider_type": "openai", "input_cost_per_1m": 4.0, "output_cost_per_1m": 16.0}),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let rows = model_pricing::Entity::find()
        .filter(model_pricing::Column::Model.eq("brand-new-model"))
        .all(&harness.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].input_cost_per_1m, 4.0);
}

#[tokio::test]
async fn settings_toggle_redact_prompts_and_persist_it() {
    let harness = AdminHarness::start().await;

    let (status, initial) = harness.get("/admin/api/settings").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(initial["redact_prompts"], false);

    let (status, updated) = harness
        .put("/admin/api/settings", json!({"redact_prompts": true}))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(updated["redact_prompts"], true);

    let (_, reread) = harness.get("/admin/api/settings").await;
    assert_eq!(reread["redact_prompts"], true);

    let stored = app_setting::Entity::find_by_id("redact_prompts")
        .one(&harness.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.value, "true");

    // Toggling back writes the same row instead of duplicating it.
    let (_, reverted) = harness
        .put("/admin/api/settings", json!({"redact_prompts": false}))
        .await;
    assert_eq!(reverted["redact_prompts"], false);

    // Scoped to this key: `app_settings` also carries the seeded routing policy,
    // and the claim under test is that toggling does not duplicate its own row.
    let rows = app_setting::Entity::find()
        .filter(app_setting::Column::Key.eq("redact_prompts"))
        .all(&harness.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].value, "false");
}

#[tokio::test]
async fn prompt_versions_bump_and_activate() {
    let harness = AdminHarness::start().await;
    let base = "/admin/api/prompts";

    let (status, first) = harness
        .post(
            base,
            json!({
                "slug": "explainer",
                "name": "Explainer",
                "system_prompt": "You explain {{topic}}.",
                "user_template": "Explain {{ topic }} for {{audience}}",
                "variables": [{ "name": "topic" }, { "name": "audience", "default": "beginners" }],
                "model": "gpt-4o-mini",
                "temperature": 0.3,
            }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::CREATED);
    assert_eq!(first["prompt"]["version"], 1);
    assert_eq!(first["prompt"]["is_active"], true);

    let (status, second) = harness
        .post(
            base,
            json!({
                "slug": "explainer",
                "name": "Explainer v2",
                "system_prompt": "You explain {{topic}} clearly.",
                "user_template": "Walk me through {{topic}}",
                "is_active": false,
            }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::CREATED);
    assert_eq!(second["prompt"]["version"], 2);

    let (_, listed) = harness.get(base).await;
    let prompts = listed["prompts"].as_array().unwrap();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0]["slug"], "explainer");
    assert_eq!(prompts[0]["versions"], 2);
    assert_eq!(prompts[0]["latest_version"], 2);

    let (_, versions) = harness.get(&format!("{base}/explainer")).await;
    let versions = versions["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0]["version"], 2, "newest first");

    let (status, activated) = harness
        .post(&format!("{base}/explainer/activate/2"), json!({}))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(activated["prompt"]["is_active"], true);

    let (_, reread) = harness.get(&format!("{base}/explainer/1")).await;
    assert_eq!(reread["prompt"]["is_active"], false);

    let (status, updated) = harness
        .patch(
            &format!("{base}/explainer/1"),
            json!({ "system_prompt": "Edited in place." }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(updated["prompt"]["version"], 1);
    assert_eq!(updated["prompt"]["system_prompt"], "Edited in place.");

    let (status, deleted) = harness.delete(&format!("{base}/explainer")).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(deleted["versions"], 2);

    let (status, _) = harness.get(&format!("{base}/explainer")).await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn prompt_render_reports_missing_variables() {
    let harness = AdminHarness::start().await;
    let base = "/admin/api/prompts";

    let (status, _) = harness
        .post(
            base,
            json!({
                "slug": "faq",
                "name": "FAQ",
                "system_prompt": "Answer as {{persona}}.",
                "user_template": "Question about {{topic}}: {{question}}",
                "variables": [
                    { "name": "persona", "default": "a docs writer" },
                    { "name": "topic" },
                    { "name": "question" },
                ],
            }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::CREATED);

    let (status, rendered) = harness
        .post(
            &format!("{base}/faq/render"),
            json!({ "variables": { "topic": "billing" } }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(rendered["system"], "Answer as a docs writer.");
    assert_eq!(rendered["user"], "Question about billing: ");
    assert_eq!(rendered["missing_variables"], json!(["question"]));

    let (status, _) = harness
        .post(&format!("{base}/absent/render"), json!({ "variables": {} }))
        .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn provider_keys_are_scoped_to_their_parent_provider() {
    let harness = AdminHarness::start().await;
    harness.seed_provider("prov-a", "Alpha", "sk-a").await;
    harness.seed_provider("prov-b", "Beta", "sk-b").await;

    let (status, created) = harness
        .post(
            "/admin/api/providers/prov-a/keys",
            json!({ "name": "primary", "api_key": "sk-secret-value" }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::CREATED, "{created}");
    let key_id = created["key"]["id"].as_str().expect("key id").to_string();

    // Addressing the key through the wrong parent must not find it, otherwise a
    // provider id would be enough to edit another provider's credentials.
    let (status, _) = harness
        .patch(
            &format!("/admin/api/providers/prov-b/keys/{key_id}"),
            json!({ "name": "hijacked" }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);

    let (status, _) = harness
        .delete(&format!("/admin/api/providers/prov-b/keys/{key_id}"))
        .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);

    // The real parent still works.
    let (status, updated) = harness
        .patch(
            &format!("/admin/api/providers/prov-a/keys/{key_id}"),
            json!({ "name": "renamed" }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{updated}");
    assert_eq!(updated["key"]["name"], "renamed");
}

#[tokio::test]
async fn routing_policy_roundtrips_and_applies_live() {
    let harness = AdminHarness::start().await;

    let (status, body) = harness.get("/admin/api/routing/policy").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // The seed migration installs the defaults routing used before they were
    // configurable, so an untouched install behaves exactly as it used to.
    assert_eq!(body["policy"]["failure_threshold"], 3);
    assert_eq!(body["policy"]["cache_ttl_seconds"], 5);
    assert_eq!(body["policy"]["strategy"], "weighted");

    let (status, saved) = harness
        .put(
            "/admin/api/routing/policy",
            json!({
                "failure_threshold": 7,
                "cooldown_seconds": 30,
                "cache_ttl_seconds": 12,
                "key_rotations": 1,
                "strategy": "latency",
                "tier_strategies": [{"position": 2, "strategy": "cost"}],
                "retry_on": {
                    "rate_limit": false,
                    "server_error": true,
                    "transport": false,
                    "protocol": false
                },
                "max_attempts": 4
            }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(saved["policy"]["failure_threshold"], 7);
    assert_eq!(saved["policy"]["strategy"], "latency");

    // The live store must reflect the write without a restart.
    let live = harness.state.routing_policy.get();
    assert_eq!(live.failure_threshold, 7);
    assert_eq!(live.cooldown_seconds, 30);
    assert_eq!(live.strategy, ferrogate::routing::TierStrategy::Latency);
    assert_eq!(live.strategy_for(2), ferrogate::routing::TierStrategy::Cost);
    assert!(!live.retry_on.rate_limit);
    assert!(live.retry_on.server_error);

    // And it survives a re-read from disk.
    let (status, again) = harness.get("/admin/api/routing/policy").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(again["policy"]["failure_threshold"], 7);
}

#[tokio::test]
async fn routing_policy_rejects_nonsense() {
    let harness = AdminHarness::start().await;

    for body in [
        json!({"failure_threshold": 0}),
        json!({"cache_ttl_seconds": 0}),
        json!({"cooldown_seconds": 0}),
        json!({
            "tier_strategies": [
                {"position": 1, "strategy": "cost"},
                {"position": 1, "strategy": "latency"}
            ]
        }),
    ] {
        let (status, _) = harness.put("/admin/api/routing/policy", body).await;
        assert_eq!(
            status,
            reqwest::StatusCode::BAD_REQUEST,
            "policy should have been rejected"
        );
    }

    // A rejected write must not have partially applied.
    let (_, body) = harness.get("/admin/api/routing/policy").await;
    assert_eq!(body["policy"]["failure_threshold"], 3);
}

#[tokio::test]
async fn webhook_secrets_are_sealed_at_rest() {
    let harness = AdminHarness::start().await;

    let (status, body) = harness
        .post(
            "/admin/api/webhooks",
            json!({
                "name": "sealed-hook",
                "url": "https://example.com/hook",
                "events": [],
                "secret": "s3cret-value",
            }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::CREATED);
    let id = body["webhook"]["id"].as_str().expect("webhook id");
    assert_eq!(body["webhook"]["secret_masked"], "••••••••");

    let row = webhook::Entity::find_by_id(id)
        .one(&harness.db)
        .await
        .expect("query webhook")
        .expect("webhook row");
    let secret = row.secret.expect("secret stored");
    assert!(
        secret.starts_with("enc:v1:"),
        "secret must be sealed: {secret}"
    );
    assert!(!secret.contains("s3cret"), "plaintext must not leak");
}

#[tokio::test]
async fn delete_log_removes_the_row() {
    let harness = AdminHarness::start().await;
    harness
        .seed_log("log-gone", &rfc3339_minutes_ago(5), 200, 0.01, None)
        .await;

    let (status, _) = harness.get("/admin/api/logs/log-gone").await;
    assert_eq!(status, reqwest::StatusCode::OK);

    let (status, body) = harness.delete("/admin/api/logs/log-gone").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["deleted"], true);

    let (status, _) = harness.get("/admin/api/logs/log-gone").await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
}
