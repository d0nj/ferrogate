use std::collections::HashMap;

use serde_json::{json, Map, Value};

use super::super::message_content_to_text;
use super::super::support::{
    gemini_thinking_budget, image_mime_from_url, image_part_url, parse_data_url,
    parse_tool_arguments,
};
use super::super::types::ChatCompletionRequest;

fn translate_part(part: &Value) -> Option<Value> {
    match part {
        Value::String(text) => Some(json!({ "text": text })),
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                return Some(json!({ "text": text }));
            }

            let url = image_part_url(part)?;
            Some(match parse_data_url(url) {
                Some(parsed) => json!({
                    "inlineData": { "mimeType": parsed.mime, "data": parsed.data },
                }),
                None => json!({
                    "fileData": {
                        "fileUri": url,
                        "mimeType": image_mime_from_url(url),
                    },
                }),
            })
        }
        _ => None,
    }
}

pub(crate) fn translate_parts(content: &Value) -> Vec<Value> {
    match content {
        Value::Array(parts) => {
            let translated: Vec<Value> = parts.iter().filter_map(translate_part).collect();

            if translated.is_empty() {
                vec![json!({ "text": message_content_to_text(content) })]
            } else {
                translated
            }
        }
        content => vec![json!({ "text": message_content_to_text(content) })],
    }
}

pub(crate) fn stop_sequences(stop: Option<&Value>) -> Option<Vec<String>> {
    let stop = stop?;
    let sequences: Vec<String> = match stop {
        Value::String(text) if !text.is_empty() => vec![text.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    };

    if sequences.is_empty() {
        None
    } else {
        Some(sequences)
    }
}

pub(crate) fn translate_tools(tools: Option<&Value>) -> Option<Value> {
    let tools = tools?.as_array()?;
    let declarations: Vec<Value> = tools
        .iter()
        .filter_map(|tool| {
            let definition = tool.get("function").unwrap_or(tool);
            let name = definition.get("name").and_then(Value::as_str)?;
            let mut entry = Map::new();
            entry.insert("name".into(), Value::String(name.to_string()));
            if let Some(description) = definition.get("description") {
                entry.insert("description".into(), description.clone());
            }
            if let Some(parameters) = definition.get("parameters") {
                entry.insert("parameters".into(), parameters.clone());
            }
            Some(Value::Object(entry))
        })
        .collect();

    if declarations.is_empty() {
        None
    } else {
        Some(json!([{ "functionDeclarations": declarations }]))
    }
}

pub(crate) fn translate_tool_choice(tool_choice: Option<&Value>) -> Option<Value> {
    let config = match tool_choice? {
        Value::String(mode) => match mode.as_str() {
            "auto" => json!({ "mode": "AUTO" }),
            "required" | "any" => json!({ "mode": "ANY" }),
            "none" => json!({ "mode": "NONE" }),
            _ => return None,
        },
        Value::Object(map) => {
            let name = map
                .get("function")
                .and_then(|function| function.get("name"))
                .or_else(|| map.get("name"))
                .and_then(Value::as_str)?;
            json!({ "mode": "ANY", "allowedFunctionNames": [name] })
        }
        _ => return None,
    };

    Some(json!({ "functionCallingConfig": config }))
}

fn push_content(contents: &mut Vec<Value>, role: &str, parts: Vec<Value>) {
    if let Some(last) = contents.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some(role) {
            if let Some(existing) = last
                .as_object_mut()
                .and_then(|map| map.get_mut("parts"))
                .and_then(Value::as_array_mut)
            {
                existing.extend(parts);
                return;
            }
        }
    }

    contents.push(json!({ "role": role, "parts": parts }));
}

fn apply_response_format(generation: &mut Map<String, Value>, response_format: Option<&Value>) {
    let Some(format) = response_format.and_then(Value::as_object) else {
        return;
    };

    match format.get("type").and_then(Value::as_str) {
        Some("json_object") => {
            generation.insert("responseMimeType".into(), json!("application/json"));
        }
        Some("json_schema") => {
            generation.insert("responseMimeType".into(), json!("application/json"));
            if let Some(schema) = format
                .get("json_schema")
                .and_then(|json_schema| json_schema.get("schema"))
            {
                generation.insert("responseSchema".into(), schema.clone());
            }
        }
        _ => {}
    }
}

fn translate_tool_calls(tool_calls: &Value) -> Vec<Value> {
    let calls = match tool_calls {
        Value::Array(calls) => calls.as_slice(),
        _ => return Vec::new(),
    };

    calls
        .iter()
        .filter_map(|call| {
            let function = call.get("function")?;
            let name = function.get("name").and_then(Value::as_str)?;
            Some(json!({
                "functionCall": {
                    "name": name,
                    "args": parse_tool_arguments(function.get("arguments")),
                },
            }))
        })
        .collect()
}

fn record_tool_names(tool_calls: Option<&Value>, tool_names: &mut HashMap<String, String>) {
    let Some(calls) = tool_calls.and_then(Value::as_array) else {
        return;
    };
    for call in calls {
        if let (Some(id), Some(name)) = (
            call.get("id").and_then(Value::as_str),
            call.get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str),
        ) {
            tool_names.insert(id.to_string(), name.to_string());
        }
    }
}

pub(crate) fn build_request_body(req: &ChatCompletionRequest) -> Value {
    let mut system_texts: Vec<String> = Vec::new();
    let mut contents: Vec<Value> = Vec::new();
    let mut tool_names: HashMap<String, String> = HashMap::new();

    for message in &req.messages {
        let text = message_content_to_text(&message.content);

        match message.role.as_str() {
            "system" | "developer" => {
                if !text.is_empty() {
                    system_texts.push(text);
                }
            }
            "tool" => {
                let name = message
                    .tool_call_id
                    .as_deref()
                    .and_then(|id| tool_names.get(id).cloned())
                    .or_else(|| message.name.clone())
                    .unwrap_or_else(|| "tool".to_string());
                let result = match &message.content {
                    Value::String(text) => json!(text),
                    Value::Null => json!(""),
                    other => other.clone(),
                };
                push_content(
                    &mut contents,
                    "user",
                    vec![json!({
                        "functionResponse": {
                            "name": name,
                            "response": { "result": result },
                        },
                    })],
                );
            }
            "assistant" => {
                record_tool_names(message.tool_calls.as_ref(), &mut tool_names);

                let mut parts = translate_parts(&message.content);
                if text.is_empty() {
                    parts.retain(|part| part.get("text").is_none());
                }
                if let Some(calls) = &message.tool_calls {
                    parts.extend(translate_tool_calls(calls));
                }

                if !parts.is_empty() {
                    push_content(&mut contents, "model", parts);
                }
            }
            _ => {
                let parts = translate_parts(&message.content);
                if !parts.is_empty() {
                    push_content(&mut contents, "user", parts);
                }
            }
        }
    }

    if contents
        .last()
        .and_then(|last| last.get("role"))
        .and_then(Value::as_str)
        == Some("model")
    {
        contents.pop();
    }

    let mut body = Map::new();
    body.insert("contents".into(), Value::Array(contents));

    if !system_texts.is_empty() {
        body.insert(
            "systemInstruction".into(),
            json!({ "parts": [{ "text": system_texts.join("\n\n") }] }),
        );
    }

    let mut generation = Map::new();
    if let Some(temperature) = req.temperature {
        generation.insert("temperature".into(), json!(temperature));
    }
    if let Some(top_p) = req.top_p {
        generation.insert("topP".into(), json!(top_p));
    }
    if let Some(max_tokens) = req.max_completion_tokens.or(req.max_tokens) {
        generation.insert("maxOutputTokens".into(), json!(max_tokens));
    }
    if let Some(sequences) = stop_sequences(req.stop.as_ref()) {
        generation.insert("stopSequences".into(), json!(sequences));
    }
    apply_response_format(&mut generation, req.response_format.as_ref());
    if let Some(budget) = gemini_thinking_budget(req.reasoning_effort.as_deref()) {
        generation.insert("thinkingConfig".into(), json!({ "thinkingBudget": budget }));
    }
    if !generation.is_empty() {
        body.insert("generationConfig".into(), Value::Object(generation));
    }

    if let Some(tools) = translate_tools(req.tools.as_ref()) {
        body.insert("tools".into(), tools);
        if let Some(config) = translate_tool_choice(req.tool_choice.as_ref()) {
            body.insert("toolConfig".into(), config);
        }
    }

    Value::Object(body)
}
