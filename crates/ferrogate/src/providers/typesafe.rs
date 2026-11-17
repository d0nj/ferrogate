use serde_json::Value;

use crate::providers::{
    decisions::{
        from_typesafe_response, to_typesafe_body, UnifiedDecisionRequest, UnifiedDecisionResponse,
    },
    join_url, BoxFuture, ChatCompletionRequest, ChatCompletionResponse, EmbeddingRequest,
    EmbeddingResponse, ProviderEndpoint, ProviderEventStream, ProviderKind, UpstreamCallError,
    UpstreamProvider,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct TypesafeProvider;

pub(crate) const DANGER_ZONE_HEADER: &str = "TypeSafe-Danger-Zone-Images";

pub(crate) fn systemone_url(base_url: &str) -> String {
    let base = super::strip_trailing_segment(base_url, "/v1");
    join_url(base, "/v1/systemone")
}

pub(crate) fn needs_danger_zone(req: &UnifiedDecisionRequest) -> bool {
    req.needs_image_processing()
}

fn unsupported() -> UpstreamCallError {
    UpstreamCallError::Status {
        status: 400,
        body: "{\"error\":{\"message\":\"typesafe provider only supports /v1/decisions and /v1/systemone\",\"type\":\"invalid_request_error\"}}"
            .to_string(),
    }
}

async fn read_error_body(resp: reqwest::Response) -> String {
    resp.text()
        .await
        .unwrap_or_else(|err| format!("<unreadable body: {err}>"))
}

impl UpstreamProvider for TypesafeProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Typesafe
    }

    fn complete<'a>(
        &'a self,
        _client: &'a reqwest::Client,
        _endpoint: &'a ProviderEndpoint,
        _req: &'a ChatCompletionRequest,
    ) -> BoxFuture<'a, Result<ChatCompletionResponse, UpstreamCallError>> {
        Box::pin(async move { Err(unsupported()) })
    }

    fn stream<'a>(
        &'a self,
        _client: &'a reqwest::Client,
        _endpoint: &'a ProviderEndpoint,
        _req: &'a ChatCompletionRequest,
    ) -> BoxFuture<'a, Result<ProviderEventStream, UpstreamCallError>> {
        Box::pin(async move { Err(unsupported()) })
    }

    fn embed<'a>(
        &'a self,
        _client: &'a reqwest::Client,
        _endpoint: &'a ProviderEndpoint,
        _req: &'a EmbeddingRequest,
    ) -> BoxFuture<'a, Result<EmbeddingResponse, UpstreamCallError>> {
        Box::pin(async move { Err(unsupported()) })
    }

    fn decide<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a UnifiedDecisionRequest,
    ) -> BoxFuture<'a, Result<UnifiedDecisionResponse, UpstreamCallError>> {
        Box::pin(async move {
            let url = systemone_url(&endpoint.base_url);
            let body = to_typesafe_body(req);

            let mut builder = client
                .post(&url)
                .timeout(std::time::Duration::from_millis(endpoint.timeout_ms.max(1)))
                .header("content-type", "application/json");

            if !endpoint.api_key.is_empty() {
                builder = builder.bearer_auth(&endpoint.api_key);
            }
            if needs_danger_zone(req) {
                builder = builder.header(DANGER_ZONE_HEADER, "true");
            }
            builder = super::support::apply_extra_headers(builder, endpoint);

            let resp = builder
                .json(&body)
                .send()
                .await
                .map_err(|err| UpstreamCallError::Transport(format!("POST {url}: {err}")))?;

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                return Err(UpstreamCallError::Status {
                    status,
                    body: read_error_body(resp).await,
                });
            }

            let value: Value = resp.json().await.map_err(|err| {
                UpstreamCallError::Protocol(format!("invalid TypeSafe body: {err}"))
            })?;

            from_typesafe_response(&value, &endpoint.upstream_model).map_err(|message| {
                UpstreamCallError::Protocol(format!("invalid TypeSafe response: {message}"))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::decisions::{DecisionAsset, DecisionQuestion};

    #[test]
    fn url_does_not_duplicate_v1() {
        assert_eq!(
            systemone_url("https://api.typesafe.example"),
            "https://api.typesafe.example/v1/systemone"
        );
        assert_eq!(
            systemone_url("https://api.typesafe.example/v1"),
            "https://api.typesafe.example/v1/systemone"
        );
        assert_eq!(
            systemone_url("https://api.typesafe.example/"),
            "https://api.typesafe.example/v1/systemone"
        );
    }

    #[test]
    fn danger_zone_header_tracks_assets_and_box_questions() {
        let plain = UnifiedDecisionRequest::new("m").with_text("hello");
        assert!(!needs_danger_zone(&plain));

        let with_image = plain
            .clone()
            .with_asset(DecisionAsset::new("image", "data:image/png;base64,AA"));
        assert!(needs_danger_zone(&with_image));

        let with_box = plain.with_question(DecisionQuestion::boxed("locate"));
        assert!(needs_danger_zone(&with_box));
    }

    #[tokio::test]
    async fn chat_surfaces_are_rejected() {
        let provider = TypesafeProvider;
        let client = crate::providers::build_http_client();
        let endpoint = ProviderEndpoint {
            id: "p".into(),
            name: "typesafe".into(),
            kind: ProviderKind::Typesafe,
            dialect: crate::providers::ApiDialect::Chat,
            base_url: "https://api.typesafe.example".into(),
            api_key: "k".into(),
            upstream_model: "m".into(),
            timeout_ms: 10,
            extra_headers: Default::default(),
            zdr: false,
            region: None,
            selected_key_id: None,
            selected_key_name: None,
        };
        let request = ChatCompletionRequest {
            model: "m".into(),
            messages: Vec::new(),
            temperature: None,
            top_p: None,
            max_tokens: None,
            max_completion_tokens: None,
            stream: None,
            stream_options: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            user: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            seed: None,
            reasoning_effort: None,
            template: None,
            template_version: None,
            template_variables: None,
        };

        match provider.complete(&client, &endpoint, &request).await {
            Err(UpstreamCallError::Status { status, body }) => {
                assert_eq!(status, 400);
                assert!(body.contains("/v1/systemone"), "got {body}");
            }
            other => panic!("unexpected: {other:?}"),
        }

        match provider.stream(&client, &endpoint, &request).await {
            Err(UpstreamCallError::Status { status, .. }) => assert_eq!(status, 400),
            Ok(_) => panic!("typesafe streaming must not succeed"),
            Err(other) => panic!("unexpected: {other:?}"),
        }

        let embed = EmbeddingRequest {
            model: "m".into(),
            input: serde_json::json!("hi"),
            encoding_format: None,
            dimensions: None,
            user: None,
        };
        match provider.embed(&client, &endpoint, &embed).await {
            Err(UpstreamCallError::Status { status, .. }) => assert_eq!(status, 400),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
