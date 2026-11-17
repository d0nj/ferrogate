//! Anthropic, Gemini and failure-injection wire handlers of the mock upstream.

use std::{sync::atomic::Ordering, time::Duration};

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    Json,
};
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;

use super::{FailureMode, MockState};

pub(crate) fn injected(state: &MockState) -> Option<Response> {
    match state.failure_mode {
        FailureMode::None => None,
        FailureMode::Unavailable => Some(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": { "message": "upstream overloaded" } })),
            )
                .into_response(),
        ),
        FailureMode::RateLimited => Some(
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({ "error": { "message": "slow down" } })),
            )
                .into_response(),
        ),
    }
}

pub(super) async fn anthropic_messages(
    State(state): State<MockState>,
    Json(body): Json<Value>,
) -> Response {
    state.anthropic_calls.fetch_add(1, Ordering::SeqCst);
    if let Some(resp) = injected(&state) {
        return resp;
    }

    let model = body["model"]
        .as_str()
        .unwrap_or("claude-3-5-haiku-latest")
        .to_string();
    let stream = body["stream"].as_bool().unwrap_or(false);

    if !stream {
        return Json(json!({
            "id": "msg_anthropic_1",
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": [{ "type": "text", "text": "hello from anthropic" }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 13, "output_tokens": 6 }
        }))
        .into_response();
    }

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(8);
    tokio::spawn(async move {
        let frames: Vec<(&str, Value)> = vec![
            (
                "message_start",
                json!({"type":"message_start","message":{"id":"msg_anthropic_1","type":"message","role":"assistant",
                       "model":model,"content":[],"usage":{"input_tokens":13,"output_tokens":0}}}),
            ),
            (
                "content_block_start",
                json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            ),
            (
                "content_block_delta",
                json!({"type":"content_block_delta","index":0,
                       "delta":{"type":"text_delta","text":"hello "}}),
            ),
            (
                "content_block_delta",
                json!({"type":"content_block_delta","index":0,
                       "delta":{"type":"text_delta","text":"from anthropic"}}),
            ),
            (
                "content_block_stop",
                json!({"type":"content_block_stop","index":0}),
            ),
            (
                "message_delta",
                json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},
                       "usage":{"output_tokens":6}}),
            ),
            ("message_stop", json!({"type":"message_stop"})),
        ];

        for (name, frame) in frames {
            let _ = tx
                .send(Ok(Event::default().event(name).data(frame.to_string())))
                .await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });

    Sse::new(ReceiverStream::new(rx)).into_response()
}

pub(super) async fn gemini_dispatch(
    State(state): State<MockState>,
    axum::extract::Path(path): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if path.ends_with(":streamGenerateContent") {
        gemini_stream(State(state), Json(body)).await
    } else if path.ends_with(":generateContent") {
        gemini_generate(State(state), Json(body)).await
    } else {
        (
            StatusCode::NOT_FOUND,
            format!("unknown Gemini route: {path}"),
        )
            .into_response()
    }
}

async fn gemini_generate(State(state): State<MockState>, Json(_body): Json<Value>) -> Response {
    state.gemini_calls.fetch_add(1, Ordering::SeqCst);
    if let Some(resp) = injected(&state) {
        return resp;
    }

    Json(json!({
        "candidates": [{
            "content": { "role": "model", "parts": [{ "text": "hello from gemini" }] },
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 9,
            "candidatesTokenCount": 4,
            "totalTokenCount": 13
        }
    }))
    .into_response()
}

async fn gemini_stream(State(state): State<MockState>, Json(_body): Json<Value>) -> Response {
    state.gemini_calls.fetch_add(1, Ordering::SeqCst);
    if let Some(resp) = injected(&state) {
        return resp;
    }

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(8);
    tokio::spawn(async move {
        let frames = [
            json!({"candidates":[{"content":{"role":"model","parts":[{"text":"hello "}]}}]}),
            json!({"candidates":[{"content":{"role":"model","parts":[{"text":"from gemini"}]}}]}),
            json!({
                "candidates":[{"content":{"role":"model","parts":[{"text":""}]},"finishReason":"STOP"}],
                "usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":4,"totalTokenCount":13}
            }),
        ];

        for frame in frames {
            let _ = tx.send(Ok(Event::default().data(frame.to_string()))).await;
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    });

    Sse::new(ReceiverStream::new(rx)).into_response()
}
