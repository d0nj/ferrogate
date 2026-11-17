use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    providers::{ChatCompletionRequest, ChatMessage},
};

pub(super) fn from_responses(raw: &Value) -> AppResult<ChatCompletionRequest> {
    let model = string(raw, "model")
        .ok_or_else(|| AppError::BadRequest("field 'model' must be a non-empty string".into()))?;

    let mut messages: Vec<ChatMessage> = Vec::new();
    if let Some(instructions) = string(raw, "instructions") {
        messages.push(message("system", Value::String(instructions)));
    }
    messages.extend(translate_input(raw.get("input"))?);

    let text_format = raw
        .get("text")
        .and_then(|text| text.get("format"))
        .and_then(translate_format);

    Ok(ChatCompletionRequest {
        model,
        messages,
        temperature: number(raw, "temperature"),
        top_p: number(raw, "top_p"),
        max_tokens: None,
        max_completion_tokens: raw
            .get("max_output_tokens")
            .and_then(Value::as_u64)
            .map(|value| value.min(u64::from(u32::MAX)) as u32),
        stream: raw.get("stream").and_then(Value::as_bool),
        stream_options: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        user: string(raw, "user"),
        tools: raw.get("tools").and_then(translate_tools),
        tool_choice: raw.get("tool_choice").cloned(),
        response_format: text_format,
        seed: None,
        reasoning_effort: raw
            .get("reasoning")
            .and_then(|reasoning| reasoning.get("effort"))
            .and_then(Value::as_str)
            .map(str::to_string),
        template: None,
        template_version: None,
        template_variables: None,
    })
}

fn translate_input(input: Option<&Value>) -> AppResult<Vec<ChatMessage>> {
    let Some(input) = input else {
        return Err(AppError::BadRequest(
            "field 'input' must be a string or an array of items".into(),
        ));
    };

    match input {
        Value::String(text) => Ok(vec![message("user", Value::String(text.clone()))]),
        Value::Array(items) => items.iter().map(translate_item).collect(),
        Value::Object(_) => Ok(vec![translate_item(input)?]),
        other => Err(AppError::BadRequest(format!("invalid 'input': {other}"))),
    }
}

fn translate_item(item: &Value) -> AppResult<ChatMessage> {
    let kind = item.get("type").and_then(Value::as_str);

    match kind {
        Some("function_call") => {
            let call_id = string(item, "call_id").or_else(|| string(item, "id"));
            Ok(ChatMessage {
                role: "assistant".into(),
                content: Value::String(String::new()),
                name: None,
                tool_call_id: None,
                tool_calls: Some(json!([{
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": item.get("name").cloned().unwrap_or(Value::Null),
                        "arguments": arguments(item.get("arguments")),
                    },
                }])),
            })
        }
        Some("function_call_output") => Ok(ChatMessage {
            role: "tool".into(),
            content: Value::String(flatten_output(item.get("output"))),
            name: None,
            tool_call_id: string(item, "call_id"),
            tool_calls: None,
        }),
        _ => {
            let role = string(item, "role").unwrap_or_else(|| "user".to_string());
            let content = item
                .get("content")
                .map(translate_content)
                .unwrap_or_else(|| Value::String(String::new()));
            Ok(message(&role, content))
        }
    }
}

fn translate_content(content: &Value) -> Value {
    match content {
        Value::String(text) => Value::String(text.clone()),
        Value::Array(parts) => Value::Array(parts.iter().map(translate_part).collect()),
        other => Value::String(other.to_string()),
    }
}

fn translate_part(part: &Value) -> Value {
    match part.get("type").and_then(Value::as_str) {
        Some("input_text") | Some("output_text") => json!({
            "type": "text",
            "text": part.get("text").cloned().unwrap_or(Value::Null),
        }),
        Some("input_image") => json!({
            "type": "image_url",
            "image_url": { "url": part.get("image_url").cloned().unwrap_or(Value::Null) },
        }),
        _ => part.clone(),
    }
}

fn translate_tools(tools: &Value) -> Option<Value> {
    let tools = tools.as_array()?;
    let translated: Vec<Value> = tools
        .iter()
        .filter_map(|tool| {
            let definition = tool.get("function").unwrap_or(tool);
            let name = definition.get("name").and_then(Value::as_str)?;
            let mut function = json!({
                "name": name,
                "parameters": definition
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
            });
            if let Some(description) = definition.get("description") {
                function["description"] = description.clone();
            }
            Some(json!({ "type": "function", "function": function }))
        })
        .collect();

    (!translated.is_empty()).then_some(Value::Array(translated))
}

fn translate_format(format: &Value) -> Option<Value> {
    let kind = format.get("type").and_then(Value::as_str)?;
    match kind {
        "json_object" => Some(json!({ "type": "json_object" })),
        "json_schema" => Some(json!({
            "type": "json_schema",
            "json_schema": {
                "name": format.get("name").and_then(Value::as_str).unwrap_or("response"),
                "schema": format.get("schema").cloned().unwrap_or(Value::Null),
                "strict": format.get("strict").and_then(Value::as_bool).unwrap_or(false),
            },
        })),
        _ => None,
    }
}

fn arguments(raw: Option<&Value>) -> Value {
    match raw {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(other) => Value::String(other.to_string()),
        None => Value::String("{}".to_string()),
    }
}

fn flatten_output(raw: Option<&Value>) -> String {
    match raw {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
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

pub(super) fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}
