use std::collections::HashMap;

use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::providers::{support, StreamEvent, UpstreamCallError, UsageInfo};

use super::map_stop_reason;

#[derive(Debug, Default)]
pub(crate) struct StreamTranslator {
    input_tokens: u32,
    message_id: Option<String>,
    model: Option<String>,
    done: bool,
    tool_indices: HashMap<u64, u32>,
}

impl StreamTranslator {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn is_done(&self) -> bool {
        self.done
    }

    pub(crate) fn translate(
        &mut self,
        event_name: &str,
        data: &str,
    ) -> Option<Result<StreamEvent, UpstreamCallError>> {
        let trimmed = data.trim();
        if trimmed.is_empty() {
            return None;
        }

        let value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(err) => {
                return Some(Err(UpstreamCallError::Protocol(format!(
                    "malformed Anthropic stream event: {err}"
                ))));
            }
        };

        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(event_name);

        match kind {
            "message_start" => self.message_start(&value),
            "content_block_start" => self.content_block_start(&value),
            "content_block_delta" => self.content_block_delta(&value),
            "message_delta" => self.message_delta(&value),
            "message_stop" => {
                self.done = true;
                Some(Ok(StreamEvent::Done))
            }
            "error" => {
                self.done = true;
                let error = value.get("error").unwrap_or(&Value::Null);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| error.to_string());
                Some(Err(UpstreamCallError::Protocol(format!(
                    "Anthropic stream error: {message}"
                ))))
            }
            _ => None,
        }
    }

    fn tool_index(&mut self, block_index: u64) -> u32 {
        let next = self.tool_indices.len() as u32;
        *self.tool_indices.entry(block_index).or_insert(next)
    }

    fn chunk(&self, delta: Value, finish_reason: Option<&str>) -> Value {
        let id = self.message_id.as_deref().unwrap_or("anthropic-message");
        let model = self.model.as_deref().unwrap_or_default();
        support::chunk_payload(id, model, 0, Some(delta), finish_reason)
    }

    fn emit(&self, data: Value, delta_text: Option<String>) -> StreamEvent {
        StreamEvent::Chunk {
            data,
            delta_text,
            usage: None,
            finish_reason: None,
        }
    }

    fn message_start(&mut self, value: &Value) -> Option<Result<StreamEvent, UpstreamCallError>> {
        let message = value.get("message").unwrap_or(&Value::Null);
        self.message_id = message
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.model = message
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(input) = message
            .get("usage")
            .and_then(|usage| usage.get("input_tokens"))
            .and_then(Value::as_u64)
        {
            self.input_tokens = input as u32;
        }

        let data = self.chunk(json!({ "role": "assistant", "content": "" }), None);
        Some(Ok(self.emit(data, None)))
    }

    fn content_block_start(
        &mut self,
        value: &Value,
    ) -> Option<Result<StreamEvent, UpstreamCallError>> {
        let block = value.get("content_block")?;
        let id = block.get("id").and_then(Value::as_str)?;
        let name = block
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            return None;
        }

        let index = self.tool_index(value.get("index").and_then(Value::as_u64).unwrap_or(0));
        let data = self.chunk(
            json!({
                "tool_calls": [{
                    "index": index,
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": "" },
                }],
            }),
            None,
        );
        Some(Ok(self.emit(data, None)))
    }

    fn content_block_delta(
        &mut self,
        value: &Value,
    ) -> Option<Result<StreamEvent, UpstreamCallError>> {
        let delta = value.get("delta")?;
        let delta_type = delta
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("text_delta");

        match delta_type {
            "text_delta" => {
                let text = delta.get("text").and_then(Value::as_str)?;
                let data = self.chunk(json!({ "content": text }), None);
                Some(Ok(self.emit(data, Some(text.to_string()))))
            }
            "thinking_delta" => {
                let thinking = delta.get("thinking").and_then(Value::as_str)?;
                if thinking.is_empty() {
                    return None;
                }
                let data = self.chunk(json!({ "reasoning_content": thinking }), None);
                Some(Ok(self.emit(data, None)))
            }
            "input_json_delta" => {
                let partial = delta.get("partial_json").and_then(Value::as_str)?;
                let block_index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
                let index = self.tool_index(block_index);
                let data = self.chunk(
                    json!({
                        "tool_calls": [{
                            "index": index,
                            "function": { "arguments": partial },
                        }],
                    }),
                    None,
                );
                Some(Ok(self.emit(data, None)))
            }
            _ => None,
        }
    }

    fn message_delta(&mut self, value: &Value) -> Option<Result<StreamEvent, UpstreamCallError>> {
        let finish_reason = map_stop_reason(
            value
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(Value::as_str),
        );

        let completion_tokens = value
            .get("usage")
            .and_then(|usage| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;

        let usage = UsageInfo {
            prompt_tokens: self.input_tokens,
            completion_tokens,
            total_tokens: self.input_tokens.saturating_add(completion_tokens),
        };

        let mut data = self.chunk(json!({}), finish_reason.as_deref());
        data["usage"] = json!({
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        });

        Some(Ok(StreamEvent::Chunk {
            data,
            delta_text: None,
            usage: Some(usage),
            finish_reason,
        }))
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
                    if let Some(item) = translator.translate(&event.event, &event.data) {
                        let terminal = translator.is_done();
                        yield item;
                        if terminal {
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
    }
}
