use serde_json::{json, Value};

use super::types::{ProviderEndpoint, ProviderKind, UpstreamCallError};

pub(crate) fn chunk_payload(
    id: &str,
    model: &str,
    index: u32,
    delta: Option<Value>,
    finish_reason: Option<&str>,
) -> Value {
    json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": unix_timestamp(),
        "model": model,
        "choices": [{
            "index": index,
            "delta": delta.unwrap_or_else(|| json!({})),
            "finish_reason": finish_reason,
        }],
    })
}

pub(crate) fn apply_extra_headers(
    builder: reqwest::RequestBuilder,
    endpoint: &ProviderEndpoint,
) -> reqwest::RequestBuilder {
    let mut builder = builder;
    for (name, value) in &endpoint.extra_headers {
        let lower = name.trim().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "authorization"
                | "proxy-authorization"
                | "cookie"
                | "set-cookie"
                | "host"
                | "content-length"
                | "transfer-encoding"
                | "x-api-key"
                | "x-goog-api-key"
        ) {
            tracing::warn!(header = %name, "refusing reserved extra header override");
            continue;
        }
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder
}

pub(crate) struct DataUrl<'a> {
    pub(crate) mime: &'a str,
    pub(crate) data: &'a str,
}

pub(crate) fn parse_data_url(url: &str) -> Option<DataUrl<'_>> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    if mime.is_empty() || data.is_empty() {
        return None;
    }
    Some(DataUrl { mime, data })
}

pub(crate) fn image_part_url(part: &Value) -> Option<&str> {
    match part.get("image_url")? {
        Value::String(url) => Some(url.as_str()),
        Value::Object(map) => map.get("url").and_then(Value::as_str),
        _ => None,
    }
}

pub(crate) fn image_mime_from_url(url: &str) -> &'static str {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    match path.rsplit('.').next() {
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("heic") => "image/heic",
        _ => "image/jpeg",
    }
}

pub(crate) fn parse_tool_arguments(raw: Option<&Value>) -> Value {
    match raw {
        Some(Value::String(text)) => serde_json::from_str(text).unwrap_or_else(|_| json!({})),
        Some(value @ Value::Object(_)) => value.clone(),
        Some(Value::Null) | None => json!({}),
        Some(other) => other.clone(),
    }
}

pub(crate) fn thinking_budget(effort: Option<&str>) -> Option<u32> {
    match effort?.trim().to_ascii_lowercase().as_str() {
        "low" | "minimal" => Some(1024),
        "medium" => Some(4096),
        "high" => Some(8192),
        _ => None,
    }
}

pub(crate) fn gemini_thinking_budget(effort: Option<&str>) -> Option<u32> {
    match effort?.trim().to_ascii_lowercase().as_str() {
        "none" => Some(0),
        "minimal" | "low" => Some(1024),
        "medium" => Some(2048),
        "high" => Some(4096),
        _ => None,
    }
}

pub(crate) fn strip_trailing_segment<'a>(base: &'a str, segment: &str) -> &'a str {
    let trimmed = base.trim_end_matches('/');
    match trimmed.strip_suffix(segment) {
        Some(head) => head.trim_end_matches('/'),
        None => trimmed,
    }
}

pub fn join_url(base: &str, suffix: &str) -> String {
    let base = base.trim_end_matches('/');
    let suffix = suffix.trim_start_matches('/');

    if suffix.is_empty() {
        return base.to_string();
    }
    if base.is_empty() {
        return format!("/{suffix}");
    }
    format!("{base}/{suffix}")
}

pub fn rewrite_content(content: &mut Value, text: &str) {
    match content {
        Value::Array(parts) => {
            let mut written = false;
            for part in parts.iter_mut() {
                match part {
                    Value::String(_) if !written => {
                        *part = Value::String(text.to_string());
                        written = true;
                    }
                    Value::Object(map) if map.contains_key("text") && !written => {
                        map.insert("text".to_string(), Value::String(text.to_string()));
                        written = true;
                    }
                    _ => {}
                }
            }
            if !written {
                if let Some(first) = parts.first_mut() {
                    *first = Value::String(text.to_string());
                }
            }
        }
        _ => *content = Value::String(text.to_string()),
    }
}

pub fn message_content_to_text(content: &Value) -> String {
    match content {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                match part {
                    Value::String(text) => out.push_str(text),
                    Value::Object(map) => {
                        if let Some(text) = map.get("text").and_then(Value::as_str) {
                            out.push_str(text);
                        }
                    }
                    _ => {}
                }
            }
            out
        }
        Value::Object(map) => match map.get("text").and_then(Value::as_str) {
            Some(text) => text.to_string(),
            None => content.to_string(),
        },
        other => other.to_string(),
    }
}

pub(crate) fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn embeddings_unsupported(kind: ProviderKind) -> UpstreamCallError {
    UpstreamCallError::Status {
        status: 400,
        body: format!(
            "{{\"error\":{{\"message\":\"{} provider does not support /v1/embeddings\",\"type\":\"invalid_request_error\"}}}}",
            kind.as_str()
        ),
    }
}

pub(crate) fn decisions_unsupported(kind: ProviderKind) -> UpstreamCallError {
    UpstreamCallError::Status {
        status: 400,
        body: format!(
            "{{\"error\":{{\"message\":\"{} provider does not support decisions\",\"type\":\"invalid_request_error\"}}}}",
            kind.as_str()
        ),
    }
}
