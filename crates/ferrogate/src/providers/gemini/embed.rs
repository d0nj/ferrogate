use serde_json::{json, Value};

use super::super::join_url;
use super::super::types::{
    EmbeddingData, EmbeddingRequest, EmbeddingResponse, EmbeddingUsage, UpstreamCallError,
};
use super::normalize_base;

pub(crate) fn embedding_url(base_url: &str, model: &str, batch: bool) -> String {
    let method = if batch {
        "batchEmbedContents"
    } else {
        "embedContent"
    };
    join_url(
        normalize_base(base_url),
        &format!("/v1beta/models/{model}:{method}"),
    )
}

pub(crate) fn decisions_url(base_url: &str) -> String {
    join_url(normalize_base(base_url), "/v1beta/decisions")
}

pub(crate) fn embedding_inputs(input: &Value) -> Result<Vec<String>, UpstreamCallError> {
    match input {
        Value::String(text) => Ok(vec![text.clone()]),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::String(text) => Ok(text.clone()),
                other => Err(UpstreamCallError::Status {
                    status: 400,
                    body: format!(
                        "{{\"error\":{{\"message\":\"gemini embeddings accept text input only, got {other}\",\"type\":\"invalid_request_error\"}}}}"
                    ),
                }),
            })
            .collect(),
        other => Err(UpstreamCallError::Status {
            status: 400,
            body: format!(
                "{{\"error\":{{\"message\":\"unsupported embedding input: {other}\",\"type\":\"invalid_request_error\"}}}}"
            ),
        }),
    }
}

pub(crate) fn build_embedding_body(
    model: &str,
    inputs: &[String],
    req: &EmbeddingRequest,
    batch: bool,
) -> Value {
    let request = |text: &str| {
        let mut entry = json!({
            "model": format!("models/{model}"),
            "content": { "parts": [{ "text": text }] },
        });
        if let Some(dimensions) = req.dimensions {
            entry["outputDimensionality"] = json!(dimensions);
        }
        entry
    };

    if batch {
        let requests: Vec<Value> = inputs.iter().map(|text| request(text)).collect();
        json!({ "requests": requests })
    } else {
        request(&inputs[0])
    }
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
            "Gemini embedding error: {message}"
        )));
    }

    let mut vectors: Vec<Value> = Vec::new();
    if let Some(embeddings) = value.get("embeddings").and_then(Value::as_array) {
        for entry in embeddings {
            if let Some(vector) = entry.get("values") {
                vectors.push(vector.clone());
            }
        }
    }
    if let Some(vector) = value
        .get("embedding")
        .and_then(|embedding| embedding.get("values"))
    {
        vectors.push(vector.clone());
    }

    if vectors.is_empty() {
        return Err(UpstreamCallError::Protocol(
            "Gemini embedding response without vectors".to_string(),
        ));
    }

    let data: Vec<EmbeddingData> = vectors
        .into_iter()
        .enumerate()
        .map(|(index, embedding)| EmbeddingData {
            object: "embedding".to_string(),
            embedding,
            index,
        })
        .collect();

    Ok(EmbeddingResponse {
        object: "list".to_string(),
        data,
        model: requested_model.to_string(),
        usage: EmbeddingUsage::default(),
    })
}
