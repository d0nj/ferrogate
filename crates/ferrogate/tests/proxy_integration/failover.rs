//! Automatic fallback when an upstream fails before the first token.

use axum::http::StatusCode;
use serde_json::Value;

use crate::support::{chat_body, collect_sse, concat_deltas, FailureMode, Harness, MockServer};

#[tokio::test]
async fn retryable_upstream_failures_fall_back_before_first_token() {
    for mode in [FailureMode::Unavailable, FailureMode::RateLimited] {
        let failing = MockServer::start(mode).await;
        let healthy = MockServer::start(FailureMode::None).await;

        let harness = Harness::start(&[
            ("prov-broken", "openai", &failing.base_url()),
            ("prov-healthy", "anthropic", &healthy.base_url()),
        ])
        .await;

        harness
            .add_rule("fb-alias", "prov-broken", "gpt-4o-mini", 0)
            .await;
        harness
            .add_rule("fb-alias", "prov-healthy", "claude-3-5-haiku-latest", 10)
            .await;

        // Non-streaming fallback.
        let response = harness.post_chat(chat_body("fb-alias", false, None)).await;
        assert_eq!(response.status(), StatusCode::OK, "mode {mode:?}");
        assert_eq!(
            response
                .headers()
                .get("x-ferrogate-fallbacks")
                .and_then(|value| value.to_str().ok()),
            Some("1")
        );
        let body: Value = response.json().await.unwrap();
        assert_eq!(
            body["choices"][0]["message"]["content"],
            "hello from anthropic"
        );
        assert_eq!(
            failing.openai_calls(),
            1,
            "primary must be tried exactly once"
        );

        let log = harness.await_log("fb-alias").await;
        assert_eq!(log["fallback_count"], 1);
        assert_eq!(log["routed_provider_id"], "prov-healthy");

        // Streaming fallback: the 503/429 arrives before any SSE byte downstream,
        // so the client sees a clean, successful stream from the secondary.
        let response = harness.post_chat(chat_body("fb-alias", true, None)).await;
        assert_eq!(response.status(), StatusCode::OK, "stream mode {mode:?}");
        let (_, frames, _) = collect_sse(response).await;
        assert_eq!(concat_deltas(&frames), "hello from anthropic");
        assert_eq!(frames.last().map(String::as_str), Some("[DONE]"));

        let log = harness.await_log("fb-alias").await;
        assert_eq!(log["fallback_count"], 1, "{log:?}");
        assert_eq!(log["status_code"], 200);
    }
}
