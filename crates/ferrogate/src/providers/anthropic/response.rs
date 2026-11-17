use serde_json::{json, Value};

use crate::providers::{
    ChatChoice, ChatChoiceMessage, ChatCompletionResponse, UpstreamCallError, UsageInfo,
};

pub(crate) fn map_stop_reason(stop_reason: Option<&str>) -> Option<String> {
    let mapped = match stop_reason? {
        "end_turn" | "stop_sequence" => "stop",
        "max_tokens" | "model_context_window_exceeded" => "length",
        "tool_use" => "tool_calls",
        "pause_turn" | "refusal" => "content_filter",
        _ => "stop",
    };
    Some(mapped.to_string())
}

pub(crate) fn translate_response(
    value: &Value,
) -> Result<ChatCompletionResponse, UpstreamCallError> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("anthropic-message")
        .to_string();
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let mut text = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();

    if let Some(blocks) = value.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(chunk) = block.get("text").and_then(Value::as_str) {
                        text.push_str(chunk);
                    }
                }
                Some("tool_use") => tool_calls.push(json!({
                    "id": block.get("id").cloned().unwrap_or(Value::Null),
                    "type": "function",
                    "function": {
                        "name": block.get("name").cloned().unwrap_or(Value::Null),
                        "arguments": block
                            .get("input")
                            .map(|input| input.to_string())
                            .unwrap_or_else(|| "{}".to_string()),
                    },
                })),
                _ => {}
            }
        }
    }

    let finish_reason = map_stop_reason(value.get("stop_reason").and_then(Value::as_str));

    let usage = value.get("usage").map(|usage| {
        let prompt_tokens = usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let completion_tokens = usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        UsageInfo {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens.saturating_add(completion_tokens),
        }
    });

    Ok(ChatCompletionResponse {
        id,
        object: "chat.completion".to_string(),
        created: crate::providers::unix_timestamp(),
        model,
        choices: vec![ChatChoice {
            index: 0,
            message: ChatChoiceMessage {
                role: "assistant".to_string(),
                content: Value::String(text),
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(Value::Array(tool_calls))
                },
            },
            finish_reason,
        }],
        usage,
    })
}
