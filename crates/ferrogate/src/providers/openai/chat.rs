use futures_util::StreamExt;
use serde_json::{json, Map, Value};

use super::super::join_url;
use super::super::support::strip_trailing_segment;
use super::super::types::{ChatCompletionRequest, StreamEvent, UpstreamCallError, UsageInfo};

pub(crate) fn chat_completions_url(base_url: &str) -> String {
    let base = strip_trailing_segment(base_url, "/v1");
    join_url(base, "/v1/chat/completions")
}

pub(crate) fn build_request_body(req: &ChatCompletionRequest, upstream_model: &str) -> Value {
    let mut body = serde_json::to_value(req).unwrap_or_else(|_| json!({}));

    let map = match body.as_object_mut() {
        Some(map) => map,
        None => {
            return json!({
                "model": upstream_model,
                "messages": req.messages,
            });
        }
    };

    map.insert(
        "model".to_string(),
        Value::String(upstream_model.to_string()),
    );

    if req.stream.unwrap_or(false) {
        let options = map
            .entry("stream_options")
            .or_insert_with(|| Value::Object(Map::new()));

        match options {
            Value::Object(options) => {
                options.entry("include_usage").or_insert(Value::Bool(true));
            }
            _ => *options = json!({ "include_usage": true }),
        }
    }

    body
}

pub(crate) fn parse_usage(value: &Value) -> Option<UsageInfo> {
    let usage = value.get("usage")?.as_object()?;

    let prompt_tokens = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(prompt_tokens.saturating_add(completion_tokens));

    Some(UsageInfo {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    })
}

pub(crate) fn parse_stream_frame(data: &str) -> Option<Result<StreamEvent, UpstreamCallError>> {
    let trimmed = data.trim();

    if trimmed.is_empty() {
        return None;
    }
    if trimmed == "[DONE]" {
        return Some(Ok(StreamEvent::Done));
    }

    let value: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(err) => {
            return Some(Err(UpstreamCallError::Protocol(format!(
                "malformed OpenAI stream frame: {err}"
            ))));
        }
    };

    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string());
        return Some(Err(UpstreamCallError::Protocol(message)));
    }

    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first());

    let delta_text = choice
        .and_then(|c| c.get("delta"))
        .and_then(|d| d.get("content"))
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string);

    let finish_reason = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let usage = parse_usage(&value);

    if delta_text.is_none() && finish_reason.is_none() && usage.is_none() {
        let has_delta = choice
            .and_then(|c| c.get("delta"))
            .and_then(Value::as_object)
            .is_some_and(|delta| !delta.is_empty());
        if !has_delta {
            return None;
        }
    }

    Some(Ok(StreamEvent::Chunk {
        data: value,
        delta_text,
        usage,
        finish_reason,
    }))
}

pub(crate) fn translate_stream<S, E>(
    mut events: S,
) -> impl futures_util::Stream<Item = Result<StreamEvent, UpstreamCallError>> + Send
where
    S: futures_util::Stream<
            Item = Result<eventsource_stream::Event, eventsource_stream::EventStreamError<E>>,
        > + Send
        + Unpin,
    E: std::fmt::Display + Send,
{
    async_stream::stream! {
        while let Some(event) = events.next().await {
            match event {
                Ok(event) => {
                    match parse_stream_frame(&event.data) {
                        Some(Ok(StreamEvent::Done)) => {
                            yield Ok(StreamEvent::Done);
                            return;
                        }
                        Some(item) => yield item,
                        None => {}
                    }
                }
                Err(err) => {
                    yield Err(UpstreamCallError::Transport(err.to_string()));
                    return;
                }
            }
        }

        yield Ok(StreamEvent::Done);
    }
}
