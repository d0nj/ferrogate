use futures_util::StreamExt;
use serde_json::{json, Value};

use super::super::support::{chunk_payload, unix_timestamp};
use super::super::types::{StreamEvent, UpstreamCallError};
use super::response::{
    candidate_text, finish_reason_for, first_candidate, parse_usage, translate_function_calls,
};

#[derive(Debug, Default)]
pub(crate) struct StreamTranslator {
    model: String,
    done: bool,
}

impl StreamTranslator {
    pub(crate) fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            done: false,
        }
    }

    pub(crate) fn translate(
        &mut self,
        data: &str,
    ) -> Option<Result<StreamEvent, UpstreamCallError>> {
        let trimmed = data.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed == "[DONE]" {
            self.done = true;
            return Some(Ok(StreamEvent::Done));
        }

        let value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(err) => {
                return Some(Err(UpstreamCallError::Protocol(format!(
                    "malformed Gemini stream frame: {err}"
                ))));
            }
        };

        if let Some(error) = value.get("error") {
            self.done = true;
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            return Some(Err(UpstreamCallError::Protocol(format!(
                "Gemini stream error: {message}"
            ))));
        }

        let candidate = first_candidate(&value);
        let text = candidate.map(candidate_text).unwrap_or_default();
        let tool_calls = translate_function_calls(candidate, unix_timestamp());
        let finish_reason = finish_reason_for(candidate, &tool_calls);
        let usage = parse_usage(value.get("usageMetadata"));

        if text.is_empty() && finish_reason.is_none() && usage.is_none() && tool_calls.is_empty() {
            return None;
        }

        let delta = if !tool_calls.is_empty() {
            json!({ "tool_calls": tool_calls })
        } else if text.is_empty() {
            json!({ "role": "assistant", "content": "" })
        } else {
            json!({ "content": text })
        };

        let mut payload = chunk_payload(
            &format!("chatcmpl-{}", unix_timestamp()),
            &self.model,
            0,
            Some(delta),
            finish_reason.as_deref(),
        );

        if let Some(usage) = &usage {
            payload["usage"] = json!({
                "prompt_tokens": usage.prompt_tokens,
                "completion_tokens": usage.completion_tokens,
                "total_tokens": usage.total_tokens,
            });
        }

        Some(Ok(StreamEvent::Chunk {
            data: payload,
            delta_text: if text.is_empty() { None } else { Some(text) },
            usage,
            finish_reason,
        }))
    }

    pub(crate) fn is_done(&self) -> bool {
        self.done
    }
}

pub(crate) fn translate_stream<S, E>(
    mut translator: StreamTranslator,
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
                    if let Some(item) = translator.translate(&event.data) {
                        let done = translator.is_done();
                        yield item;
                        if done {
                            return;
                        }
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
