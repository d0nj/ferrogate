use std::{
    fmt::Write as _,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use dashmap::DashMap;

use crate::telemetry::TelemetryEvent;

pub const LATENCY_BUCKETS: [f64; 10] = [0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0];

const UNKNOWN: &str = "unknown";

const MAX_REQUEST_SERIES: usize = 2_000;

const OVERFLOW: &str = "__other__";

#[derive(Debug)]
pub struct Histogram {
    counts: [AtomicU64; LATENCY_BUCKETS.len()],
    overflow: AtomicU64,
    sum_micros: AtomicU64,
    total: AtomicU64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
            overflow: AtomicU64::new(0),
            sum_micros: AtomicU64::new(0),
            total: AtomicU64::new(0),
        }
    }
}

impl Histogram {
    pub fn observe(&self, seconds: f64) {
        if !seconds.is_finite() || seconds < 0.0 {
            return;
        }

        let bucket = LATENCY_BUCKETS
            .iter()
            .position(|bound| seconds <= *bound)
            .unwrap_or(LATENCY_BUCKETS.len());
        match bucket < LATENCY_BUCKETS.len() {
            true => self.counts[bucket].fetch_add(1, Ordering::Relaxed),
            false => self.overflow.fetch_add(1, Ordering::Relaxed),
        };
        self.sum_micros
            .fetch_add((seconds * 1_000_000.0).round() as u64, Ordering::Relaxed);
        self.total.fetch_add(1, Ordering::Relaxed);
    }

    fn render(&self, out: &mut String, name: &str, labels: &str) {
        let mut cumulative = 0_u64;
        for (bound, count) in LATENCY_BUCKETS.iter().zip(self.counts.iter()) {
            cumulative += count.load(Ordering::Relaxed);
            render_sample(
                out,
                &format!("{name}_bucket"),
                &with_le(labels, *bound),
                cumulative,
            );
        }
        cumulative += self.overflow.load(Ordering::Relaxed);
        render_sample(
            out,
            &format!("{name}_bucket"),
            &with_le(labels, f64::INFINITY),
            cumulative,
        );
        render_float(
            out,
            &format!("{name}_sum"),
            labels,
            self.sum_micros.load(Ordering::Relaxed) as f64 / 1_000_000.0,
        );
        render_sample(
            out,
            &format!("{name}_count"),
            labels,
            self.total.load(Ordering::Relaxed),
        );
    }
}

#[derive(Debug, Default)]
pub struct Metrics {
    requests: DashMap<(i64, String, String), u64>,
    latency: DashMap<String, Histogram>,
    ttft: DashMap<String, Histogram>,
    tokens: DashMap<(String, String), u64>,
    spend_micros: DashMap<String, AtomicU64>,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    guardrail_blocks: DashMap<String, u64>,
    audit_failures: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, event: &TelemetryEvent) {
        let provider = event
            .routed_provider_name
            .clone()
            .unwrap_or_else(|| UNKNOWN.to_string());
        let model = event
            .routed_model
            .clone()
            .unwrap_or_else(|| event.requested_model.clone());

        let series = (event.status_code, provider.clone(), model);
        if self.requests.contains_key(&series) {
            self.requests.entry(series).and_modify(|count| *count += 1);
        } else if self.requests.len() < MAX_REQUEST_SERIES {
            self.requests.insert(series, 1);
        } else {
            let fallback = (event.status_code, provider.clone(), OVERFLOW.to_string());
            self.requests
                .entry(fallback)
                .and_modify(|count| *count += 1)
                .or_insert(1);
        }

        self.latency
            .entry(provider.clone())
            .or_default()
            .observe(event.latency_ms.max(0.0) / 1000.0);

        if let Some(ttft_ms) = event
            .ttft_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.ttft
                .entry(provider.clone())
                .or_default()
                .observe(ttft_ms / 1000.0);
        }

        if event.input_tokens > 0 {
            self.add_tokens("input", &provider, u64::from(event.input_tokens));
        }
        if event.output_tokens > 0 {
            self.add_tokens("output", &provider, u64::from(event.output_tokens));
        }

        if let Some(key_id) = event.virtual_key_id.as_deref() {
            self.spend_micros
                .entry(key_id.to_string())
                .or_default()
                .fetch_add(usd_to_micros(event.cost_usd), Ordering::Relaxed);
        }

        if event.cache_hit {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
        } else if event.cache_type.is_some() {
            self.cache_misses.fetch_add(1, Ordering::Relaxed);
        }

        if let Some(action) = event.guardrail_action.as_deref() {
            for rule in guardrail_rules(action) {
                self.guardrail_blocks
                    .entry(rule)
                    .and_modify(|count| *count += 1)
                    .or_insert(1);
            }
        }
    }

    pub fn record_block(&self, rule_name: &str) {
        self.guardrail_blocks
            .entry(rule_name.to_string())
            .and_modify(|count| *count += 1)
            .or_insert(1);
    }

    pub fn record_audit_failure(&self) {
        self.audit_failures.fetch_add(1, Ordering::Relaxed);
    }

    fn add_tokens(&self, direction: &str, provider: &str, tokens: u64) {
        self.tokens
            .entry((direction.to_string(), provider.to_string()))
            .and_modify(|count| *count += tokens)
            .or_insert(tokens);
    }

    pub fn render(&self) -> String {
        let mut out = String::with_capacity(4096);

        out.push_str("# TYPE ferrogate_requests_total counter\n");
        for entry in self.requests.iter() {
            let (status, provider, model) = entry.key();
            let labels = format!(
                "status=\"{}\",provider=\"{}\",model=\"{}\"",
                escape(&status.to_string()),
                escape(provider),
                escape(model),
            );
            render_sample(
                &mut out,
                "ferrogate_requests_total",
                &labels,
                *entry.value(),
            );
        }

        out.push_str("# TYPE ferrogate_request_duration_seconds histogram\n");
        for entry in self.latency.iter() {
            let labels = format!("provider=\"{}\"", escape(entry.key()));
            entry
                .value()
                .render(&mut out, "ferrogate_request_duration_seconds", &labels);
        }

        out.push_str("# TYPE ferrogate_ttft_seconds histogram\n");
        for entry in self.ttft.iter() {
            let labels = format!("provider=\"{}\"", escape(entry.key()));
            entry
                .value()
                .render(&mut out, "ferrogate_ttft_seconds", &labels);
        }

        out.push_str("# TYPE ferrogate_tokens_total counter\n");
        for entry in self.tokens.iter() {
            let (direction, provider) = entry.key();
            let labels = format!(
                "direction=\"{}\",provider=\"{}\"",
                escape(direction),
                escape(provider),
            );
            render_sample(&mut out, "ferrogate_tokens_total", &labels, *entry.value());
        }

        out.push_str("# TYPE ferrogate_spend_usd_total counter\n");
        for entry in self.spend_micros.iter() {
            let labels = format!("virtual_key=\"{}\"", escape(entry.key()));
            render_float(
                &mut out,
                "ferrogate_spend_usd_total",
                &labels,
                entry.value().load(Ordering::Relaxed) as f64 / 1_000_000.0,
            );
        }

        out.push_str("# TYPE ferrogate_audit_failures_total counter\n");
        render_sample(
            &mut out,
            "ferrogate_audit_failures_total",
            "",
            self.audit_failures.load(Ordering::Relaxed),
        );

        out.push_str("# TYPE ferrogate_telemetry_drops_total counter\n");
        render_sample(
            &mut out,
            "ferrogate_telemetry_drops_total",
            "",
            crate::telemetry::dropped_events(),
        );

        out.push_str("# TYPE ferrogate_cache_hits_total counter\n");
        render_sample(
            &mut out,
            "ferrogate_cache_hits_total",
            "",
            self.cache_hits.load(Ordering::Relaxed),
        );
        out.push_str("# TYPE ferrogate_cache_misses_total counter\n");
        render_sample(
            &mut out,
            "ferrogate_cache_misses_total",
            "",
            self.cache_misses.load(Ordering::Relaxed),
        );

        out.push_str("# TYPE ferrogate_guardrail_blocks_total counter\n");
        for entry in self.guardrail_blocks.iter() {
            let labels = format!("rule=\"{}\"", escape(entry.key()));
            render_sample(
                &mut out,
                "ferrogate_guardrail_blocks_total",
                &labels,
                *entry.value(),
            );
        }

        out
    }

    pub async fn snapshot(&self, state: &crate::AppState) -> String {
        let mut out = self.render();

        match state.routing.health_snapshots().await {
            Ok(snapshots) => Self::render_providers(
                &mut out,
                snapshots
                    .iter()
                    .map(|snap| (snap.name.as_str(), snap.state == "closed")),
            ),
            Err(err) => tracing::debug!(error = %err, "provider health unavailable for /metrics"),
        }

        out
    }

    pub fn render_providers<'a>(out: &mut String, health: impl Iterator<Item = (&'a str, bool)>) {
        out.push_str("# TYPE ferrogate_provider_up gauge\n");
        for (provider, up) in health {
            let labels = format!("provider=\"{}\"", escape(provider));
            render_sample(out, "ferrogate_provider_up", &labels, u64::from(up));
        }
    }
}

pub type SharedMetrics = Arc<Metrics>;

fn guardrail_rules(action: &str) -> Vec<String> {
    action
        .split(';')
        .flat_map(|part| part.rsplit(':').next().unwrap_or(part).split(','))
        .map(str::trim)
        .filter(|rule| !rule.is_empty())
        .map(str::to_string)
        .collect()
}

fn usd_to_micros(cost_usd: f64) -> u64 {
    match cost_usd.is_finite() && cost_usd > 0.0 {
        true => (cost_usd * 1_000_000.0).round() as u64,
        false => 0,
    }
}

fn with_le(labels: &str, bound: f64) -> String {
    let le = match bound.is_infinite() {
        true => "+Inf".to_string(),
        false => bound.to_string(),
    };
    match labels.is_empty() {
        true => format!("le=\"{le}\""),
        false => format!("{labels},le=\"{le}\""),
    }
}

fn render_sample(out: &mut String, name: &str, labels: &str, value: u64) {
    let _ = match labels.is_empty() {
        true => writeln!(out, "{name} {value}"),
        false => writeln!(out, "{name}{{{labels}}} {value}"),
    };
}

fn render_float(out: &mut String, name: &str, labels: &str, value: f64) {
    let _ = match labels.is_empty() {
        true => writeln!(out, "{name} {value}"),
        false => writeln!(out, "{name}{{{labels}}} {value}"),
    };
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests;
