use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
    time::Instant,
};

use crate::{
    providers::{StreamEvent, UpstreamCallError, UsageInfo},
    routing::RoutingTarget,
    telemetry::{RoutingAttempt, TelemetryEvent},
    tokenizer::count_text_tokens,
    AppState,
};

use super::super::{chat::cost_from_units, failover, streaming::MAX_RESPONSE_ACCUMULATOR};

pub(super) struct StreamTelemetry {
    pub(super) state: AppState,
    pub(super) event: TelemetryEvent,
    pub(super) attempts: Vec<RoutingAttempt>,
    pub(super) fallback_count: u32,
    pub(super) target: RoutingTarget,
    pub(super) started: Instant,
    pub(super) ttft_ms: Option<f64>,
    pub(super) accumulator: String,
    pub(super) usage: Option<UsageInfo>,
    pub(super) redact: bool,
    pub(super) estimated_input_tokens: Arc<AtomicU32>,
    pub(super) unit_costs: Option<(f64, f64)>,
    pub(super) finished: Arc<AtomicBool>,
    pub(super) error: Option<UpstreamCallError>,
}

impl StreamTelemetry {
    pub(super) fn observe(&mut self, event: &StreamEvent) {
        let StreamEvent::Chunk {
            delta_text, usage, ..
        } = event
        else {
            return;
        };

        if self.ttft_ms.is_none() && delta_text.as_ref().is_some_and(|text| !text.is_empty()) {
            self.ttft_ms = Some(self.started.elapsed().as_secs_f64() * 1000.0);
        }

        if let Some(text) = delta_text {
            if !text.is_empty() && self.accumulator.len() < MAX_RESPONSE_ACCUMULATOR {
                let remaining = MAX_RESPONSE_ACCUMULATOR - self.accumulator.len();
                self.accumulator.extend(text.chars().take(remaining));
            }
        }

        if let Some(usage) = usage {
            self.usage = Some(usage.clone());
        }
    }

    pub(super) fn record_attempt(
        &mut self,
        target: &RoutingTarget,
        status_code: u16,
        latency_ms: f64,
        error: Option<&str>,
    ) {
        self.attempts
            .push(failover::trace(target, status_code, latency_ms, error));
        self.fallback_count = self.fallback_count.saturating_add(1);
    }

    pub(super) async fn finish(&mut self) {
        let Some(record) = self.record() else {
            return;
        };

        let filtered = match record.text.as_deref() {
            None => None,
            Some(text) => {
                let verdict = self
                    .state
                    .guardrails
                    .filter_output_cached(
                        text,
                        self.event.virtual_key_id.as_deref(),
                        "/v1/chat/completions",
                    )
                    .await;
                if verdict.redacted {
                    tracing::info!(
                        request_id = %record.event.id,
                        rules = ?verdict.triggered,
                        "guardrail.redacted"
                    );
                }
                Some(verdict.text)
            }
        };

        record.emit(filtered).await;
    }

    fn record(&mut self) -> Option<RawRecord> {
        if self.finished.swap(true, Ordering::SeqCst) {
            return None;
        }

        let input_tokens = self
            .usage
            .as_ref()
            .map(|usage| usage.prompt_tokens)
            .unwrap_or_else(|| self.estimated_input_tokens.load(Ordering::Relaxed));

        let output_tokens = self
            .usage
            .as_ref()
            .map(|usage| usage.completion_tokens)
            .unwrap_or_else(|| count_text_tokens(&self.target.upstream_model, &self.accumulator));

        let mut event = self.event.clone();
        if let Some(err) = &self.error {
            event.status_code = i64::from(err.status_code());
            event.error_message = Some(err.to_string());
        } else {
            event.status_code = 200;
        }
        event.input_tokens = input_tokens;
        event.output_tokens = output_tokens;
        event.cost_usd = cost_from_units(self.unit_costs, input_tokens, output_tokens);
        event.ttft_ms = self.ttft_ms;
        event.latency_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        event.fallback_count = self.fallback_count;
        event.routing_trace = self.attempts.clone();
        event.routed_provider_id = Some(self.target.provider_id.clone());
        event.routed_provider_name = Some(self.target.provider_name.clone());
        event.routed_model = Some(self.target.upstream_model.clone());

        failover::book_spend(&self.state, event.virtual_key_id.as_deref(), event.cost_usd);

        let text = match self.redact || self.accumulator.is_empty() {
            true => None,
            false => Some(self.accumulator.clone()),
        };

        Some(RawRecord {
            state: self.state.clone(),
            event,
            text,
        })
    }

    fn finish_detached(&mut self) {
        let Some(record) = self.record() else {
            return;
        };
        record.state.metrics.record(&record.event);
        record.state.telemetry.record(record.event);
    }
}

struct RawRecord {
    state: AppState,
    event: TelemetryEvent,
    text: Option<String>,
}

impl RawRecord {
    async fn emit(self, filtered: Option<String>) {
        let mut event = self.event;
        if let Some(text) = filtered {
            event.response_text = Some(text);
        }
        self.state.metrics.record(&event);
        self.state.telemetry.record(event);
    }
}

impl Drop for StreamTelemetry {
    fn drop(&mut self) {
        self.finish_detached();
    }
}
