use axum::response::sse::Event;
use serde_json::{json, Value};

use crate::providers::{ChatCompletionResponse, StreamEvent};

use super::super::streaming::Formatter;

pub(crate) fn completed_body(completion: &ChatCompletionResponse, model: &str) -> Value {
    let id = response_id(&completion.id);
    let message = completion.choices.first();
    let text = message
        .map(|choice| crate::providers::message_content_to_text(&choice.message.content))
        .unwrap_or_default();
    let tool_calls = message
        .and_then(|choice| choice.message.tool_calls.clone())
        .unwrap_or_else(|| json!([]));

    let mut output = vec![message_item(&id, &text)];
    if let Some(calls) = tool_calls.as_array() {
        for (index, call) in calls.iter().enumerate() {
            output.push(function_item(&id, index as u64 + 1, call));
        }
    }

    let usage = completion.usage.clone().unwrap_or_default();
    json!({
        "id": id,
        "object": "response",
        "created_at": completion.created,
        "status": "completed",
        "model": model,
        "output": output,
        "usage": {
            "input_tokens": usage.prompt_tokens,
            "output_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        },
    })
}

pub(crate) fn format() -> Formatter {
    let mut state = ResponsesStream::default();
    Box::new(move |event: &StreamEvent| state.translate(event))
}

#[derive(Default)]
struct ResponsesStream {
    id: String,
    model: String,
    opened: bool,
    text: String,
}

impl ResponsesStream {
    fn translate(&mut self, event: &StreamEvent) -> Vec<Event> {
        match event {
            StreamEvent::Done => Vec::new(),
            StreamEvent::Chunk { data, .. } => {
                self.observe(data);
                let mut events = self.open();
                let text = chunk_text(data);
                if !text.is_empty() {
                    self.text.push_str(&text);
                    events.push(emit(
                        "response.output_text.delta",
                        json!({
                            "type": "response.output_text.delta",
                            "item_id": format!("msg_{}", self.id),
                            "output_index": 0,
                            "content_index": 0,
                            "delta": text,
                        }),
                    ));
                }
                if let Some(usage) = data.get("usage").filter(|usage| usage.is_object()) {
                    events.extend(self.close(usage));
                }
                events
            }
        }
    }

    fn observe(&mut self, data: &Value) {
        if self.id.is_empty() {
            self.id = response_id(data.get("id").and_then(Value::as_str).unwrap_or("stream"));
        }
        if let Some(model) = data.get("model").and_then(Value::as_str) {
            self.model = model.to_string();
        }
    }

    fn open(&mut self) -> Vec<Event> {
        if self.opened {
            return Vec::new();
        }
        self.opened = true;
        let response = json!({
            "id": self.id,
            "object": "response",
            "status": "in_progress",
            "model": self.model,
            "output": [],
        });
        vec![
            emit(
                "response.created",
                json!({ "type": "response.created", "response": response }),
            ),
            emit(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": 0,
                    "item": {
                        "id": format!("msg_{}", self.id),
                        "type": "message",
                        "status": "in_progress",
                        "role": "assistant",
                        "content": [],
                    },
                }),
            ),
            emit(
                "response.content_part.added",
                json!({
                    "type": "response.content_part.added",
                    "item_id": format!("msg_{}", self.id),
                    "output_index": 0,
                    "content_index": 0,
                    "part": { "type": "output_text", "text": "", "annotations": [] },
                }),
            ),
        ]
    }

    fn close(&mut self, usage: &Value) -> Vec<Event> {
        vec![
            emit(
                "response.output_text.done",
                json!({
                    "type": "response.output_text.done",
                    "item_id": format!("msg_{}", self.id),
                    "output_index": 0,
                    "content_index": 0,
                    "text": self.text,
                }),
            ),
            emit(
                "response.completed",
                json!({
                    "type": "response.completed",
                    "response": {
                        "id": self.id,
                        "object": "response",
                        "status": "completed",
                        "model": self.model,
                        "usage": {
                            "input_tokens": usage.get("prompt_tokens").cloned().unwrap_or(json!(0)),
                            "output_tokens": usage.get("completion_tokens").cloned().unwrap_or(json!(0)),
                            "total_tokens": usage.get("total_tokens").cloned().unwrap_or(json!(0)),
                        },
                    },
                }),
            ),
        ]
    }
}

fn chunk_text(data: &Value) -> String {
    data.pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn response_id(raw: &str) -> String {
    if raw.starts_with("resp_") {
        raw.to_string()
    } else {
        format!("resp_{raw}")
    }
}

fn message_item(id: &str, text: &str) -> Value {
    json!({
        "id": format!("msg_{id}"),
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{ "type": "output_text", "text": text, "annotations": [] }],
    })
}

fn function_item(id: &str, index: u64, call: &Value) -> Value {
    json!({
        "id": format!("fc_{id}_{index}"),
        "type": "function_call",
        "status": "completed",
        "call_id": call.get("id").cloned().unwrap_or(Value::Null),
        "name": call.pointer("/function/name").cloned().unwrap_or(Value::Null),
        "arguments": call.pointer("/function/arguments").cloned().unwrap_or(Value::Null),
    })
}

fn emit(event: &str, data: Value) -> Event {
    Event::default()
        .event(event)
        .json_data(data)
        .unwrap_or_default()
}
