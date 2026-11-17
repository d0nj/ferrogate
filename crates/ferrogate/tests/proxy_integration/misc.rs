//! Model listing and request-validation behaviour.

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::support::{chat_body, FailureMode, Harness, MockServer};

#[tokio::test]
async fn models_endpoint_lists_routing_aliases() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("listed-alias", "prov-openai", "gpt-4o-mini", 0)
        .await;

    let response = harness.get_models().await;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["object"], "list");

    let ids: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["id"].as_str())
        .collect();

    assert!(ids.contains(&"listed-alias"), "ids={ids:?}");
    assert!(ids.contains(&"gpt-4o-mini"), "ids={ids:?}");
    assert_eq!(body["data"][0]["object"], "model");
    assert_eq!(body["data"][0]["owned_by"], "ferrogate");
}

#[tokio::test]
async fn invalid_requests_are_rejected_before_any_upstream_call() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("v-alias", "prov-openai", "gpt-4o-mini", 0)
        .await;

    let response = harness
        .post_chat(json!({ "model": "", "messages": [{ "role": "user", "content": "hi" }] }))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = harness
        .post_chat(json!({ "model": "v-alias", "messages": [] }))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    assert_eq!(mock.openai_calls(), 0);
}

#[tokio::test]
async fn unknown_model_reports_not_found() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;

    let response = harness
        .post_chat(chat_body("no-such-model", false, None))
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(mock.openai_calls(), 0);
}
