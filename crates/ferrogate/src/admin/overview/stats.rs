use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDateTime, SecondsFormat, TimeZone, Timelike, Utc};
use sea_orm::DatabaseConnection;
use serde_json::{json, Value};

use crate::{entities::provider, error::AppResult, AppState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    Hour,
    Day,
    Week,
}

impl Window {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim) {
            Some("1h") => Self::Hour,
            Some("7d") => Self::Week,
            _ => Self::Day,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hour => "1h",
            Self::Day => "24h",
            Self::Week => "7d",
        }
    }

    fn span(self) -> Duration {
        match self {
            Self::Hour => Duration::hours(1),
            Self::Day => Duration::hours(24),
            Self::Week => Duration::days(7),
        }
    }

    fn bucket(self) -> Duration {
        match self {
            Self::Hour => Duration::minutes(5),
            Self::Day => Duration::hours(1),
            Self::Week => Duration::days(1),
        }
    }

    fn bucket_count(self) -> i64 {
        self.span().num_seconds() / self.bucket().num_seconds()
    }

    fn bucket_format(self) -> &'static str {
        match self {
            Self::Hour => "%Y-%m-%dT%H:%M:00",
            Self::Day => "%Y-%m-%dT%H:00:00",
            Self::Week => "%Y-%m-%dT00:00:00",
        }
    }
}

fn bucket_start(timestamp: DateTime<Utc>, window: Window) -> DateTime<Utc> {
    let (minute, hour) = match window {
        Window::Hour => (Some(timestamp.minute() - timestamp.minute() % 5), None),
        Window::Day => (Some(0), None),
        Window::Week => (Some(0), Some(0)),
    };

    timestamp
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))
        .and_then(|value| minute.map_or(Some(value), |m| value.with_minute(m)))
        .and_then(|value| hour.map_or(Some(value), |h| value.with_hour(h)))
        .unwrap_or(timestamp)
}

fn bucket_label(start: DateTime<Utc>, window: Window) -> String {
    match window {
        Window::Hour | Window::Day => start.format("%H:%M").to_string(),
        Window::Week => start.format("%m-%d").to_string(),
    }
}

pub fn cutoff_iso(window: Window) -> String {
    (Utc::now() - window.span()).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn percentile(values: &[f64], pct: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = ((pct / 100.0) * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted.get(rank - 1).copied()
}

fn pct(part: f64, total: f64) -> f64 {
    if total <= 0.0 {
        0.0
    } else {
        ((part / total) * 10_000.0).round() / 100.0
    }
}

#[derive(Debug)]
pub struct AggregateRow {
    pub bucket: String,
    pub model: String,
    pub provider: String,
    pub requests: i64,
    pub errors: i64,
    pub fallbacks: i64,
    pub cache_hits: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub spend_usd: f64,
}

type AggregateTuple = (String, String, String, i64, i64, i64, i64, i64, i64, f64);

pub struct Aggregates {
    pub rows: Vec<AggregateRow>,
    pub ttfts: Vec<f64>,
}

pub async fn fetch_aggregates(state: &AppState, window: Window) -> AppResult<Aggregates> {
    let cutoff = cutoff_iso(window);
    let bucket_format = window.bucket_format();

    let rows = aggregate_rows(&state.db, &cutoff, bucket_format).await?;
    let ttfts = ttft_samples(&state.db, &cutoff).await?;

    Ok(Aggregates { rows, ttfts })
}

async fn aggregate_rows(
    db: &DatabaseConnection,
    cutoff: &str,
    bucket_format: &str,
) -> Result<Vec<AggregateRow>, sqlx::Error> {
    let rows: Vec<AggregateTuple> = sqlx::query_as(
        r#"
        SELECT
            strftime(?, created_at) AS bucket,
            COALESCE(routed_model, requested_model) AS model,
            COALESCE(routed_provider_name, 'unknown') AS provider,
            COUNT(*) AS requests,
            SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END) AS errors,
            SUM(CASE WHEN fallback_count > 0 THEN 1 ELSE 0 END) AS fallbacks,
            SUM(CASE WHEN cache_hit != 0 THEN 1 ELSE 0 END) AS cache_hits,
            COALESCE(SUM(input_tokens), 0) AS input_tokens,
            COALESCE(SUM(output_tokens), 0) AS output_tokens,
            COALESCE(SUM(cost_usd), 0.0) AS spend_usd
        FROM request_logs
        WHERE created_at >= ?
        GROUP BY bucket, model, provider
        ORDER BY bucket ASC
        "#,
    )
    .bind(bucket_format)
    .bind(cutoff)
    .fetch_all(db.get_sqlite_connection_pool())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                bucket,
                model,
                provider,
                requests,
                errors,
                fallbacks,
                cache_hits,
                input_tokens,
                output_tokens,
                spend_usd,
            )| AggregateRow {
                bucket,
                model,
                provider,
                requests,
                errors,
                fallbacks,
                cache_hits,
                input_tokens,
                output_tokens,
                spend_usd,
            },
        )
        .collect())
}

async fn ttft_samples(db: &DatabaseConnection, cutoff: &str) -> Result<Vec<f64>, sqlx::Error> {
    let rows: Vec<(f64,)> = sqlx::query_as(
        "SELECT ttft_ms FROM request_logs WHERE created_at >= ? AND ttft_ms IS NOT NULL",
    )
    .bind(cutoff)
    .fetch_all(db.get_sqlite_connection_pool())
    .await?;

    Ok(rows.into_iter().map(|(ttft,)| ttft).collect())
}

#[derive(Default)]
struct Totals {
    requests: i64,
    errors: i64,
    fallbacks: i64,
    cache_hits: i64,
    spend_usd: f64,
}

fn totals(rows: &[AggregateRow]) -> Totals {
    rows.iter().fold(Totals::default(), |mut acc, row| {
        acc.requests += row.requests;
        acc.errors += row.errors;
        acc.fallbacks += row.fallbacks;
        acc.cache_hits += row.cache_hits;
        acc.spend_usd += row.spend_usd;
        acc
    })
}

pub fn compute_kpis(aggregates: &Aggregates, window: Window) -> Value {
    let totals = totals(&aggregates.rows);
    let span_minutes = (window.span().num_seconds() as f64 / 60.0).max(1.0);

    json!({
        "total_requests": totals.requests,
        "rpm": (totals.requests as f64) / span_minutes,
        "total_spend_usd": (totals.spend_usd * 1_000_000.0).round() / 1_000_000.0,
        "ttft_p50_ms": percentile(&aggregates.ttfts, 50.0),
        "ttft_p95_ms": percentile(&aggregates.ttfts, 95.0),
        "error_rate_pct": pct(totals.errors as f64, totals.requests as f64),
        "fallback_rate_pct": pct(totals.fallbacks as f64, totals.requests as f64),
        "cache_hit_rate_pct": pct(totals.cache_hits as f64, totals.requests as f64),
    })
}

#[derive(Default)]
struct Bucket {
    requests: i64,
    input_tokens: i64,
    output_tokens: i64,
    spend_usd: f64,
    by_model: BTreeMap<(String, String), (i64, f64)>,
}

pub fn build_timeseries(aggregates: &Aggregates, window: Window) -> Value {
    let now = Utc::now();
    let width = window.bucket();
    let aligned_now = bucket_start(now, window);
    let mut buckets: BTreeMap<i64, Bucket> = BTreeMap::new();

    let first = aligned_now - width * (window.bucket_count() as i32 - 1);
    for index in 0..window.bucket_count() {
        buckets.insert(
            (first + width * index as i32).timestamp(),
            Bucket::default(),
        );
    }

    for row in &aggregates.rows {
        let Some(created) = parse_bucket(&row.bucket) else {
            continue;
        };
        let slot = bucket_start(created, window);
        let bucket = buckets.entry(slot.timestamp()).or_default();

        bucket.requests += row.requests;
        bucket.input_tokens += row.input_tokens;
        bucket.output_tokens += row.output_tokens;
        bucket.spend_usd += row.spend_usd;

        let entry = bucket
            .by_model
            .entry((row.model.clone(), row.provider.clone()))
            .or_insert((0, 0.0));
        entry.0 += row.input_tokens + row.output_tokens;
        entry.1 += row.spend_usd;
    }

    let series: Vec<Value> = buckets
        .into_iter()
        .map(|(timestamp, bucket)| {
            let start = Utc.timestamp_opt(timestamp, 0).single().unwrap_or(now);
            let by_model: Vec<Value> = bucket
                .by_model
                .into_iter()
                .map(|((model, provider), (tokens, spend))| {
                    json!({
                        "model": model,
                        "provider": provider,
                        "tokens": tokens,
                        "spend_usd": (spend * 1_000_000.0).round() / 1_000_000.0,
                    })
                })
                .collect();

            json!({
                "bucket": start.to_rfc3339_opts(SecondsFormat::Secs, true),
                "label": bucket_label(start, window),
                "requests": bucket.requests,
                "input_tokens": bucket.input_tokens,
                "output_tokens": bucket.output_tokens,
                "spend_usd": (bucket.spend_usd * 1_000_000.0).round() / 1_000_000.0,
                "by_model": by_model,
            })
        })
        .collect();

    Value::Array(series)
}

fn parse_bucket(raw: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|value| value.and_utc())
}

pub fn provider_health(state: &AppState, providers: &[provider::Model]) -> Value {
    let snapshots = state.routing.circuit().snapshots(providers);

    Value::Array(
        snapshots
            .into_iter()
            .map(|snap| {
                json!({
                    "provider_id": snap.provider_id,
                    "name": snap.name,
                    "provider_type": snap.provider_type,
                    "is_active": snap.is_active,
                    "circuit_state": snap.state,
                    "consecutive_failures": snap.consecutive_failures,
                    "total_requests": snap.total_requests,
                    "total_failures": snap.total_failures,
                    "last_error": snap.last_error,
                })
            })
            .collect(),
    )
}
