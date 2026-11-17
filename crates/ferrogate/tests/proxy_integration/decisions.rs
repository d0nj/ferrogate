//! Decision ingress: OpenAI, TypeSafe and Gemini dialects over any provider.

use axum::http::StatusCode;
use sea_orm::{ActiveModelTrait, Set};
use serde_json::{json, Value};

use ferrogate::{auth::sha256_hex, entities::virtual_key};

use crate::support::{FailureMode, Harness, MockServer};

fn openai_body(model: &str) -> Value {
    json!({
        "model": model,
        "input": "Is this message harmful?",
        "questions": [{
            "name": "harmful",
            "type": "predicate",
            "instructions": "Decide whether the message is harmful.",
            "false_description": "The message is safe."
        }]
    })
}

fn typesafe_body(model: &str) -> Value {
    json!({
        "model": model,
        "state": "Is this message harmful?",
        "questions": {
            "harmful": {
                "type": "noul",
                "instructions": "Decide whether the message is harmful.",
                "criteria": { "true": "Harmful.", "false": "Safe." }
            }
        }
    })
}

fn gemini_body(model: &str) -> Value {
    json!({
        "model": model,
        "input": [{ "type": "text", "text": "Is this message harmful?" }],
        "questions": {
            "harmful": {
                "type": "noul",
                "instructions": "Decide whether the message is harmful.",
                "criteria": {
                    "true_description": "Harmful.",
                    "false_description": "Safe."
                }
            }
        }
    })
}

#[tokio::test]
async fn every_decision_dialect_routes_and_prices() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;

    for (alias, path, body) in [
        ("dec-openai", "/v1/decisions", openai_body("dec-openai")),
        (
            "dec-typesafe",
            "/v1/systemone",
            typesafe_body("dec-typesafe"),
        ),
        ("dec-gemini", "/v1beta/decisions", gemini_body("dec-gemini")),
    ] {
        harness
            .add_rule(alias, "prov-openai", "gpt-4o-mini", 0)
            .await;

        let response = harness.post_json(path, body).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            response
                .headers()
                .get("x-ferrogate-provider")
                .and_then(|value| value.to_str().ok()),
            Some("mock-prov-openai"),
            "{path}"
        );
        assert!(response.headers().get("x-request-id").is_some(), "{path}");
        assert!(
            response.headers().get("x-ferrogate-cost-usd").is_some(),
            "{path}"
        );

        let body: Value = response.json().await.unwrap();
        let answer = if body["answers"].is_array() {
            body["answers"][0].clone()
        } else {
            body["answers"]["harmful"].clone()
        };
        assert!(answer.is_object(), "{path}: {body}");

        let log = harness.await_log(alias).await;
        assert_eq!(log["status_code"], 200, "{path}");
        assert!(log["input_tokens"].as_i64().unwrap() > 0, "{path}");
        assert_eq!(log["routed_provider_id"], "prov-openai", "{path}");
    }
}

#[tokio::test]
async fn decision_failover_and_cross_dialect_aliases() {
    let failing = MockServer::start(FailureMode::Unavailable).await;
    let healthy = MockServer::start(FailureMode::None).await;

    let harness = Harness::start(&[
        ("prov-broken", "typesafe", &failing.base_url()),
        ("prov-typesafe", "typesafe", &healthy.base_url()),
    ])
    .await;

    harness
        .add_rule("dec-alias", "prov-broken", "safety-1", 0)
        .await;
    harness
        .add_rule("dec-alias", "prov-typesafe", "safety-1", 10)
        .await;

    // TypeSafe ingress routed onto a TypeSafe provider after the OpenAI primary fails.
    let response = harness
        .post_json("/typesafe/v1/systemone", typesafe_body("dec-alias"))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-ferrogate-fallbacks")
            .and_then(|value| value.to_str().ok()),
        Some("1")
    );

    let body: Value = response.json().await.unwrap();
    // The unified value (false) is re-encoded for TypeSafe as a probability.
    assert_eq!(body["answers"]["harmful"]["type"], "noul");

    let log = harness.await_log("dec-alias").await;
    assert_eq!(log["fallback_count"], 1);
    assert_eq!(log["routed_provider_id"], "prov-typesafe");
}

#[tokio::test]
async fn malformed_decision_bodies_are_rejected() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;

    let response = harness.post_json("/v1/decisions", json!({})).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(mock.openai_calls(), 0);

    let response = harness
        .post_json("/v1beta/decisions", json!({ "model": 3 }))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn typesafe_model_listing() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("ts-model", "prov-openai", "gpt-4o-mini", 0)
        .await;

    let response = harness.get("/typesafe/v1/models").await;
    assert_eq!(response.status(), StatusCode::OK);

    let body: Value = response.json().await.unwrap();
    let names: Vec<&str> = body["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .collect();

    assert!(names.contains(&"ts-model"), "names={names:?}");
    assert_eq!(body["models"][0]["description"], "");
    assert_eq!(body["models"][0]["release_date"], "");
}

#[tokio::test]
async fn typesafe_listing_respects_the_allow_list() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("ts-model", "prov-openai", "gpt-4o-mini", 0)
        .await;

    let restricted_raw = "sk-fg-test-ts-restricted-01";
    virtual_key::ActiveModel {
        id: Set("key-ts-restricted".to_string()),
        name: Set("TS Restricted".to_string()),
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

    let response = reqwest::Client::new()
        .get(format!("{}/typesafe/v1/models", harness.base_url))
        .bearer_auth(restricted_raw)
        .send()
        .await
        .expect("typesafe models");
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    let names: Vec<&str> = body["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .collect();
    assert!(
        !names.contains(&"ts-model"),
        "restricted key must not enumerate it: {names:?}"
    );
}
