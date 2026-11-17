//! Mock decision and embedding upstream endpoints.

use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use super::{injected, MockState};

/// `POST /v1/decisions` — OpenAI decision dialect.
pub(super) async fn openai_decisions(
    State(state): State<MockState>,
    Json(body): Json<Value>,
) -> Response {
    if let Some(response) = injected(&state) {
        return response;
    }
    Json(json!({
        "model": body["model"],
        "answers": [{
            "name": "harmful",
            "type": "predicate",
            "selected": false,
            "probability": 0.12,
            "confidence": 0.9
        }],
        "usage": { "input_tokens": 17, "output_tokens": 3, "total_tokens": 20 }
    }))
    .into_response()
}

/// `POST /v1/systemone` — TypeSafe decision dialect.
pub(super) async fn typesafe_decisions(
    State(state): State<MockState>,
    Json(_body): Json<Value>,
) -> Response {
    if let Some(response) = injected(&state) {
        return response;
    }
    Json(json!({
        "model": "safety-1",
        "answers": {
            "harmful": { "type": "noul", "noul": 0.08, "confidence": 0.95 }
        },
        "usage": { "input_tokens": 21, "output_tokens": 2 }
    }))
    .into_response()
}

/// `POST /v1beta/decisions` — Gemini decision dialect.
pub(super) async fn gemini_decisions(
    State(state): State<MockState>,
    Json(_body): Json<Value>,
) -> Response {
    if let Some(response) = injected(&state) {
        return response;
    }
    Json(json!({
        "answers": {
            "harmful": { "type": "noul", "value": true, "confidence": 0.7 }
        },
        "usage": {
            "total_input_tokens": 12,
            "total_output_tokens": 4,
            "total_tokens": 16
        }
    }))
    .into_response()
}

/// `POST /v1/embeddings` — OpenAI embeddings dialect.
pub(super) async fn openai_embeddings(
    State(state): State<MockState>,
    Json(body): Json<Value>,
) -> Response {
    if let Some(response) = injected(&state) {
        return response;
    }

    let count = match &body["input"] {
        Value::Array(items) => items.len().max(1),
        _ => 1,
    };
    let data: Vec<Value> = (0..count)
        .map(|index| {
            json!({
                "object": "embedding",
                "embedding": [0.1_f64 * (index as f64 + 1.0), 0.2, 0.3],
                "index": index,
            })
        })
        .collect();

    Json(json!({
        "object": "list",
        "data": data,
        "model": body["model"],
        "usage": { "prompt_tokens": 9 * count, "total_tokens": 9 * count },
    }))
    .into_response()
}
