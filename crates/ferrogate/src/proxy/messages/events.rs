use axum::response::sse::Event;
use serde_json::{json, Value};

use crate::providers::{ChatCompletionResponse, StreamEvent};

use super::super::streaming::Formatter;

pub(crate) fn message_body(completion: &ChatCompletionResponse, model: &str) -> Value {
    let choice = completion.choices.first();
    let mut content: Vec<Value> = Vec::new();

    if let Some(choice) = choice {
        let text = crate::providers::message_content_to_text(&choice.message.content);
        if !text.is_empty() {
            content.push(json!({ "type": "text", "text": text }));
        }
        if let Some(calls) = choice.message.tool_calls.as_ref().and_then(Value::as_array) {
            for call in calls {
                content.push(json!({
                    "type": "tool_use",
                    "id": call.get("id").cloned().unwrap_or(Value::Null),
                    "name": call.pointer("/function/name").cloned().unwrap_or(Value::Null),
                    "input": input(&call.pointer("/function/arguments").cloned()),
                }));
            }
        }
    }

    let usage = completion.usage.clone().unwrap_or_default();
    json!({
        "id": format!("msg_{}", completion.id),
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content,
        "stop_reason": stop_reason(choice.and_then(|choice| choice.finish_reason.as_deref())),
        "stop_sequence": Value::Null,
        "usage": {
            "input_tokens": usage.prompt_tokens,
            "output_tokens": usage.completion_tokens,
        },
    })
}

pub(crate) fn format(model: &str) -> Formatter {
    let mut state = MessageStream {
        model: model.to_string(),
        ..MessageStream::default()
    };
    Box::new(move |event: &StreamEvent| state.translate(event))
}

#[derive(Default)]
struct MessageStream {
    id: String,
    model: String,
    opened: bool,
    closed: bool,
    text_index: Option<u64>,
    tools: Vec<ToolBlock>,
}

#[derive(Default)]
struct ToolBlock {
    index: u64,
    announced: bool,
    id: String,
    name: String,
}

impl MessageStream {
    fn translate(&mut self, event: &StreamEvent) -> Vec<Event> {
        match event {
            StreamEvent::Done => self.finish(),
            StreamEvent::Chunk { data, .. } => self.chunk(data),
        }
    }

    fn chunk(&mut self, data: &Value) -> Vec<Event> {
        if self.id.is_empty() {
            self.id = data
                .get("id")
                .and_then(Value::as_str)
                .map(|id| format!("msg_{id}"))
                .unwrap_or_else(|| "msg_ferrogate".to_string());
        }
        if let Some(model) = data.get("model").and_then(Value::as_str) {
            if self.model.is_empty() {
                self.model = model.to_string();
            }
        }

        let mut events = self.open();
        let choice = data.get("choices").and_then(|choices| choices.get(0));
        let delta = choice.and_then(|choice| choice.get("delta"));

        if let Some(text) = delta
            .and_then(|delta| delta.get("content"))
            .and_then(Value::as_str)
        {
            if !text.is_empty() {
                events.extend(self.text_delta(text));
            }
        }

        if let Some(calls) = delta
            .and_then(|delta| delta.get("tool_calls"))
            .and_then(Value::as_array)
        {
            for call in calls {
                events.extend(self.tool_delta(call));
            }
        }

        if let Some(reason) = choice.and_then(|choice| choice.get("finish_reason")) {
            if !reason.is_null() {
                events.extend(self.finish());
            }
        }
        events
    }

    fn open(&mut self) -> Vec<Event> {
        if self.opened {
            return Vec::new();
        }
        self.opened = true;
        vec![emit(
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": self.id,
                    "type": "message",
                    "role": "assistant",
                    "model": self.model,
                    "content": [],
                    "stop_reason": Value::Null,
                    "usage": { "input_tokens": 0, "output_tokens": 0 },
                },
            }),
        )]
    }

    fn text_delta(&mut self, text: &str) -> Vec<Event> {
        let mut events = Vec::new();
        let index = *self.text_index.get_or_insert_with(|| {
            events.push(emit(
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": { "type": "text", "text": "" },
                }),
            ));
            0
        });
        events.push(emit(
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": index,
                "delta": { "type": "text_delta", "text": text },
            }),
        ));
        events
    }

    fn tool_delta(&mut self, call: &Value) -> Vec<Event> {
        let mut events = Vec::new();
        let raw_index = call.get("index").and_then(Value::as_u64).unwrap_or(0);

        let position = self
            .tools
            .iter()
            .position(|tool| tool.index == raw_index)
            .unwrap_or_else(|| {
                self.tools.push(ToolBlock {
                    index: raw_index,
                    announced: false,
                    id: call
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    name: call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                });
                self.tools.len() - 1
            });
        let block = &mut self.tools[position];
        if block.name.is_empty() {
            return events;
        }
        let anthropic_index = 1 + block.index;

        if !block.announced {
            block.announced = true;
            events.push(emit(
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": anthropic_index,
                    "content_block": {
                        "type": "tool_use",
                        "id": block.id,
                        "name": block.name,
                        "input": {},
                    },
                }),
            ));
        }
        if let Some(partial) = call.pointer("/function/arguments").and_then(Value::as_str) {
            if !partial.is_empty() {
                events.push(emit(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": anthropic_index,
                        "delta": { "type": "input_json_delta", "partial_json": partial },
                    }),
                ));
            }
        }
        events
    }

    fn finish(&mut self) -> Vec<Event> {
        if self.closed {
            return Vec::new();
        }
        self.closed = true;
        let mut events = Vec::new();
        for block in &self.tools {
            events.push(emit(
                "content_block_stop",
                json!({ "type": "content_block_stop", "index": 1 + block.index }),
            ));
        }
        events.push(emit(
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": self.stop_reason(), "stop_sequence": Value::Null },
                "usage": { "output_tokens": 0 },
            }),
        ));
        events.push(emit("message_stop", json!({ "type": "message_stop" })));
        events
    }

    fn stop_reason(&self) -> &'static str {
        if self.tools.is_empty() {
            "end_turn"
        } else {
            "tool_use"
        }
    }
}

fn stop_reason(reason: Option<&str>) -> &'static str {
    match reason {
        Some("length") => "max_tokens",
        Some("tool_calls") => "tool_use",
        Some("content_filter") => "refusal",
        _ => "end_turn",
    }
}

fn input(raw: &Option<Value>) -> Value {
    match raw {
        Some(Value::String(text)) if !text.is_empty() => {
            serde_json::from_str(text).unwrap_or_else(|_| json!({}))
        }
        Some(value @ Value::Object(_)) => value.clone(),
        _ => json!({}),
    }
}

fn emit(event: &str, data: Value) -> Event {
    Event::default()
        .event(event)
        .json_data(data)
        .unwrap_or_default()
}
