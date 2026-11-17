use serde_json::{json, Map, Value};

use crate::providers::{message_content_to_text, support};

fn translate_part(part: &Value) -> Option<Value> {
    match part {
        Value::String(text) => Some(json!({ "type": "text", "text": text })),
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                let mut block = json!({ "type": "text", "text": text });
                if let Some(cache_control) = map.get("cache_control") {
                    block["cache_control"] = cache_control.clone();
                }
                return Some(block);
            }

            let url = support::image_part_url(part)?;
            if let Some(parsed) = support::parse_data_url(url) {
                return Some(json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": parsed.mime,
                        "data": parsed.data,
                    },
                }));
            }
            Some(json!({
                "type": "image",
                "source": { "type": "url", "url": url },
            }))
        }
        _ => None,
    }
}

pub(crate) fn translate_content(content: &Value) -> Value {
    match content {
        Value::String(text) => Value::String(text.clone()),
        Value::Array(parts) => {
            let blocks: Vec<Value> = parts.iter().filter_map(translate_part).collect();

            if blocks.is_empty() {
                Value::String(message_content_to_text(content))
            } else {
                Value::Array(blocks)
            }
        }
        Value::Null => Value::String(String::new()),
        other => Value::String(message_content_to_text(other)),
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

pub(crate) fn translate_tools(tools: Option<&Value>) -> Option<Vec<Value>> {
    let tools = tools?.as_array()?;
    let translated: Vec<Value> = tools
        .iter()
        .filter_map(|tool| {
            let definition = tool.get("function").unwrap_or(tool);
            let name = definition.get("name").and_then(Value::as_str)?;
            let mut entry = Map::new();
            entry.insert("name".into(), Value::String(name.to_string()));
            if let Some(description) = definition.get("description") {
                entry.insert("description".into(), description.clone());
            }
            let schema = definition
                .get("parameters")
                .filter(|value| value.is_object())
                .cloned()
                .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
            entry.insert("input_schema".into(), schema);
            Some(Value::Object(entry))
        })
        .collect();

    if translated.is_empty() {
        None
    } else {
        Some(translated)
    }
}

pub(crate) fn translate_tool_choice(tool_choice: Option<&Value>) -> Option<Value> {
    match tool_choice? {
        Value::String(mode) => match mode.as_str() {
            "auto" => Some(json!({ "type": "auto" })),
            "required" | "any" => Some(json!({ "type": "any" })),
            "none" => Some(json!({ "type": "none" })),
            _ => None,
        },
        Value::Object(map) => {
            let name = map
                .get("function")
                .and_then(|function| function.get("name"))
                .or_else(|| map.get("name"))
                .and_then(Value::as_str)?;
            Some(json!({ "type": "tool", "name": name }))
        }
        _ => None,
    }
}

pub(crate) fn translate_response_format(response_format: Option<&Value>) -> Option<Value> {
    match response_format? {
        Value::Object(map) => match map.get("type").and_then(Value::as_str)? {
            "json_schema" => {
                let schema = map
                    .get("json_schema")
                    .and_then(|json_schema| json_schema.get("schema"))
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object" }));
                Some(json!({ "format": { "type": "json_schema", "schema": schema } }))
            }
            "json_object" => Some(json!({
                "format": { "type": "json_schema", "schema": { "type": "object" } }
            })),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn translate_tool_calls(tool_calls: &Value) -> Vec<Value> {
    let calls = match tool_calls {
        Value::Array(calls) => calls.as_slice(),
        _ => return Vec::new(),
    };

    calls
        .iter()
        .filter_map(|call| {
            let function = call.get("function")?;
            let name = function.get("name").and_then(Value::as_str)?;
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("toolu_call");
            Some(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": support::parse_tool_arguments(function.get("arguments")),
            }))
        })
        .collect()
}
