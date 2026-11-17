pub mod anthropic;
pub mod decisions;
pub mod gemini;
pub mod openai;
pub(crate) mod support;
mod types;
pub mod typesafe;

use std::time::Duration;

pub(crate) use support::{
    decisions_unsupported, embeddings_unsupported, strip_trailing_segment, unix_timestamp,
};
pub use support::{join_url, message_content_to_text, rewrite_content};
pub use types::*;

use crate::providers::{
    anthropic::AnthropicProvider, decisions::UnifiedDecisionRequest,
    decisions::UnifiedDecisionResponse, gemini::GeminiProvider, openai::OpenAiProvider,
    typesafe::TypesafeProvider,
};

pub trait UpstreamProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;

    fn complete<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a ChatCompletionRequest,
    ) -> BoxFuture<'a, Result<ChatCompletionResponse, UpstreamCallError>>;

    fn stream<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a ChatCompletionRequest,
    ) -> BoxFuture<'a, Result<ProviderEventStream, UpstreamCallError>>;

    fn embed<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a EmbeddingRequest,
    ) -> BoxFuture<'a, Result<EmbeddingResponse, UpstreamCallError>> {
        let _ = (client, endpoint, req);
        Box::pin(async move { Err(embeddings_unsupported(self.kind())) })
    }

    fn decide<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a UnifiedDecisionRequest,
    ) -> BoxFuture<'a, Result<UnifiedDecisionResponse, UpstreamCallError>> {
        let _ = (client, endpoint, req);
        Box::pin(async move { Err(decisions_unsupported(self.kind())) })
    }
}

static OPENAI: OpenAiProvider = OpenAiProvider;
static ANTHROPIC: AnthropicProvider = AnthropicProvider;
static GEMINI: GeminiProvider = GeminiProvider;
static TYPESAFE: TypesafeProvider = TypesafeProvider;

pub fn build_http_client() -> reqwest::Client {
    let redirect = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > 10 {
            return attempt.error("too many redirects");
        }
        let same_origin = attempt.previous().last().is_some_and(|prev| {
            prev.scheme() == attempt.url().scheme()
                && prev.host_str() == attempt.url().host_str()
                && prev.port_or_known_default() == attempt.url().port_or_known_default()
        });
        if same_origin {
            attempt.follow()
        } else {
            tracing::warn!(url = %attempt.url(), "refusing cross-host redirect");
            attempt.stop()
        }
    });
    reqwest::Client::builder()
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(64)
        .tcp_keepalive(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .http2_adaptive_window(true)
        .http2_keep_alive_interval(Some(Duration::from_secs(15)))
        .http2_keep_alive_timeout(Duration::from_secs(10))
        .redirect(redirect)
        .build()
        .unwrap_or_else(|err| {
            tracing::error!(error = %err, "failed to build tuned HTTP client; using defaults");
            reqwest::Client::new()
        })
}

pub fn adapter_for(kind: ProviderKind) -> &'static dyn UpstreamProvider {
    match kind {
        ProviderKind::OpenAi => &OPENAI,
        ProviderKind::Anthropic => &ANTHROPIC,
        ProviderKind::Gemini => &GEMINI,
        ProviderKind::Typesafe => &TYPESAFE,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;

    #[test]
    fn provider_kind_round_trips() {
        for (raw, kind) in [
            ("openai", ProviderKind::OpenAi),
            ("  Anthropic ", ProviderKind::Anthropic),
            ("GEMINI", ProviderKind::Gemini),
            ("TypeSafe", ProviderKind::Typesafe),
            ("systemone", ProviderKind::Typesafe),
        ] {
            assert_eq!(ProviderKind::parse(raw), Some(kind));
            assert_eq!(kind.as_str(), kind.as_str().to_ascii_lowercase());
        }
        assert_eq!(ProviderKind::parse("groq"), None);
        assert_eq!(ProviderKind::OpenAi.as_str(), "openai");
        assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
        assert_eq!(ProviderKind::Gemini.as_str(), "gemini");
        assert_eq!(ProviderKind::Typesafe.as_str(), "typesafe");
    }

    #[test]
    fn adapter_for_returns_matching_kind() {
        for kind in [
            ProviderKind::OpenAi,
            ProviderKind::Anthropic,
            ProviderKind::Gemini,
            ProviderKind::Typesafe,
        ] {
            assert_eq!(adapter_for(kind).kind(), kind);
        }
    }

    #[test]
    fn message_content_flattens_strings_and_parts() {
        assert_eq!(message_content_to_text(&json!("hello")), "hello");
        assert_eq!(
            message_content_to_text(&json!([
                { "type": "text", "text": "foo" },
                { "type": "image_url", "image_url": { "url": "http://x" } },
                { "type": "text", "text": "bar" }
            ])),
            "foobar"
        );
        assert_eq!(message_content_to_text(&Value::Null), "");
        assert_eq!(message_content_to_text(&json!({ "text": "solo" })), "solo");
    }

    #[test]
    fn join_url_normalizes_slashes() {
        assert_eq!(
            join_url("https://api.openai.com/", "/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            join_url("https://api.openai.com", "v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            join_url("http://localhost:8080", ""),
            "http://localhost:8080"
        );
    }

    #[test]
    fn join_url_does_not_duplicate_version_segments() {
        assert_eq!(
            join_url(
                strip_trailing_segment("https://api.openai.com/v1/", "/v1"),
                "/v1/chat/completions"
            ),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            strip_trailing_segment("https://x.dev/v1beta", "/v1beta"),
            "https://x.dev"
        );
        assert_eq!(
            strip_trailing_segment("https://x.dev", "/v1beta"),
            "https://x.dev"
        );
    }

    #[test]
    fn error_retryability_and_status_codes() {
        let rate_limited = UpstreamCallError::Status {
            status: 429,
            body: String::new(),
        };
        let server_error = UpstreamCallError::Status {
            status: 503,
            body: String::new(),
        };
        let bad_request = UpstreamCallError::Status {
            status: 400,
            body: String::new(),
        };

        assert!(rate_limited.is_retryable());
        assert!(server_error.is_retryable());
        assert!(!bad_request.is_retryable());
        assert!(UpstreamCallError::Transport("reset".into()).is_retryable());
        assert!(!UpstreamCallError::Protocol("bad".into()).is_retryable());

        assert_eq!(rate_limited.status_code(), 429);
        assert_eq!(UpstreamCallError::Transport("x".into()).status_code(), 502);
        assert_eq!(UpstreamCallError::Protocol("x".into()).status_code(), 504);
    }
}
