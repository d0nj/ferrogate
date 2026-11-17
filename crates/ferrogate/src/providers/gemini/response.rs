use serde_json::{json, Value};

use super::super::support::unix_timestamp;
use super::super::types::{
    ChatChoice, ChatChoiceMessage, ChatCompletionResponse, UpstreamCallError, UsageInfo,
};

pub(crate) fn map_finish_reason(reason: Option<&str>) -> Option<String> {
    let mapped = match reason? {
        "STOP" => "stop",
        "MAX_TOKENS" => "length",
        "SAFETY"
        | "RECITATION"
        | "BLOCKLIST"
        | "PROHIBITED_CONTENT"
        | "SPII"
        | "IMAGE_SAFETY"
        | "IMAGE_PROHIBITED_CONTENT"
        | "IMAGE_RECITATION" => "content_filter",
        _ => "stop",
    };
    Some(mapped.to_string())
}

pub(crate) fn parse_usage(metadata: Option<&Value>) -> Option<UsageInfo> {
    let metadata = metadata?.as_object()?;

    let prompt_tokens = metadata
        .get("promptTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let completion_tokens = metadata
        .get("candidatesTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let total_tokens = metadata
        .get("totalTokenCount")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(prompt_tokens.saturating_add(completion_tokens));

    Some(UsageInfo {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    })
}

pub(crate) fn candidate_text(candidate: &Value) -> String {
    candidate_parts(candidate)
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<String>()
}

fn candidate_parts(candidate: &Value) -> impl Iterator<Item = &Value> {
    candidate
        .get("content")
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
        .map(|parts| parts.as_slice())
        .unwrap_or(&[])
        .iter()
}

pub(crate) fn first_candidate(value: &Value) -> Option<&Value> {
    value
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
}

pub(crate) fn translate_function_calls(candidate: Option<&Value>, stamp: i64) -> Vec<Value> {
    let Some(candidate) = candidate else {
        return Vec::new();
    };

    candidate_parts(candidate)
        .filter_map(|part| part.get("functionCall"))
        .enumerate()
        .filter_map(|(index, call)| {
            let name = call.get("name").and_then(Value::as_str)?;
            let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
            Some(json!({
                "id": format!("call_{index}_{stamp}"),
                "type": "function",
                "function": { "name": name, "arguments": args.to_string() },
            }))
        })
        .collect()
}

pub(crate) fn finish_reason_for(candidate: Option<&Value>, tool_calls: &[Value]) -> Option<String> {
    if !tool_calls.is_empty() {
        return Some("tool_calls".to_string());
    }
    map_finish_reason(
        candidate
            .and_then(|candidate| candidate.get("finishReason"))
            .and_then(Value::as_str),
    )
}

pub(crate) fn translate_response(
    value: &Value,
    requested_model: &str,
) -> Result<ChatCompletionResponse, UpstreamCallError> {
    let candidate = first_candidate(value);

    if candidate.is_none() {
        if let Some(reason) = value
            .get("promptFeedback")
            .and_then(|feedback| feedback.get("blockReason"))
            .and_then(Value::as_str)
        {
            return Err(UpstreamCallError::Protocol(format!(
                "Gemini blocked the prompt: {reason}"
            )));
        }
    }

    let created = unix_timestamp();
    let text = candidate.map(candidate_text).unwrap_or_default();
    let tool_calls = translate_function_calls(candidate, created);
    let finish_reason = finish_reason_for(candidate, &tool_calls);

    Ok(ChatCompletionResponse {
        id: format!("chatcmpl-{created}"),
        object: "chat.completion".to_string(),
        created,
        model: requested_model.to_string(),
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
        usage: parse_usage(value.get("usageMetadata")),
    })
}
