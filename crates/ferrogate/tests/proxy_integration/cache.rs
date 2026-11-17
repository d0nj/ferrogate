//! Exact response-cache behaviour for deterministic requests.

use axum::http::StatusCode;
use sea_orm::{ActiveModelTrait, Set};
use serde_json::Value;

use ferrogate::{auth::sha256_hex, entities::virtual_key};

use crate::support::{chat_body, FailureMode, Harness, MockServer};

#[tokio::test]
async fn identical_deterministic_requests_hit_the_exact_cache() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("cache-alias", "prov-openai", "gpt-4o-mini", 0)
        .await;

    let body = chat_body("cache-alias", false, Some(0.0));

    let first = harness.post_chat(body.clone()).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_ne!(
        first
            .headers()
            .get("x-ferrogate-cache")
            .and_then(|value| value.to_str().ok()),
        Some("HIT")
    );
    let first_body: Value = first.json().await.unwrap();

    let second = harness.post_chat(body.clone()).await;
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        second
            .headers()
            .get("x-ferrogate-cache")
            .and_then(|value| value.to_str().ok()),
        Some("HIT"),
        "second identical request must be served from cache"
    );
    let second_body: Value = second.json().await.unwrap();
    assert_eq!(first_body["choices"], second_body["choices"]);

    assert_eq!(
        mock.openai_calls(),
        1,
        "upstream must be called exactly once"
    );

    let logs = harness.await_logs("cache-alias").await;
    assert_eq!(logs.len(), 2, "MISS and HIT are both logged: {logs:?}");
    let log = &logs[1];
    assert_eq!(log["status_code"], 200);
    assert_eq!(log["cache_hit"], 1);

    // Non-deterministic sampling must bypass the cache entirely.
    let warm = harness
        .post_chat(chat_body("cache-alias", false, Some(0.9)))
        .await;
    assert_ne!(
        warm.headers()
            .get("x-ferrogate-cache")
            .and_then(|value| value.to_str().ok()),
        Some("HIT")
    );
    assert_eq!(mock.openai_calls(), 2);
}

#[tokio::test]
async fn cache_hits_are_scoped_per_key_and_enforce_the_allow_list() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("scoped-alias", "prov-openai", "gpt-4o-mini", 0)
        .await;

    let restricted_raw = "sk-fg-test-restricted-key-01";
    virtual_key::ActiveModel {
        id: Set("key-restricted".to_string()),
        name: Set("Restricted".to_string()),
        key_prefix: Set("sk-fg-test...".to_string()),
        key_hash: Set(sha256_hex(restricted_raw)),
        budget_usd: Set(None),
        spent_usd: Set(0.0),
        rpm_limit: Set(None),
        tpm_limit: Set(None),
        allowed_models_json: Set(r#"["other-model"]"#.to_string()),
        is_active: Set(1),
        created_at: Set("2026-01-01T00:00:00Z".to_string()),
        last_used_at: Set(None),
        ..Default::default()
    }
    .insert(&harness.db)
    .await
    .expect("seed restricted key");

    let body = chat_body("scoped-alias", false, Some(0.0));
    let first = harness.post_chat(body.clone()).await;
    assert_eq!(first.status(), StatusCode::OK);

    let replay = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", harness.base_url))
        .bearer_auth(restricted_raw)
        .json(&body)
        .send()
        .await
        .expect("replay");
    assert_eq!(
        replay.status(),
        StatusCode::FORBIDDEN,
        "a key outside the allow-list must not receive another key's hit"
    );
    assert_ne!(
        replay
            .headers()
            .get("x-ferrogate-cache")
            .and_then(|value| value.to_str().ok()),
        Some("HIT")
    );
    assert_eq!(
        mock.openai_calls(),
        1,
        "denied replay must not reach upstream"
    );
}
