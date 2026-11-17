mod content;
mod request;
mod response;
mod stream;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_stream;

use eventsource_stream::Eventsource;
use serde_json::Value;

use crate::providers::{
    join_url, BoxFuture, ChatCompletionRequest, ChatCompletionResponse, ProviderEndpoint,
    ProviderEventStream, ProviderKind, UpstreamCallError, UpstreamProvider,
};

pub(crate) use content::{
    stop_sequences, translate_content, translate_response_format, translate_tool_choice,
    translate_tools,
};
pub(crate) use request::build_request_body;
pub(crate) use response::{map_stop_reason, translate_response};
pub(crate) use stream::{translate_stream, StreamTranslator};

pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub(crate) const DEFAULT_MAX_TOKENS: u32 = 4096;

#[derive(Debug, Default, Clone, Copy)]
pub struct AnthropicProvider;

pub fn messages_url(base_url: &str) -> String {
    let base = super::strip_trailing_segment(base_url, "/v1");
    join_url(base, "/v1/messages")
}

async fn read_error_body(resp: reqwest::Response) -> String {
    resp.text()
        .await
        .unwrap_or_else(|err| format!("<unreadable body: {err}>"))
}

impl UpstreamProvider for AnthropicProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }

    fn complete<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a ChatCompletionRequest,
    ) -> BoxFuture<'a, Result<ChatCompletionResponse, UpstreamCallError>> {
        Box::pin(async move {
            let url = messages_url(&endpoint.base_url);
            let body = build_request_body(req, &endpoint.upstream_model);

            let resp = client
                .post(&url)
                .timeout(std::time::Duration::from_millis(endpoint.timeout_ms.max(1)))
                .header("x-api-key", &endpoint.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json");

            let resp = super::support::apply_extra_headers(resp, endpoint)
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
                UpstreamCallError::Protocol(format!("invalid Anthropic body: {err}"))
            })?;

            translate_response(&value)
        })
    }

    fn stream<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a ChatCompletionRequest,
    ) -> BoxFuture<'a, Result<ProviderEventStream, UpstreamCallError>> {
        Box::pin(async move {
            let url = messages_url(&endpoint.base_url);
            let body = build_request_body(req, &endpoint.upstream_model);

            let resp = client
                .post(&url)
                .timeout(std::time::Duration::from_millis(endpoint.timeout_ms.max(1)))
                .header("x-api-key", &endpoint.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .header("accept", "text/event-stream");

            let resp = super::support::apply_extra_headers(resp, endpoint)
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

            let events =
                translate_stream(StreamTranslator::new(), resp.bytes_stream().eventsource());

            let stream: ProviderEventStream = Box::pin(events);
            Ok(stream)
        })
    }
}
