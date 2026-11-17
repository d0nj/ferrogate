mod chat;
mod embed;
mod responses;
#[cfg(test)]
mod tests;

pub(crate) use chat::*;
pub(crate) use embed::*;

use eventsource_stream::Eventsource;
use serde_json::Value;

use super::decisions::{
    from_openai_response, to_openai_body, UnifiedDecisionRequest, UnifiedDecisionResponse,
};
use super::support::apply_extra_headers;
use super::types::{
    ApiDialect, BoxFuture, ChatCompletionRequest, ChatCompletionResponse, EmbeddingRequest,
    EmbeddingResponse, ProviderEndpoint, ProviderEventStream, ProviderKind, UpstreamCallError,
};
use super::UpstreamProvider;

#[derive(Debug, Default, Clone, Copy)]
pub struct OpenAiProvider;

async fn read_error_body(resp: reqwest::Response) -> String {
    const MAX_ERROR_BODY: usize = 8 * 1024;

    match resp.text().await {
        Ok(body) if body.len() > MAX_ERROR_BODY => {
            let mut cut = MAX_ERROR_BODY;
            while cut > 0 && !body.is_char_boundary(cut) {
                cut -= 1;
            }
            format!("{}…", &body[..cut])
        }
        Ok(body) => body,
        Err(err) => format!("<unreadable body: {err}>"),
    }
}

async fn send(
    builder: reqwest::RequestBuilder,
    url: &str,
) -> Result<reqwest::Response, UpstreamCallError> {
    let resp = builder
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

    Ok(resp)
}

impl UpstreamProvider for OpenAiProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAi
    }

    fn complete<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a ChatCompletionRequest,
    ) -> BoxFuture<'a, Result<ChatCompletionResponse, UpstreamCallError>> {
        Box::pin(async move {
            let responses = endpoint.dialect == ApiDialect::Responses;
            let (url, body) = if responses {
                (
                    responses::responses_url(&endpoint.base_url),
                    responses::build_request_body(req, &endpoint.upstream_model),
                )
            } else {
                (
                    chat_completions_url(&endpoint.base_url),
                    build_request_body(req, &endpoint.upstream_model),
                )
            };

            let mut builder = client
                .post(&url)
                .timeout(std::time::Duration::from_millis(endpoint.timeout_ms.max(1)))
                .json(&body);

            if !endpoint.api_key.is_empty() {
                builder = builder.bearer_auth(&endpoint.api_key);
            }
            builder = apply_extra_headers(builder, endpoint);

            let value: Value = send(builder, &url).await?.json().await.map_err(|err| {
                UpstreamCallError::Protocol(format!("invalid completion body: {err}"))
            })?;

            if responses {
                return responses::parse_response(&value, &endpoint.upstream_model);
            }
            serde_json::from_value(value).map_err(|err| {
                UpstreamCallError::Protocol(format!("invalid completion body: {err}"))
            })
        })
    }

    fn stream<'a>(
        &'a self,
        client: &'a reqwest::Client,
        endpoint: &'a ProviderEndpoint,
        req: &'a ChatCompletionRequest,
    ) -> BoxFuture<'a, Result<ProviderEventStream, UpstreamCallError>> {
        Box::pin(async move {
            let responses = endpoint.dialect == ApiDialect::Responses;
            let (url, body) = if responses {
                (
                    responses::responses_url(&endpoint.base_url),
                    responses::build_request_body(req, &endpoint.upstream_model),
                )
            } else {
                (
                    chat_completions_url(&endpoint.base_url),
                    build_request_body(req, &endpoint.upstream_model),
                )
            };

            let mut builder = client
                .post(&url)
                .timeout(std::time::Duration::from_millis(endpoint.timeout_ms.max(1)))
                .header("accept", "text/event-stream")
                .json(&body);

            if !endpoint.api_key.is_empty() {
                builder = builder.bearer_auth(&endpoint.api_key);
            }
            builder = apply_extra_headers(builder, endpoint);

            let resp = send(builder, &url).await?;

            let events = resp.bytes_stream().eventsource();
            if responses {
                let stream = responses::translate_stream(events);
                return Ok(Box::pin(stream) as ProviderEventStream);
            }
            let stream = translate_stream(events);
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
            if endpoint.dialect == ApiDialect::Responses {
                return Err(UpstreamCallError::Status {
                    status: 400,
                    body: serde_json::json!({
                        "error": {
                            "message": "this provider uses the Responses API, which has no embeddings endpoint",
                            "type": "invalid_request_error",
                        }
                    })
                    .to_string(),
                });
            }
            let url = embeddings_url(&endpoint.base_url);
            let body = build_embedding_body(req, &endpoint.upstream_model);

            let mut builder = client
                .post(&url)
                .timeout(std::time::Duration::from_millis(endpoint.timeout_ms.max(1)))
                .json(&body);

            if !endpoint.api_key.is_empty() {
                builder = builder.bearer_auth(&endpoint.api_key);
            }
            builder = apply_extra_headers(builder, endpoint);

            let value: Value = send(builder, &url).await?.json().await.map_err(|err| {
                UpstreamCallError::Protocol(format!("invalid embedding body: {err}"))
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
            let body = to_openai_body(req);

            let mut builder = client
                .post(&url)
                .timeout(std::time::Duration::from_millis(endpoint.timeout_ms.max(1)))
                .header("content-type", "application/json");

            if !endpoint.api_key.is_empty() {
                builder = builder.bearer_auth(&endpoint.api_key);
            }
            builder = apply_extra_headers(builder, endpoint);

            let value: Value = send(builder.json(&body), &url)
                .await?
                .json()
                .await
                .map_err(|err| {
                    UpstreamCallError::Protocol(format!("invalid decisions body: {err}"))
                })?;

            from_openai_response(&value, &endpoint.upstream_model).map_err(|message| {
                UpstreamCallError::Protocol(format!("invalid decisions response: {message}"))
            })
        })
    }
}
