use serde_json::{json, Value};

use super::super::join_url;
use super::super::support::strip_trailing_segment;
use super::super::types::{
    EmbeddingData, EmbeddingRequest, EmbeddingResponse, EmbeddingUsage, UpstreamCallError,
};

pub(crate) fn embeddings_url(base_url: &str) -> String {
    let base = strip_trailing_segment(base_url, "/v1");
    join_url(base, "/v1/embeddings")
}

pub(crate) fn decisions_url(base_url: &str) -> String {
    let base = strip_trailing_segment(base_url, "/v1");
    join_url(base, "/v1/decisions")
}

pub(crate) fn build_embedding_body(req: &EmbeddingRequest, upstream_model: &str) -> Value {
    let mut body = json!({
        "model": upstream_model,
        "input": req.input,
    });

    if let Some(map) = body.as_object_mut() {
        if let Some(format) = &req.encoding_format {
            map.insert("encoding_format".into(), json!(format));
        }
        if let Some(dimensions) = req.dimensions {
            map.insert("dimensions".into(), json!(dimensions));
        }
        if let Some(user) = &req.user {
            map.insert("user".into(), json!(user));
        }
    }

    body
}

pub(crate) fn parse_embedding_response(
    value: &Value,
    requested_model: &str,
) -> Result<EmbeddingResponse, UpstreamCallError> {
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string());
        return Err(UpstreamCallError::Protocol(format!(
            "embedding error: {message}"
        )));
    }

    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| UpstreamCallError::Protocol("embedding body without 'data'".to_string()))?;

    let mut embeddings = Vec::with_capacity(data.len());
    for (position, entry) in data.iter().enumerate() {
        let embedding = entry.get("embedding").cloned().ok_or_else(|| {
            UpstreamCallError::Protocol("embedding entry without 'embedding'".to_string())
        })?;
        embeddings.push(EmbeddingData {
            object: entry
                .get("object")
                .and_then(Value::as_str)
                .unwrap_or("embedding")
                .to_string(),
            embedding,
            index: entry
                .get("index")
                .and_then(Value::as_u64)
                .map(|index| index as usize)
                .unwrap_or(position),
        });
    }

    let prompt_tokens = value
        .get("usage")
        .and_then(|usage| usage.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let total_tokens = value
        .get("usage")
        .and_then(|usage| usage.get("total_tokens"))
        .and_then(Value::as_u64)
        .map(|total| total as u32)
        .unwrap_or(prompt_tokens);

    Ok(EmbeddingResponse {
        object: value
            .get("object")
            .and_then(Value::as_str)
            .unwrap_or("list")
            .to_string(),
        data: embeddings,
        model: value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(requested_model)
            .to_string(),
        usage: EmbeddingUsage {
            prompt_tokens,
            total_tokens,
        },
    })
}
