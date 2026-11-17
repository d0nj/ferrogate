//! Zero-buffer SSE streaming tests plus TTFT/token telemetry assertions.

use crate::support::{chat_body, collect_sse, concat_deltas, FailureMode, Harness, MockServer};

#[tokio::test]
async fn streaming_forwards_chunks_and_persists_ttft() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[
        ("prov-openai", "openai", &mock.base_url()),
        ("prov-anthropic", "anthropic", &mock.base_url()),
        ("prov-gemini", "gemini", &mock.base_url()),
    ])
    .await;

    for (alias, provider, model, expected_text) in [
        (
            "s-openai",
            "prov-openai",
            "gpt-4o-mini",
            "hello from openai",
        ),
        (
            "s-anthropic",
            "prov-anthropic",
            "claude-3-5-haiku-latest",
            "hello from anthropic",
        ),
        (
            "s-gemini",
            "prov-gemini",
            "gemini-2.5-flash",
            "hello from gemini",
        ),
    ] {
        harness.add_rule(alias, provider, model, 0).await;

        let response = harness.post_chat(chat_body(alias, true, None)).await;
        let (_, frames, content_type) = collect_sse(response).await;

        assert!(
            content_type.starts_with("text/event-stream"),
            "alias {alias} content-type was {content_type}"
        );
        assert_eq!(
            frames.last().map(String::as_str),
            Some("[DONE]"),
            "alias {alias} must terminate with [DONE]; frames={frames:?}"
        );
        assert_eq!(concat_deltas(&frames), expected_text, "alias {alias}");

        let log = harness.await_log(alias).await;
        assert_eq!(log["status_code"], 200);
        assert_eq!(log["is_streaming"], 1);
        assert_eq!(log["cache_hit"], 0);
        assert!(
            log["ttft_ms"].as_f64().unwrap_or(0.0) > 0.0,
            "ttft_ms must be latched for {alias}: {log:?}"
        );
        assert!(log["input_tokens"].as_i64().unwrap() > 0, "{log:?}");
        assert!(log["output_tokens"].as_i64().unwrap() > 0, "{log:?}");
        assert!(
            log["cost_usd"].as_f64().unwrap_or(0.0) > 0.0,
            "cost_usd must be priced for {alias}: {log:?}"
        );
        assert!(
            log["response_text"]
                .as_str()
                .is_some_and(|text| text.contains("hello")),
            "response_text must capture the streamed answer: {log:?}"
        );
    }
}
