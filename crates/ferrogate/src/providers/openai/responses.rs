use serde_json::{json, Value};

use super::super::join_url;
use super::super::support::{message_content_to_text, strip_trailing_segment};
use super::super::types::{
    ChatChoice, ChatChoiceMessage, ChatCompletionRequest, ChatCompletionResponse, StreamEvent,
    UpstreamCallError, UsageInfo,
};

pub(crate) fn responses_url(base_url: &str) -> String {
    let base = strip_trailing_segment(base_url, "/v1");
    join_url(base, "/v1/responses")
}

pub(crate) fn build_request_body(req: &ChatCompletionRequest, upstream_model: &str) -> Value {
    let mut instructions: Vec<String> = Vec::new();
    let mut input: Vec<Value> = Vec::new();

    for message in &req.messages {
        let text = message_content_to_text(&message.content);
        let is_prompt = matches!(message.role.as_str(), "system" | "developer");

        if is_prompt && input.is_empty() && instructions.is_empty() {
            instructions.push(text);
            continue;
        }

        input.push(json!({
            "type": "message",
            "role": message.role,
            "content": [{ "type": "input_text", "text": text }],
        }));
    }

    let mut body = json!({
        "model": upstream_model,
        "input": input,
    });

    let object = body.as_object_mut().expect("object literal");
    if !instructions.is_empty() {
        object.insert("instructions".into(), json!(instructions.join("\n\n")));
    }
    if let Some(temperature) = req.temperature {
        object.insert("temperature".into(), json!(temperature));
    }
    if let Some(top_p) = req.top_p {
        object.insert("top_p".into(), json!(top_p));
    }
    if let Some(max_tokens) = req.max_tokens {
        object.insert("max_output_tokens".into(), json!(max_tokens.max(16)));
    }
    if let Some(stream) = req.stream {
        object.insert("stream".into(), json!(stream));
    }
    body
}

pub(crate) fn parse_response(
    value: &Value,
    fallback_model: &str,
) -> Result<ChatCompletionResponse, UpstreamCallError> {
    if matches!(
        value.get("status").and_then(Value::as_str),
        Some("failed") | Some("cancelled")
    ) {
        let message = value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("the upstream reported a failed response");
        return Err(UpstreamCallError::Protocol(message.to_string()));
    }

    Ok(ChatCompletionResponse {
        id: string_field(value, "id").unwrap_or_else(|| "resp".to_string()),
        object: "chat.completion".to_string(),
        created: value
            .get("created_at")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        model: string_field(value, "model").unwrap_or_else(|| fallback_model.to_string()),
        choices: vec![ChatChoice {
            index: 0,
            message: ChatChoiceMessage {
                role: "assistant".to_string(),
                content: Value::String(output_text(value)),
                tool_calls: None,
            },
            finish_reason: Some(finish_reason(value)),
        }],
        usage: parse_usage(value.get("usage")),
    })
}

fn output_text(value: &Value) -> String {
    let mut text = String::new();
    for item in array_field(value, "output") {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        for part in array_field(item, "content") {
            if part.get("type").and_then(Value::as_str) == Some("output_text") {
                if let Some(chunk) = part.get("text").and_then(Value::as_str) {
                    text.push_str(chunk);
                }
            }
        }
    }
    text
}

fn finish_reason(value: &Value) -> String {
    match value
        .get("incomplete_details")
        .and_then(|details| details.get("reason"))
        .and_then(Value::as_str)
    {
        Some("max_output_tokens" | "max_messages") => "length".to_string(),
        Some("content_filter" | "steered") => "content_filter".to_string(),
        Some(other) => other.to_string(),
        None => "stop".to_string(),
    }
}

fn parse_usage(usage: Option<&Value>) -> Option<UsageInfo> {
    let usage = usage?;
    Some(UsageInfo {
        prompt_tokens: count(usage, "input_tokens"),
        completion_tokens: count(usage, "output_tokens"),
        total_tokens: count(usage, "total_tokens"),
    })
}

fn count(value: &Value, key: &str) -> u32 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default() as u32
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn array_field<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

pub(crate) fn translate_stream<S, E>(
    events: S,
) -> impl futures_util::Stream<Item = Result<StreamEvent, UpstreamCallError>> + Send
where
    S: futures_util::Stream<
            Item = Result<eventsource_stream::Event, eventsource_stream::EventStreamError<E>>,
        > + Send
        + Unpin,
    E: std::fmt::Display,
{
    use futures_util::StreamExt;

    events
        .map(|event| match event {
            Ok(message) => translate_event(&message.data),
            Err(err) => Some(Err(UpstreamCallError::Transport(err.to_string()))),
        })
        .filter_map(|item| async move { item })
}

pub(crate) fn translate_event(data: &str) -> Option<Result<StreamEvent, UpstreamCallError>> {
    let value: Value = serde_json::from_str(data).ok()?;

    match value.get("type").and_then(Value::as_str) {
        Some("response.output_text.delta") => {
            let delta = value
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(Ok(chunk_event(delta, None, None)))
        }
        Some("response.completed") | Some("response.incomplete") => {
            let usage = value
                .get("response")
                .and_then(|response| response.get("usage"))
                .and_then(|usage| parse_usage(Some(usage)));
            Some(Ok(chunk_event(String::new(), None, usage)))
        }
        Some("response.failed") => Some(Err(UpstreamCallError::Protocol(
            failure_message(&value).to_string(),
        ))),
        Some("error") => Some(Err(UpstreamCallError::Protocol(
            value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the upstream stream reported an error")
                .to_string(),
        ))),
        _ => None,
    }
}

fn failure_message(value: &Value) -> &str {
    value
        .get("response")
        .and_then(|response| response.get("error"))
        .and_then(|error| error.get("message"))
        .or_else(|| value.get("error").and_then(|error| error.get("message")))
        .and_then(Value::as_str)
        .unwrap_or("the upstream reported a failed response")
}

fn chunk_event(
    delta_text: String,
    finish_reason: Option<String>,
    usage: Option<UsageInfo>,
) -> StreamEvent {
    StreamEvent::Chunk {
        data: json!({
            "object": "chat.completion.chunk",
            "choices": [{
                "index": 0,
                "delta": if delta_text.is_empty() {
                    json!({})
                } else {
                    json!({ "content": delta_text })
                },
                "finish_reason": finish_reason,
            }],
        }),
        delta_text: (!delta_text.is_empty()).then_some(delta_text),
        usage,
        finish_reason,
    }
}
