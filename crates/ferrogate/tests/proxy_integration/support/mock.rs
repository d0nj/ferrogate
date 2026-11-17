//! Mock provider servers speaking the real OpenAI, Anthropic and Gemini wire formats.

#[path = "mock/mock_extra.rs"]
mod mock_extra;
#[path = "mock/wire.rs"]
mod wire;

pub(super) use wire::injected;

use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{
    extract::State,
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;

/// Failure injected into every upstream call while active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureMode {
    None,
    Unavailable,
    RateLimited,
}

#[derive(Clone)]
pub(super) struct MockState {
    pub(super) failure_mode: FailureMode,
    pub(super) openai_calls: Arc<AtomicUsize>,
    pub(super) anthropic_calls: Arc<AtomicUsize>,
    pub(super) gemini_calls: Arc<AtomicUsize>,
    pub(super) last_openai_request: Arc<std::sync::Mutex<Option<Value>>>,
}

/// A mock upstream listening on an ephemeral loopback port.
pub struct MockServer {
    addr: SocketAddr,
    state: MockState,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl MockServer {
    /// Start a mock with the given failure behaviour.
    pub async fn start(failure_mode: FailureMode) -> Self {
        let state = MockState {
            failure_mode,
            openai_calls: Arc::new(AtomicUsize::new(0)),
            anthropic_calls: Arc::new(AtomicUsize::new(0)),
            gemini_calls: Arc::new(AtomicUsize::new(0)),
            last_openai_request: Arc::new(std::sync::Mutex::new(None)),
        };

        let app = Router::new()
            .route("/v1/chat/completions", post(openai_chat))
            .route("/v1/messages", post(wire::anthropic_messages))
            .route("/v1/embeddings", post(mock_extra::openai_embeddings))
            .route("/v1/decisions", post(mock_extra::openai_decisions))
            .route("/v1/systemone", post(mock_extra::typesafe_decisions))
            .route("/v1beta/decisions", post(mock_extra::gemini_decisions))
            // axum cannot mix a parameter and a literal suffix in one segment,
            // so Gemini's `:action` verbs arrive through a catch-all.
            .route("/v1beta/models/{*path}", post(wire::gemini_dispatch))
            .route("/health", get(|| async { "ok" }))
            .with_state(state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        Self {
            addr,
            state,
            shutdown: Some(shutdown_tx),
        }
    }

    /// `http://127.0.0.1:<port>` base URL to seed into `providers.base_url`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Number of OpenAI chat calls served.
    pub fn openai_calls(&self) -> usize {
        self.state.openai_calls.load(Ordering::SeqCst)
    }

    /// Body of the most recent OpenAI chat request, for wire-level assertions.
    pub fn last_openai_body(&self) -> Value {
        self.state
            .last_openai_request
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(Value::Null)
    }
}

async fn openai_chat(State(state): State<MockState>, Json(body): Json<Value>) -> Response {
    state.openai_calls.fetch_add(1, Ordering::SeqCst);
    *state.last_openai_request.lock().unwrap() = Some(body.clone());

    let model = body["model"].as_str().unwrap_or("gpt-4o-mini").to_string();
    let stream = body["stream"].as_bool().unwrap_or(false);

    if !stream {
        if let Some(resp) = injected(&state) {
            return resp;
        }
        return Json(json!({
            "id": "chatcmpl-openai-1",
            "object": "chat.completion",
            "created": 1_700_000_000,
            "model": model,
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "hello from openai" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 11, "completion_tokens": 5, "total_tokens": 16 }
        }))
        .into_response();
    }

    if let Some(resp) = injected(&state) {
        return resp;
    }

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(8);
    tokio::spawn(async move {
        let frames = [
            json!({"id":"chatcmpl-openai-1","object":"chat.completion.chunk","created":1_700_000_000,"model":model,
                   "choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}),
            json!({"id":"chatcmpl-openai-1","object":"chat.completion.chunk","created":1_700_000_000,"model":model,
                   "choices":[{"index":0,"delta":{"content":"hello "},"finish_reason":null}]}),
            json!({"id":"chatcmpl-openai-1","object":"chat.completion.chunk","created":1_700_000_000,"model":model,
                   "choices":[{"index":0,"delta":{"content":"from openai"},"finish_reason":null}]}),
            json!({"id":"chatcmpl-openai-1","object":"chat.completion.chunk","created":1_700_000_000,"model":model,
                   "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            json!({"id":"chatcmpl-openai-1","object":"chat.completion.chunk","created":1_700_000_000,"model":model,
                   "choices":[], "usage":{"prompt_tokens":11,"completion_tokens":5,"total_tokens":16}}),
        ];

        for frame in frames {
            let _ = tx.send(Ok(Event::default().data(frame.to_string()))).await;
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
    });

    Sse::new(ReceiverStream::new(rx)).into_response()
}
