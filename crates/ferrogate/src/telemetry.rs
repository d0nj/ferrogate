use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use chrono::Utc;
use dashmap::DashMap;
use sea_orm::{
    sea_query::{Expr, ExprTrait, OnConflict},
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::entities::{request_log, virtual_key};

pub const TELEMETRY_CHANNEL_CAPACITY: usize = 10_000;

const MAX_TELEMETRY_BATCH_SIZE: usize = 100;

const FLUSH_INTERVAL: Duration = Duration::from_millis(500);

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn tags_with_guardrails(event: &TelemetryEvent) -> Option<String> {
    if event.guardrail_flags.is_empty() {
        return event.tags_json.clone();
    }

    let mut value = event
        .tags_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    if !value.is_object() {
        value = Value::Object(serde_json::Map::new());
    }
    if let Value::Object(map) = &mut value {
        map.insert(
            "guardrails".to_string(),
            serde_json::to_value(&event.guardrail_flags).unwrap_or(Value::Null),
        );
    }
    Some(value.to_string())
}

pub const DEFAULT_ENDPOINT: &str = "/v1/chat/completions";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingAttempt {
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub status_code: u16,
    pub latency_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TelemetryEvent {
    pub id: String,
    pub created_at: String,
    pub virtual_key_id: Option<String>,
    pub virtual_key_name: Option<String>,
    pub requested_model: String,
    pub routed_provider_id: Option<String>,
    pub routed_provider_name: Option<String>,
    pub routed_model: Option<String>,
    pub status_code: i64,
    pub is_streaming: bool,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
    pub ttft_ms: Option<f64>,
    pub latency_ms: f64,
    pub fallback_count: u32,
    pub cache_hit: bool,
    pub routing_trace: Vec<RoutingAttempt>,
    pub request_messages_json: Option<String>,
    pub response_text: Option<String>,
    pub error_message: Option<String>,
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub endpoint: String,
    pub upstream_ttfb_ms: Option<f64>,
    pub guardrail_action: Option<String>,
    pub guardrail_flags: Vec<String>,
    pub cache_type: Option<String>,
    pub tags_json: Option<String>,
}

impl TelemetryEvent {
    pub fn new(requested_model: impl Into<String>, is_streaming: bool) -> Self {
        Self {
            id: format!("req-{}", Uuid::new_v4()),
            created_at: now_rfc3339(),
            virtual_key_id: None,
            virtual_key_name: None,
            requested_model: requested_model.into(),
            routed_provider_id: None,
            routed_provider_name: None,
            routed_model: None,
            status_code: 200,
            is_streaming,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            ttft_ms: None,
            latency_ms: 0.0,
            fallback_count: 0,
            cache_hit: false,
            routing_trace: Vec::new(),
            request_messages_json: None,
            response_text: None,
            error_message: None,
            request_id: None,
            session_id: None,
            endpoint: DEFAULT_ENDPOINT.to_owned(),
            upstream_ttfb_ms: None,
            guardrail_action: None,
            guardrail_flags: Vec::new(),
            cache_type: None,
            tags_json: None,
        }
    }
}

static DROPPED_EVENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn dropped_events() -> u64 {
    DROPPED_EVENTS.load(Ordering::Relaxed)
}

#[derive(Clone)]
pub struct TelemetrySender {
    tx: mpsc::Sender<TelemetryEvent>,
}

impl TelemetrySender {
    pub fn new(tx: mpsc::Sender<TelemetryEvent>) -> Self {
        Self { tx }
    }

    pub fn create_channel() -> (Self, mpsc::Receiver<TelemetryEvent>) {
        let (tx, rx) = mpsc::channel(TELEMETRY_CHANNEL_CAPACITY);
        (Self::new(tx), rx)
    }

    pub fn record(&self, event: TelemetryEvent) {
        if let Err(err) = self.tx.try_send(event) {
            match err {
                mpsc::error::TrySendError::Full(event) => {
                    DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(request_id = %event.id, "telemetry channel full; dropping event");
                }
                mpsc::error::TrySendError::Closed(event) => {
                    tracing::debug!(request_id = %event.id, "telemetry worker gone; dropping event");
                }
            }
        }
    }
}

pub fn add_live_spend(live_spend: &DashMap<String, Arc<AtomicI64>>, key_id: &str, cost_usd: f64) {
    let micros = (cost_usd * 1_000_000.0).round() as i64;
    let entry = live_spend
        .entry(key_id.to_string())
        .or_insert_with(|| Arc::new(AtomicI64::new(0)));
    entry.fetch_add(micros, Ordering::Relaxed);
}

pub fn set_or_reset_live_spend(
    live_spend: &DashMap<String, Arc<AtomicI64>>,
    key_id: &str,
    db_spent_usd: f64,
) {
    let persisted = (db_spent_usd * 1_000_000.0).round() as i64;
    let entry = live_spend
        .entry(key_id.to_string())
        .or_insert_with(|| Arc::new(AtomicI64::new(0)));

    let current = entry.load(Ordering::Relaxed);
    if persisted > current {
        entry.store(persisted, Ordering::Relaxed);
    }
}

pub fn live_spend_micros(live_spend: &DashMap<String, Arc<AtomicI64>>, key_id: &str) -> i64 {
    live_spend
        .get(key_id)
        .map(|entry| entry.load(Ordering::Relaxed))
        .unwrap_or(0)
}

pub fn spawn_telemetry_worker(
    db: DatabaseConnection,
    mut rx: mpsc::Receiver<TelemetryEvent>,
    live_spend: Arc<DashMap<String, Arc<AtomicI64>>>,
    metrics: Arc<crate::metrics::Metrics>,
    observability: Arc<crate::observability::Observability>,
    http_client: reqwest::Client,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer: Vec<TelemetryEvent> = Vec::with_capacity(MAX_TELEMETRY_BATCH_SIZE);
        let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;

        loop {
            tokio::select! {
                biased;
                maybe_event = rx.recv() => {
                    match maybe_event {
                        Some(event) => {
                            observe(&metrics, &observability, &http_client, &event);
                            buffer.push(event);
                            while buffer.len() < MAX_TELEMETRY_BATCH_SIZE {
                                match rx.try_recv() {
                                    Ok(event) => {
                                        observe(&metrics, &observability, &http_client, &event);
                                        buffer.push(event);
                                    }
                                    Err(_) => break,
                                }
                            }
                            if buffer.len() >= MAX_TELEMETRY_BATCH_SIZE {
                                mirror_flushed_spend_with_live_repair(&db, &live_spend, &mut buffer).await;
                            }
                        }
                        None => break,
                    }
                }
                _ = ticker.tick() => {
                    if !buffer.is_empty() {
                        mirror_flushed_spend_with_live_repair(&db, &live_spend, &mut buffer).await;
                    }
                }
            }
        }

        if !buffer.is_empty() {
            mirror_flushed_spend_with_live_repair(&db, &live_spend, &mut buffer).await;
        }

        tracing::debug!("telemetry worker drained and stopped");
    })
}

fn observe(
    metrics: &crate::metrics::Metrics,
    observability: &crate::observability::Observability,
    http_client: &reqwest::Client,
    event: &TelemetryEvent,
) {
    metrics.record(event);
    observability.export(http_client, event);
}

async fn mirror_flushed_spend_with_live_repair(
    db: &DatabaseConnection,
    live_spend: &DashMap<String, Arc<AtomicI64>>,
    buffer: &mut Vec<TelemetryEvent>,
) {
    let ids: Vec<String> = buffer
        .iter()
        .filter_map(|event| event.virtual_key_id.clone())
        .collect();

    let spent = flush(db, buffer).await;
    if spent.is_empty() {
        return;
    }

    match virtual_key::Entity::find()
        .filter(virtual_key::Column::Id.is_in(ids))
        .all(db)
        .await
    {
        Ok(rows) => {
            for row in rows {
                set_or_reset_live_spend(live_spend, &row.id, row.spent_usd);
            }
        }
        Err(err) => tracing::debug!(error = %err, "live spend mirror skipped"),
    }
}

async fn flush(db: &DatabaseConnection, buffer: &mut Vec<TelemetryEvent>) -> HashMap<String, f64> {
    if buffer.is_empty() {
        return HashMap::new();
    }

    let events = std::mem::take(buffer);

    let mut spend_by_key: HashMap<String, f64> = HashMap::new();
    for event in &events {
        if let Some(key_id) = &event.virtual_key_id {
            *spend_by_key.entry(key_id.clone()).or_insert(0.0) += event.cost_usd;
        }
    }

    let result = async {
        let txn = db.begin().await?;

        let rows: Vec<request_log::ActiveModel> = events
            .iter()
            .map(|event| request_log::ActiveModel {
                id: Set(event.id.clone()),
                created_at: Set(event.created_at.clone()),
                virtual_key_id: Set(event.virtual_key_id.clone()),
                virtual_key_name: Set(event.virtual_key_name.clone()),
                requested_model: Set(event.requested_model.clone()),
                routed_provider_id: Set(event.routed_provider_id.clone()),
                routed_provider_name: Set(event.routed_provider_name.clone()),
                routed_model: Set(event.routed_model.clone()),
                status_code: Set(event.status_code),
                is_streaming: Set(i64::from(event.is_streaming)),
                input_tokens: Set(i64::from(event.input_tokens)),
                output_tokens: Set(i64::from(event.output_tokens)),
                cost_usd: Set(event.cost_usd),
                ttft_ms: Set(event.ttft_ms),
                latency_ms: Set(event.latency_ms),
                fallback_count: Set(i64::from(event.fallback_count)),
                cache_hit: Set(i64::from(event.cache_hit)),
                routing_trace_json: Set(serde_json::to_string(&event.routing_trace)
                    .unwrap_or_else(|_| "[]".to_string())),
                request_messages_json: Set(event.request_messages_json.clone()),
                response_text: Set(event.response_text.clone()),
                error_message: Set(event.error_message.clone()),
                request_id: Set(event.request_id.clone()),
                session_id: Set(event.session_id.clone()),
                endpoint: Set(event.endpoint.clone()),
                upstream_ttfb_ms: Set(event.upstream_ttfb_ms),
                guardrail_action: Set(event.guardrail_action.clone()),
                cache_type: Set(event.cache_type.clone()),
                tags_json: Set(tags_with_guardrails(event)),
            })
            .collect();

        if !rows.is_empty() {
            request_log::Entity::insert_many(rows)
                .on_conflict(
                    OnConflict::column(request_log::Column::Id)
                        .do_nothing()
                        .to_owned(),
                )
                .exec(&txn)
                .await?;
        }

        let used_at = now_rfc3339();
        for (key_id, spend) in &spend_by_key {
            virtual_key::Entity::update_many()
                .col_expr(
                    virtual_key::Column::SpentUsd,
                    Expr::col(virtual_key::Column::SpentUsd).add(*spend),
                )
                .col_expr(
                    virtual_key::Column::LastUsedAt,
                    Expr::value(used_at.clone()),
                )
                .filter(virtual_key::Column::Id.eq(key_id.as_str()))
                .exec(&txn)
                .await?;
        }

        txn.commit().await?;
        Ok::<(), sea_orm::DbErr>(())
    }
    .await;

    match result {
        Ok(()) => {
            tracing::trace!(count = events.len(), "telemetry batch flushed");
            spend_by_key
        }
        Err(err) => {
            tracing::error!(error = %err, count = events.len(), "telemetry batch write failed");
            HashMap::new()
        }
    }
}

pub fn messages_json(messages: &[crate::providers::ChatMessage]) -> Option<String> {
    serde_json::to_string(messages).ok()
}

pub fn content_preview(content: &Value) -> Option<String> {
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => serde_json::to_string(other).ok(),
    }
}
