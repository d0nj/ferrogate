//! Non-streaming completion tests across all three provider dialects.

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::support::{chat_body, FailureMode, Harness, MockServer};

#[tokio::test]
async fn non_streaming_completion_translates_every_provider() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[
        ("prov-openai", "openai", &mock.base_url()),
        ("prov-anthropic", "anthropic", &mock.base_url()),
        ("prov-gemini", "gemini", &mock.base_url()),
    ])
    .await;

    for (alias, provider, model, expected_text) in [
        (
            "alias-openai",
            "prov-openai",
            "gpt-4o-mini",
            "hello from openai",
        ),
        (
            "alias-anthropic",
            "prov-anthropic",
            "claude-3-5-haiku-latest",
            "hello from anthropic",
        ),
        (
            "alias-gemini",
            "prov-gemini",
            "gemini-2.5-flash",
            "hello from gemini",
        ),
    ] {
        harness.add_rule(alias, provider, model, 0).await;

        let response = harness.post_chat(chat_body(alias, false, None)).await;
        assert_eq!(response.status(), StatusCode::OK, "alias {alias} failed");

        let headers = response.headers().clone();
        let body: Value = response.json().await.unwrap();

        assert_eq!(body["object"], "chat.completion", "alias {alias}");
        assert_eq!(
            body["choices"][0]["message"]["content"],
            json!(expected_text),
            "alias {alias} content"
        );
        assert_eq!(body["choices"][0]["message"]["role"], "assistant");
        assert!(body["usage"]["prompt_tokens"].as_u64().unwrap() > 0);
        assert_eq!(
            headers
                .get("x-ferrogate-provider")
                .and_then(|value| value.to_str().ok()),
            Some(format!("mock-{provider}").as_str())
        );
        assert_eq!(
            headers
                .get("x-ferrogate-model")
                .and_then(|value| value.to_str().ok()),
            Some(model)
        );

        // Telemetry is written asynchronously by the mpsc worker.
        let log = harness.await_log(alias).await;
        assert_eq!(log["status_code"], 200);
        assert!(log["input_tokens"].as_i64().unwrap() > 0);
        assert!(log["output_tokens"].as_i64().unwrap() > 0);
        assert_eq!(log["routed_provider_id"], provider);
        assert_eq!(log["cache_hit"], 0);
    }
}
