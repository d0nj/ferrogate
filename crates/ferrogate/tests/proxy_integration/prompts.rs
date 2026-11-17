//! Prompt template data-plane tests: rendering, defaults and unknown slugs.

use axum::http::StatusCode;
use sea_orm::{ActiveModelTrait, Set};
use serde_json::{json, Value};

use ferrogate::entities::prompt_template;

use crate::support::{FailureMode, Harness, MockServer};

async fn insert_template(harness: &Harness, slug: &str, is_active: bool) {
    prompt_template::ActiveModel {
        id: Set(format!("prompt-{slug}")),
        slug: Set(slug.to_owned()),
        version: Set(1),
        name: Set(slug.to_owned()),
        system_prompt: Set("You are an expert on {{topic}}.".to_owned()),
        user_template: Set("Explain {{ topic }} for {{audience}}".to_owned()),
        model: Set(Some("prompt-default-model".to_owned())),
        temperature: Set(Some(0.2)),
        max_tokens: Set(Some(256)),
        variables_json: Set(
            r#"[{"name":"topic"},{"name":"audience","default":"beginners"}]"#.to_owned(),
        ),
        is_active: Set(i64::from(is_active)),
        created_at: Set("2026-01-01T00:00:00Z".to_owned()),
        updated_at: Set("2026-01-01T00:00:00Z".to_owned()),
    }
    .insert(&harness.db)
    .await
    .unwrap();
}

#[tokio::test]
async fn template_replaces_client_messages_and_supplies_defaults() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("alias-templated", "prov-openai", "gpt-4o-mini", 0)
        .await;
    insert_template(&harness, "explainer", true).await;

    let response = harness
        .post_chat(json!({
            "model": "alias-templated",
            "messages": [{ "role": "user", "content": "client text must be ignored" }],
            "template": "explainer",
            "template_variables": { "topic": "rust" },
        }))
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    let sent = mock.last_openai_body();
    let messages = sent["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2, "system + user only: {messages:?}");
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "You are an expert on rust.");
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[1]["content"], "Explain rust for beginners");
    assert_eq!(sent["temperature"], json!(0.2));
    assert_eq!(sent["max_tokens"], json!(256));
}

#[tokio::test]
async fn explicit_values_win_over_template_defaults() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("alias-templated", "prov-openai", "gpt-4o-mini", 0)
        .await;
    insert_template(&harness, "explainer", true).await;

    let response = harness
        .post_chat(json!({
            "model": "alias-templated",
            "messages": [],
            "template": "explainer",
            "template_variables": { "topic": "async", "audience": "experts" },
            "temperature": 0.9,
            "max_tokens": 64,
        }))
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    let sent = mock.last_openai_body();
    assert_eq!(sent["temperature"], json!(0.9));
    assert_eq!(sent["max_tokens"], json!(64));
    assert_eq!(
        sent["messages"][1]["content"],
        json!("Explain async for experts")
    );
}

#[tokio::test]
async fn template_model_fills_an_omitted_model() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("prompt-default-model", "prov-openai", "gpt-4o-mini", 0)
        .await;
    insert_template(&harness, "explainer", true).await;

    let response = harness
        .post_chat(json!({
            "model": "",
            "messages": [],
            "template": "explainer",
            "template_variables": { "topic": "lifetimes" },
        }))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(mock.openai_calls(), 1, "template model routed the request");
}

#[tokio::test]
async fn unknown_template_is_not_found() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("alias-templated", "prov-openai", "gpt-4o-mini", 0)
        .await;

    let response = harness
        .post_chat(json!({
            "model": "alias-templated",
            "messages": [],
            "template": "missing-slug",
        }))
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body: Value = response.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("missing-slug"),
        "unexpected body: {body}"
    );
    assert_eq!(mock.openai_calls(), 0, "no upstream call on a bad slug");
}
