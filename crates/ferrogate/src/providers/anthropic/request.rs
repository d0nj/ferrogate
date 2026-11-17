use serde_json::{json, Map, Value};

use crate::providers::{message_content_to_text, support, ChatCompletionRequest, ChatMessage};

use super::content::translate_tool_calls;
use super::DEFAULT_MAX_TOKENS;
use super::{
    stop_sequences, translate_content, translate_response_format, translate_tool_choice,
    translate_tools,
};

pub(crate) fn build_request_body(req: &ChatCompletionRequest, upstream_model: &str) -> Value {
    let mut system_parts: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();

    for message in &req.messages {
        push_openai_message(&mut messages, &mut system_parts, message);
    }

    let mut max_tokens = req
        .max_completion_tokens
        .or(req.max_tokens)
        .unwrap_or(DEFAULT_MAX_TOKENS);

    let mut body = Map::new();
    body.insert("model".into(), Value::String(upstream_model.to_string()));
    body.insert("messages".into(), Value::Array(messages));

    if let Some(budget) = support::thinking_budget(req.reasoning_effort.as_deref()) {
        if max_tokens <= budget {
            max_tokens = budget + DEFAULT_MAX_TOKENS;
        }
        body.insert(
            "thinking".into(),
            json!({ "type": "enabled", "budget_tokens": budget }),
        );
    } else if let Some(temperature) = req.temperature {
        body.insert("temperature".into(), json!(temperature));
    }

    body.insert("max_tokens".into(), json!(max_tokens));

    if !system_parts.is_empty() {
        body.insert("system".into(), Value::String(system_parts.join("\n\n")));
    }
    if let Some(top_p) = req.top_p {
        body.insert("top_p".into(), json!(top_p));
    }
    if let Some(sequences) = stop_sequences(req.stop.as_ref()) {
        body.insert("stop_sequences".into(), json!(sequences));
    }
    if req.stream.unwrap_or(false) {
        body.insert("stream".into(), Value::Bool(true));
    }

    if let Some(tools) = translate_tools(req.tools.as_ref()) {
        body.insert("tools".into(), Value::Array(tools));
        if let Some(choice) = translate_tool_choice(req.tool_choice.as_ref()) {
            if choice["type"] != "none" {
                body.insert("tool_choice".into(), choice);
            }
        }
    }
    if let Some(output_config) = translate_response_format(req.response_format.as_ref()) {
        body.insert("output_config".into(), output_config);
    }

    Value::Object(body)
}

fn push_openai_message(
    messages: &mut Vec<Value>,
    system_parts: &mut Vec<String>,
    message: &ChatMessage,
) {
    let text = message_content_to_text(&message.content);

    match message.role.as_str() {
        "system" | "developer" => {
            if !text.is_empty() {
                system_parts.push(text);
            }
        }
        "tool" => {
            let tool_use_id = message
                .tool_call_id
                .clone()
                .unwrap_or_else(|| format!("toolu_{}", messages.len()));
            let block = json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": translate_content(&message.content),
            });
            push_message(messages, "user", Value::Array(vec![block]));
        }
        "assistant" => {
            let mut blocks: Vec<Value> = Vec::new();
            if !text.is_empty() {
                blocks.push(json!({ "type": "text", "text": text }));
            }
            if let Some(tool_calls) = &message.tool_calls {
                blocks.extend(translate_tool_calls(tool_calls));
            }

            if blocks.is_empty() {
                push_message(messages, "assistant", translate_content(&message.content));
            } else {
                push_message(messages, "assistant", Value::Array(blocks));
            }
        }
        _ => push_message(messages, "user", translate_content(&message.content)),
    }
}

fn push_message(messages: &mut Vec<Value>, role: &str, content: Value) {
    if let Some(last) = messages.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some(role) {
            let existing = last.as_object().and_then(|map| map.get("content")).cloned();
            if let Some(map) = last.as_object_mut() {
                map.insert("content".into(), merge_content(existing, content));
            }
            return;
        }
    }

    let block_list = blockify(&content);
    let stored = if block_list.len() == 1
        && block_list[0].get("type").and_then(Value::as_str) == Some("text")
    {
        content
    } else {
        Value::Array(block_list)
    };
    messages.push(json!({ "role": role, "content": stored }));
}

fn merge_content(existing: Option<Value>, next: Value) -> Value {
    match (existing, next) {
        (Some(Value::String(mut previous)), Value::String(next)) => {
            if !previous.is_empty() && !next.is_empty() {
                previous.push_str("\n\n");
            }
            previous.push_str(&next);
            Value::String(previous)
        }
        (Some(Value::Array(mut blocks)), Value::Array(other)) => {
            blocks.extend(other);
            Value::Array(blocks)
        }
        (Some(Value::Array(mut blocks)), other) => {
            blocks.extend(blockify(&other));
            Value::Array(blocks)
        }
        (Some(Value::String(previous)), other) => {
            let mut blocks = blockify(&Value::String(previous));
            blocks.extend(blockify(&other));
            Value::Array(blocks)
        }
        (None, other) => other,
        (Some(_), other) => other,
    }
}

fn blockify(content: &Value) -> Vec<Value> {
    match content {
        Value::String(text) => vec![json!({ "type": "text", "text": text })],
        Value::Array(blocks) => blocks.clone(),
        other => vec![json!({ "type": "text", "text": message_content_to_text(other) })],
    }
}
