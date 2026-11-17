mod telemetry;

use std::{
    collections::VecDeque,
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU32},
        Arc,
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use telemetry::StreamTelemetry;

use axum::response::{
    sse::{Event, KeepAlive, Sse},
    IntoResponse, Response,
};
use futures_util::{Stream, StreamExt};
use tracing::warn;

use crate::{
    auth::limiter::RateLimitSnapshot,
    providers::{
        adapter_for, ChatCompletionRequest, ProviderEventStream, StreamEvent, UpstreamCallError,
    },
    routing::{CircuitBreakerRegistry, ProviderKeyPool, RoutingTarget},
    telemetry::{RoutingAttempt, TelemetryEvent},
    AppState,
};

use super::{
    failover::{self, AttemptContext},
    headers::{attach_gateway_headers, attach_rate_limit_headers},
};

pub(crate) const MAX_RESPONSE_ACCUMULATOR: usize = 64 * 1024;

const MAX_STREAM_ROTATIONS: u32 = 1;
const STREAM_ROTATION_BACKOFF: Duration = Duration::from_millis(100);

pub(crate) struct OpenedStream {
    pub stream: ProviderEventStream,
    pub first_event: Option<StreamEvent>,
    pub target: RoutingTarget,
    pub ttft_ms: f64,
}

pub(crate) async fn open_stream(
    state: &AppState,
    target: &RoutingTarget,
    request: &ChatCompletionRequest,
) -> Result<OpenedStream, UpstreamCallError> {
    let opened_at = Instant::now();
    let mut stream = open_upstream(state, target, request).await?;

    match stream.next().await {
        Some(Ok(event)) => Ok(OpenedStream {
            stream,
            first_event: Some(event),
            target: target.clone(),
            ttft_ms: opened_at.elapsed().as_secs_f64() * 1000.0,
        }),
        Some(Err(err)) => Err(err),
        None => Err(UpstreamCallError::Protocol(
            "upstream stream ended before any event".into(),
        )),
    }
}

async fn open_upstream(
    state: &AppState,
    target: &RoutingTarget,
    request: &ChatCompletionRequest,
) -> Result<ProviderEventStream, UpstreamCallError> {
    adapter_for(target.kind)
        .stream(&state.http_client, &target.endpoint(), request)
        .await
}

pub(crate) struct StreamSetup {
    pub telemetry: TelemetryEvent,
    pub attempts: Vec<RoutingAttempt>,
    pub fallback_count: u32,
    pub estimated_input_tokens: AtomicU32,
    pub redact: bool,
    pub unit_costs: Option<(f64, f64)>,
    pub request_id: String,
    pub rate_limits: RateLimitSnapshot,
    pub request: ChatCompletionRequest,
    pub circuit: Arc<CircuitBreakerRegistry>,
}

pub(crate) type Formatter = Box<dyn FnMut(&StreamEvent) -> Vec<Event> + Send>;

pub(crate) fn stream_response(
    state: AppState,
    opened: OpenedStream,
    setup: StreamSetup,
    format: Formatter,
) -> Response {
    let request_id = setup.request_id;
    let rate_limits = setup.rate_limits;
    let fallbacks = setup.fallback_count;
    let target = opened.target.clone();

    let tracker = StreamTelemetry {
        state: state.clone(),
        event: setup.telemetry,
        attempts: setup.attempts,
        fallback_count: fallbacks,
        target: opened.target.clone(),
        started: Instant::now(),
        ttft_ms: Some(opened.ttft_ms),
        accumulator: String::new(),
        usage: None,
        redact: setup.redact,
        estimated_input_tokens: Arc::new(setup.estimated_input_tokens),
        unit_costs: setup.unit_costs,
        finished: Arc::new(AtomicBool::new(false)),
        error: None,
    };
    let request = setup.request;
    let key_id = tracker.event.virtual_key_id.clone();
    let rotate = RotateCtx {
        state: state.clone(),
        circuit: setup.circuit,
        request,
        target: opened.target.clone(),
        key_id,
        rotations: 0,
    };
    let body = ManagedStream::new(
        opened.stream,
        opened.first_event,
        Some(tracker),
        format,
        Some(rotate),
    );

    let mut response = Sse::new(body)
        .keep_alive(KeepAlive::default())
        .into_response();
    let headers = response.headers_mut();
    attach_gateway_headers(
        headers,
        &request_id,
        &target.provider_name,
        &target.upstream_model,
        fallbacks,
        None,
    );
    attach_rate_limit_headers(headers, rate_limits);
    response
}

struct RotateCtx {
    state: AppState,
    circuit: Arc<CircuitBreakerRegistry>,
    request: ChatCompletionRequest,
    target: RoutingTarget,
    key_id: Option<String>,
    rotations: u32,
}

impl RotateCtx {
    fn attempt_ctx(&self) -> AttemptContext<'_> {
        AttemptContext {
            state: &self.state,
            request_model: &self.request.model,
            scope: &self.request.model,
            session_id: None,
            key_id: self.key_id.as_deref(),
            circuit: &self.circuit,
            ewma: &self.state.ewma,
            key_pool: &self.state.key_pool,
            affinity: &self.state.affinity,
        }
    }
}

type Retried = (Reopened, RotateCtx, StreamTelemetry);

enum Reopened {
    Stream(Box<OpenedStream>),
    Failed(UpstreamCallError),
}

enum Stage {
    Streaming,
    Retrying(Pin<Box<dyn Future<Output = Retried> + Send>>),
}

pub(crate) struct ManagedStream {
    inner: ProviderEventStream,
    first: Option<StreamEvent>,
    tracker: Option<StreamTelemetry>,
    format: Formatter,
    pending: VecDeque<Event>,
    rotate: Option<RotateCtx>,
    stage: Stage,
}

async fn reopen(
    rotate: &mut RotateCtx,
    tracker: &mut StreamTelemetry,
    err: &UpstreamCallError,
) -> Reopened {
    let failed = rotate.target.selected_key_id.clone();
    failover::note_failure(
        &rotate.attempt_ctx(),
        &rotate.target,
        failed.as_deref(),
        err,
    );
    tracker.record_attempt(
        &rotate.target,
        err.status_code(),
        0.0,
        Some(&err.to_string()),
    );

    tokio::time::sleep(STREAM_ROTATION_BACKOFF).await;

    let Some((key_id, key_name, api_key)) = rotate_key(rotate, failed.as_deref()).await else {
        return Reopened::Failed(err.clone());
    };
    let mut bound = rotate.target.clone();
    bound.selected_key_id = Some(key_id.clone());
    bound.selected_key_name = Some(key_name);
    bound.api_key = api_key;

    let opened_at = Instant::now();
    match open_upstream(&rotate.state, &bound, &rotate.request).await {
        Ok(mut stream) => match stream.next().await {
            Some(Ok(first)) => {
                let ttft_ms = opened_at.elapsed().as_secs_f64() * 1000.0;
                failover::note_success(&rotate.attempt_ctx(), &bound, Some(&key_id), ttft_ms);
                tracker.record_attempt(&bound, 200, ttft_ms, None);
                rotate.target = bound.clone();
                Reopened::Stream(Box::new(OpenedStream {
                    stream,
                    first_event: Some(first),
                    target: bound,
                    ttft_ms,
                }))
            }
            Some(Err(next)) => {
                let latency_ms = opened_at.elapsed().as_secs_f64() * 1000.0;
                failover::note_failure(&rotate.attempt_ctx(), &bound, Some(&key_id), &next);
                tracker.record_attempt(
                    &bound,
                    next.status_code(),
                    latency_ms,
                    Some(&next.to_string()),
                );
                Reopened::Failed(next)
            }
            None => {
                let next =
                    UpstreamCallError::Protocol("upstream stream ended before any event".into());
                let latency_ms = opened_at.elapsed().as_secs_f64() * 1000.0;
                failover::note_failure(&rotate.attempt_ctx(), &bound, Some(&key_id), &next);
                tracker.record_attempt(
                    &bound,
                    next.status_code(),
                    latency_ms,
                    Some(&next.to_string()),
                );
                Reopened::Failed(next)
            }
        },
        Err(next) => {
            let latency_ms = opened_at.elapsed().as_secs_f64() * 1000.0;
            failover::note_failure(&rotate.attempt_ctx(), &bound, Some(&key_id), &next);
            tracker.record_attempt(
                &bound,
                next.status_code(),
                latency_ms,
                Some(&next.to_string()),
            );
            Reopened::Failed(next)
        }
    }
}

async fn rotate_key(rotate: &RotateCtx, failed: Option<&str>) -> Option<(String, String, String)> {
    let pool: &ProviderKeyPool = &rotate.state.key_pool;
    let entries = pool.entries(&rotate.target.provider_id).await;
    let model = rotate.target.upstream_model.as_str();
    entries
        .iter()
        .filter(|entry| Some(entry.id.as_str()) != failed)
        .filter(|entry| !entry.api_key.trim().is_empty() && entry.serves(model))
        .filter(|entry| !pool.in_cooldown(&entry.id))
        .max_by_key(|entry| entry.weight)
        .map(|entry| (entry.id.clone(), entry.name.clone(), entry.api_key.clone()))
}

fn translate(
    tracker: Option<&mut StreamTelemetry>,
    format: &mut Formatter,
    pending: &mut VecDeque<Event>,
    event: &StreamEvent,
) {
    if let Some(tracker) = tracker {
        tracker.observe(event);
    }
    pending.extend(format(event));
}

fn finish(tracker: &mut Option<StreamTelemetry>) {
    if let Some(mut tracker) = tracker.take() {
        tokio::spawn(async move { tracker.finish().await });
    }
}

fn error_event(err: &UpstreamCallError) -> Event {
    let payload = serde_json::json!({
        "error": { "message": err.to_string(), "type": "upstream_error" },
    });
    Event::default().json_data(payload).unwrap_or_default()
}

impl ManagedStream {
    fn new(
        inner: ProviderEventStream,
        first: Option<StreamEvent>,
        tracker: Option<StreamTelemetry>,
        format: Formatter,
        rotate: Option<RotateCtx>,
    ) -> Self {
        Self {
            inner,
            first,
            tracker,
            format,
            pending: VecDeque::new(),
            rotate,
            stage: Stage::Streaming,
        }
    }

    fn interrupt(&mut self, err: &UpstreamCallError) -> Poll<Option<Result<Event, Infallible>>> {
        let retryable = err.is_retryable()
            && self.tracker.is_some()
            && self
                .rotate
                .as_ref()
                .is_some_and(|rotate| rotate.rotations < MAX_STREAM_ROTATIONS);
        if !retryable {
            return self.give_up(err);
        }
        let (Some(mut rotate), Some(mut tracker)) = (self.rotate.take(), self.tracker.take())
        else {
            return self.give_up(err);
        };
        rotate.rotations += 1;
        let err = err.clone();
        self.stage = Stage::Retrying(Box::pin(async move {
            let reopened = reopen(&mut rotate, &mut tracker, &err).await;
            (reopened, rotate, tracker)
        }));
        Poll::Pending
    }

    fn give_up(&mut self, err: &UpstreamCallError) -> Poll<Option<Result<Event, Infallible>>> {
        warn!(error = %err, "upstream stream interrupted");
        if let Some(tracker) = self.tracker.as_mut() {
            tracker.error = Some(err.clone());
        }
        finish(&mut self.tracker);
        Poll::Ready(Some(Ok(error_event(err))))
    }

    fn adopt(&mut self, opened: OpenedStream) {
        self.inner = opened.stream;
        self.first = opened.first_event;
        if let Some(tracker) = self.tracker.as_mut() {
            tracker.target = opened.target;
        }
        self.stage = Stage::Streaming;
    }
}

impl Stream for ManagedStream {
    type Item = Result<Event, std::convert::Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(event) = this.pending.pop_front() {
                return Poll::Ready(Some(Ok(event)));
            }

            if let Stage::Retrying(rotation) = &mut this.stage {
                match rotation.as_mut().poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready((reopened, rotate, tracker)) => {
                        this.rotate = Some(rotate);
                        this.tracker = Some(tracker);
                        match reopened {
                            Reopened::Stream(opened) => this.adopt(*opened),
                            Reopened::Failed(err) => return this.give_up(&err),
                        }
                        continue;
                    }
                }
            }

            if let Some(event) = this.first.take() {
                translate(
                    this.tracker.as_mut(),
                    &mut this.format,
                    &mut this.pending,
                    &event,
                );
                if matches!(event, StreamEvent::Done) {
                    finish(&mut this.tracker);
                }
                continue;
            }

            match this.inner.as_mut().poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    finish(&mut this.tracker);
                    return Poll::Ready(None);
                }
                Poll::Ready(Some(Ok(event))) => {
                    translate(
                        this.tracker.as_mut(),
                        &mut this.format,
                        &mut this.pending,
                        &event,
                    );
                    if matches!(event, StreamEvent::Done) {
                        finish(&mut this.tracker);
                    }
                }
                Poll::Ready(Some(Err(err))) => return this.interrupt(&err),
            }
        }
    }
}
