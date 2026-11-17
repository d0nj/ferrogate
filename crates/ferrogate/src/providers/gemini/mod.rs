mod embed;
mod request;
mod response;
mod stream;
#[cfg(test)]
mod tests;

pub(crate) use embed::*;
pub(crate) use request::*;
pub(crate) use response::*;
pub(crate) use stream::*;

use eventsource_stream::Eventsource;
use serde_json::Value;

use super::decisions::{
    from_gemini_response, to_gemini_body, UnifiedDecisionRequest, UnifiedDecisionResponse,
};
use super::join_url;
use super::support::{apply_extra_headers, strip_trailing_segment};
use super::types::{
    BoxFuture, ChatCompletionRequest, ChatCompletionResponse, EmbeddingRequest, EmbeddingResponse,
    ProviderEndpoint, ProviderEventStream, ProviderKind, UpstreamCallError,
};
use super::UpstreamProvider;

#[derive(Debug, Default, Clone, Copy)]
pub struct GeminiProvider;

pub(crate) fn normalize_base(base_url: &str) -> &str {
    let base = strip_trailing_segment(base_url, "/v1beta");
    strip_trailing_segment(base, "/v1")
}

pub(crate) fn generate_url(base_url: &str, model: &str) -> String {
    join_url(
        normalize_base(base_url),
        &format!("/v1beta/models/{model}:generateContent"),
    )
}

pub(crate) fn stream_url(base_url: &str, model: &str) -> String {
    join_url(
        normalize_base(base_url),
        &format!("/v1beta/models/{model}:streamGenerateContent?alt=sse"),
    )
}

async fn read_error_body(resp: reqwest::Response) -> String {
    resp.text()
        .await
        .unwrap_or_else(|err| format!("<unreadable body: {err}>"))
}

impl UpstreamProvider for GeminiProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Gemini
    }

    fn complete<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a ChatCompletionRequest,
    ) -> BoxFuture<'a, Result<ChatCompletionResponse, UpstreamCallError>> {
        Box::pin(async move {
            let url = generate_url(&endpoint.base_url, &endpoint.upstream_model);
            let body = build_request_body(req);

            let resp = client
                .post(&url)
                .timeout(std::time::Duration::from_millis(endpoint.timeout_ms.max(1)))
                .header("x-goog-api-key", &endpoint.api_key)
                .header("content-type", "application/json");

            let resp = apply_extra_headers(resp, endpoint)
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
                UpstreamCallError::Protocol(format!("invalid Gemini body: {err}"))
            })?;

            translate_response(&value, &endpoint.upstream_model)
        })
    }

    fn stream<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a ChatCompletionRequest,
    ) -> BoxFuture<'a, Result<ProviderEventStream, UpstreamCallError>> {
        Box::pin(async move {
            let url = stream_url(&endpoint.base_url, &endpoint.upstream_model);
            let body = build_request_body(req);

            let resp = client
                .post(&url)
                .timeout(std::time::Duration::from_millis(endpoint.timeout_ms.max(1)))
                .header("x-goog-api-key", &endpoint.api_key)
                .header("content-type", "application/json")
                .header("accept", "text/event-stream");

            let resp = apply_extra_headers(resp, endpoint)
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

            let stream = translate_stream(
                StreamTranslator::new(endpoint.upstream_model.clone()),
                resp.bytes_stream().eventsource(),
            );

            Ok(Box::pin(stream) as ProviderEventStream)
        })
    }

    fn embed<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a EmbeddingRequest,
    ) -> BoxFuture<'a, Result<EmbeddingResponse, UpstreamCallError>> {
        Box::pin(async move {
            let inputs = embedding_inputs(&req.input)?;
            let batch = inputs.len() > 1;
            let url = embedding_url(&endpoint.base_url, &endpoint.upstream_model, batch);
            let body = build_embedding_body(&endpoint.upstream_model, &inputs, req, batch);

            let builder = client
                .post(&url)
                .timeout(std::time::Duration::from_millis(endpoint.timeout_ms.max(1)))
                .header("x-goog-api-key", &endpoint.api_key)
                .header("content-type", "application/json");

            let resp = apply_extra_headers(builder, endpoint)
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
                UpstreamCallError::Protocol(format!("invalid Gemini embedding body: {err}"))
            })?;

            parse_embedding_response(&value, &endpoint.upstream_model)
        })
    }

    fn decide<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a UnifiedDecisionRequest,
    ) -> BoxFuture<'a, Result<UnifiedDecisionResponse, UpstreamCallError>> {
        Box::pin(async move {
            let url = decisions_url(&endpoint.base_url);
            let body = to_gemini_body(req);

            let builder = client
                .post(&url)
                .timeout(std::time::Duration::from_millis(endpoint.timeout_ms.max(1)))
                .header("x-goog-api-key", &endpoint.api_key)
                .header("content-type", "application/json");

            let resp = apply_extra_headers(builder, endpoint)
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
                UpstreamCallError::Protocol(format!("invalid Gemini decisions body: {err}"))
            })?;

            from_gemini_response(&value, &endpoint.upstream_model).map_err(|message| {
                UpstreamCallError::Protocol(format!("invalid Gemini decisions response: {message}"))
            })
        })
    }
}
