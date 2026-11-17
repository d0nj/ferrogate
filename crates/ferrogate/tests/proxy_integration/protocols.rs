//! Responses, Messages, embeddings, routing suffixes and response headers.

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::support::{chat_body, collect_sse, concat_deltas, FailureMode, Harness, MockServer};

#[tokio::test]
async fn responses_non_streaming_and_streaming() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("resp-alias", "prov-openai", "gpt-4o-mini", 0)
        .await;

    let response = harness
        .post_json(
            "/v1/responses",
            json!({
                "model": "resp-alias",
                "instructions": "Be brief.",
                "input": "Say hello",
                "max_output_tokens": 64
            }),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    let body: Value = response.json().await.unwrap();
    assert_eq!(body["object"], "response");
    assert_eq!(body["status"], "completed");
    assert_eq!(body["output"][0]["type"], "message");
    assert_eq!(body["output"][0]["role"], "assistant");
    assert_eq!(body["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(body["output"][0]["content"][0]["text"], "hello from openai");
    assert!(body["usage"]["input_tokens"].as_i64().unwrap() > 0);

    let streamed = harness
        .post_json(
            "/v1/responses",
            json!({ "model": "resp-alias", "input": "Say hello", "stream": true }),
        )
        .await;
    let (names, frames, content_type) = collect_sse(streamed).await;

    assert!(
        content_type.starts_with("text/event-stream"),
        "{content_type}"
    );
    assert_eq!(names.first().map(String::as_str), Some("response.created"));
    assert!(names
        .iter()
        .any(|name| name == "response.output_text.delta"));
    assert_eq!(names.last().map(String::as_str), Some("response.completed"));

    let text: String = frames
        .iter()
        .filter_map(|frame| serde_json::from_str::<Value>(frame).ok())
        .filter(|value| value["type"] == "response.output_text.delta")
        .filter_map(|value| value["delta"].as_str().map(str::to_string))
        .collect();
    assert_eq!(text, "hello from openai");
}

#[tokio::test]
async fn messages_streaming_and_non_streaming_with_api_key_auth() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-anthropic", "anthropic", &mock.base_url())]).await;
    harness
        .add_rule("msg-alias", "prov-anthropic", "claude-3-5-haiku-latest", 0)
        .await;

    let body = json!({
        "model": "msg-alias",
        "max_tokens": 128,
        "system": "Be brief.",
        "messages": [{ "role": "user", "content": "Say hello" }]
    });

    let response = harness.post_json("/v1/messages", body).await;
    assert_eq!(response.status(), StatusCode::OK);

    let parsed: Value = response.json().await.unwrap();
    assert_eq!(parsed["type"], "message");
    assert_eq!(parsed["role"], "assistant");
    assert_eq!(parsed["model"], "msg-alias");
    assert_eq!(parsed["content"][0]["type"], "text");
    assert_eq!(parsed["content"][0]["text"], "hello from anthropic");
    assert_eq!(parsed["stop_reason"], "end_turn");
    assert!(parsed["usage"]["input_tokens"].as_i64().unwrap() > 0);

    let streamed = harness
        .post_json(
            "/v1/messages",
            json!({
                "model": "msg-alias",
                "max_tokens": 128,
                "messages": [{ "role": "user", "content": "Say hello" }],
                "stream": true
            }),
        )
        .await;
    let (names, frames, _) = collect_sse(streamed).await;

    assert_eq!(names.first().map(String::as_str), Some("message_start"));
    assert!(names.iter().any(|name| name == "content_block_delta"));
    assert_eq!(names.last().map(String::as_str), Some("message_stop"));

    let text: String = frames
        .iter()
        .filter_map(|frame| serde_json::from_str::<Value>(frame).ok())
        .filter(|value| value["type"] == "content_block_delta")
        .filter_map(|value| value["delta"]["text"].as_str().map(str::to_string))
        .collect();
    assert_eq!(text, "hello from anthropic");
}

#[tokio::test]
async fn embeddings_are_routed_and_priced() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("embed-alias", "prov-openai", "gpt-4o-mini", 0)
        .await;

    let response = harness
        .post_json(
            "/v1/embeddings",
            json!({ "model": "embed-alias", "input": ["one", "two"] }),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    let body: Value = response.json().await.unwrap();
    assert_eq!(body["object"], "list");
    assert_eq!(body["model"], "gpt-4o-mini");
    assert_eq!(body["data"].as_array().unwrap().len(), 2);
    assert_eq!(body["data"][1]["index"], 1);

    let log = harness.await_log("embed-alias").await;
    assert_eq!(log["status_code"], 200);
    assert!(log["input_tokens"].as_i64().unwrap() > 0);

    let rejected = harness
        .post_json(
            "/v1/embeddings",
            json!({ "model": "embed-alias", "input": "" }),
        )
        .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn routing_suffixes_and_gateway_headers() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("suffix-alias", "prov-openai", "gpt-4o-mini", 0)
        .await;
    harness.mark_zdr("prov-openai").await;

    for model in [
        "suffix-alias:floor",
        "suffix-alias:zdr",
        "mock-prov-openai/suffix-alias",
    ] {
        let response = harness
            .post_chat_with_headers(
                chat_body(model, false, None),
                &[("x-request-id", "req-abc")],
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK, "{model}");

        assert_eq!(
            response
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok()),
            Some("req-abc"),
            "{model}"
        );
        assert_eq!(
            response
                .headers()
                .get("x-ferrogate-model")
                .and_then(|value| value.to_str().ok()),
            Some("gpt-4o-mini"),
            "{model}"
        );
        assert!(response.headers().get("x-ferrogate-cost-usd").is_some());
    }
}

#[tokio::test]
async fn rate_limit_headers_and_streaming_headers() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("hdr-alias", "prov-openai", "gpt-4o-mini", 0)
        .await;

    let response = harness
        .post_chat_with_headers(
            chat_body("hdr-alias", false, None),
            &[("x-request-id", "req-h1")],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();

    // The demo key is uncapped in the harness, so limits are simply absent.
    assert_eq!(headers.get("x-ratelimit-limit-requests"), None);
    assert_eq!(
        headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("req-h1")
    );
    assert_eq!(
        headers
            .get("x-ferrogate-fallbacks")
            .and_then(|value| value.to_str().ok()),
        Some("0")
    );

    let streamed = harness.post_chat(chat_body("hdr-alias", true, None)).await;
    assert_eq!(
        streamed
            .headers()
            .get("x-ferrogate-provider")
            .and_then(|value| value.to_str().ok()),
        Some("mock-prov-openai")
    );
    assert!(streamed.headers().get("x-request-id").is_some());
    let (_, frames, _) = collect_sse(streamed).await;
    assert_eq!(concat_deltas(&frames), "hello from openai");
    assert_eq!(frames.last().map(String::as_str), Some("[DONE]"));
}
