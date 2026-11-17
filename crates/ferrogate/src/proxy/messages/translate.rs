use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    providers::{ChatCompletionRequest, ChatMessage},
};

pub(crate) struct Parsed {
    pub request: ChatCompletionRequest,
    pub model: String,
}

pub(crate) fn from_messages(raw: &Value) -> AppResult<Parsed> {
    let model = raw
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.trim().is_empty())
        .ok_or_else(|| AppError::BadRequest("field 'model' must be a non-empty string".into()))?
        .to_string();

    let max_tokens = raw
        .get("max_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| AppError::BadRequest("field 'max_tokens' is required".into()))?;

    let mut messages: Vec<ChatMessage> = Vec::new();
    if let Some(system) = raw.get("system") {
        messages.push(message("system", system_text(system)));
    }
    if let Some(items) = raw.get("messages").and_then(Value::as_array) {
        for item in items {
            messages.extend(translate_message(item));
        }
    }

    let tools = translate_tools(raw.get("tools"));

    Ok(Parsed {
        model: model.clone(),
        request: ChatCompletionRequest {
            model,
            messages,
            temperature: raw.get("temperature").and_then(Value::as_f64),
            top_p: raw.get("top_p").and_then(Value::as_f64),
            max_tokens: Some(max_tokens.min(u64::from(u32::MAX)) as u32),
            max_completion_tokens: None,
            stream: raw.get("stream").and_then(Value::as_bool),
            stream_options: None,
            stop: raw
                .get("stop_sequences")
                .filter(|value| value.as_array().is_some_and(|items| !items.is_empty()))
                .cloned(),
            presence_penalty: None,
            frequency_penalty: None,
            user: None,
            tools,
            tool_choice: translate_tool_choice(raw.get("tool_choice")),
            response_format: None,
            seed: None,
            reasoning_effort: thinking_effort(raw.get("thinking")),
            template: None,
            template_version: None,
            template_variables: None,
        },
    })
}

fn thinking_effort(thinking: Option<&Value>) -> Option<String> {
    let budget = thinking?.get("budget_tokens")?.as_u64()?;
    Some(
        match budget {
            0..=1_024 => "low",
            1_025..=4_096 => "medium",
            _ => "high",
        }
        .to_string(),
    )
}

fn system_text(system: &Value) -> Value {
    match system {
        Value::String(text) => Value::String(text.clone()),
        Value::Array(blocks) => Value::Array(
            blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .map(|text| json!({ "type": "text", "text": text }))
                .collect(),
        ),
        other => Value::String(other.to_string()),
    }
}

fn translate_message(item: &Value) -> Vec<ChatMessage> {
    let role = item.get("role").and_then(Value::as_str).unwrap_or("user");

    match item.get("content") {
        Some(Value::String(text)) => vec![message(role, Value::String(text.clone()))],
        Some(Value::Array(blocks)) => blocks_to_messages(role, blocks),
        _ => vec![message(role, Value::String(String::new()))],
    }
}

fn blocks_to_messages(role: &str, blocks: &[Value]) -> Vec<ChatMessage> {
    let mut out = Vec::new();
    let mut text_parts: Vec<Value> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();

    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => text_parts.push(json!({
                "type": "text",
                "text": block.get("text").cloned().unwrap_or(Value::Null),
            })),
            Some("image") => {
                if let Some(url) = image_url(block) {
                    text_parts.push(json!({ "type": "image_url", "image_url": { "url": url } }));
                }
            }
            Some("tool_use") => tool_calls.push(json!({
                "id": block.get("id").cloned().unwrap_or(Value::Null),
                "type": "function",
                "function": {
                    "name": block.get("name").cloned().unwrap_or(Value::Null),
                    "arguments": block
                        .get("input")
                        .map(Value::to_string)
                        .unwrap_or_else(|| "{}".to_string()),
                },
            })),
            Some("tool_result") => {
                let mut result = message(
                    "tool",
                    Value::String(tool_result_text(block.get("content"))),
                );
                result.tool_call_id = block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                out.push(result);
            }
            _ => {}
        }
    }

    if !text_parts.is_empty() || role == "assistant" {
        let content = collapse(text_parts);
        let mut assistant = message(role, content);
        if !tool_calls.is_empty() {
            assistant.tool_calls = Some(Value::Array(tool_calls));
        }
        out.push(assistant);
    }
    out
}

fn collapse(parts: Vec<Value>) -> Value {
    let all_text = parts
        .iter()
        .all(|part| part.get("type").and_then(Value::as_str) == Some("text"));

    if all_text && parts.len() == 1 {
        return parts
            .into_iter()
            .next()
            .and_then(|part| part.get("text").cloned())
            .unwrap_or_else(|| Value::String(String::new()));
    }
    Value::Array(parts)
}

fn image_url(block: &Value) -> Option<String> {
    let source = block.get("source")?;
    match source.get("type").and_then(Value::as_str) {
        Some("base64") => {
            let media = source
                .get("media_type")
                .and_then(Value::as_str)
                .unwrap_or("image/png");
            let data = source.get("data").and_then(Value::as_str)?;
            Some(format!("data:{media};base64,{data}"))
        }
        _ => source
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn translate_tools(tools: Option<&Value>) -> Option<Value> {
    let tools = tools?.as_array()?;
    let translated: Vec<Value> = tools
        .iter()
        .filter_map(|tool| {
            let name = tool.get("name").and_then(Value::as_str)?;
            let mut function = json!({
                "name": name,
                "parameters": tool
                    .get("input_schema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
            });
            if let Some(description) = tool.get("description") {
                function["description"] = description.clone();
            }
            Some(json!({ "type": "function", "function": function }))
        })
        .collect();

    (!translated.is_empty()).then_some(Value::Array(translated))
}

fn translate_tool_choice(choice: Option<&Value>) -> Option<Value> {
    let choice = choice?;
    match choice.get("type").and_then(Value::as_str) {
        Some("auto") => Some(json!("auto")),
        Some("any") => Some(json!("required")),
        Some("none") => Some(json!("none")),
        Some("tool") => choice
            .get("name")
            .and_then(Value::as_str)
            .map(|name| json!({ "type": "function", "function": { "name": name } })),
        _ => None,
    }
}

fn message(role: &str, content: Value) -> ChatMessage {
    ChatMessage {
        role: role.to_string(),
        content,
        name: None,
        tool_call_id: None,
        tool_calls: None,
    }
}
