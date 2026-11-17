use std::time::Duration;

use hmac::{Hmac, Mac};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde::Serialize;
use sha2::Sha256;
use tokio::sync::mpsc;

use crate::entities::webhook;

pub const EVENT_REQUEST_BLOCKED: &str = "request.blocked";
pub const EVENT_BUDGET_EXCEEDED: &str = "budget.exceeded";
pub const EVENT_GUARDRAIL_TRIGGERED: &str = "guardrail.triggered";
pub const EVENT_PING: &str = "ping";

pub const KNOWN_EVENTS: [&str; 4] = [
    EVENT_REQUEST_BLOCKED,
    EVENT_BUDGET_EXCEEDED,
    EVENT_GUARDRAIL_TRIGGERED,
    EVENT_PING,
];

const CHANNEL_CAPACITY: usize = 256;

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
pub struct WebhookEvent {
    pub event: String,
    pub timestamp: String,
    #[serde(flatten)]
    pub data: serde_json::Value,
}

impl WebhookEvent {
    pub fn new(event: &str, data: serde_json::Value) -> Self {
        Self {
            event: event.to_owned(),
            timestamp: timestamp(),
            data,
        }
    }
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[derive(Clone)]
pub struct WebhookSender {
    tx: mpsc::Sender<WebhookEvent>,
}

impl WebhookSender {
    pub fn create_channel() -> (Self, mpsc::Receiver<WebhookEvent>) {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        (Self { tx }, rx)
    }

    pub fn disabled() -> Self {
        let (tx, _rx) = mpsc::channel(1);
        Self { tx }
    }

    pub fn emit(&self, event: WebhookEvent) {
        if let Err(err) = self.tx.try_send(event) {
            match err {
                mpsc::error::TrySendError::Full(event) => {
                    tracing::warn!(event = %event.event, "webhook queue full; dropping event");
                }
                mpsc::error::TrySendError::Closed(event) => {
                    tracing::debug!(event = %event.event, "webhook worker gone; dropping event");
                }
            }
        }
    }
}

pub async fn run_worker(
    db: DatabaseConnection,
    http: reqwest::Client,
    secrets: std::sync::Arc<crate::crypto::SecretBox>,
    mut rx: mpsc::Receiver<WebhookEvent>,
) {
    while let Some(event) = rx.recv().await {
        match active_subscribers(&db, &event.event).await {
            Ok(subscribers) => {
                for subscriber in subscribers {
                    deliver(&db, &http, &secrets, &subscriber, &event).await;
                }
            }
            Err(err) => {
                tracing::warn!(event = %event.event, error = %err, "failed to load webhook subscribers");
            }
        }
    }
}

pub async fn deliver(
    db: &DatabaseConnection,
    http: &reqwest::Client,
    secrets: &crate::crypto::SecretBox,
    subscriber: &webhook::Model,
    event: &WebhookEvent,
) -> Outcome {
    let body = match serde_json::to_vec(event) {
        Ok(body) => body,
        Err(err) => {
            return Outcome {
                status: None,
                error: Some(err.to_string()),
            }
        }
    };

    let mut request = http
        .post(&subscriber.url)
        .timeout(DELIVERY_TIMEOUT)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("x-ferrogate-event", &event.event)
        .body(body.clone());

    if let Some(secret) = subscriber
        .secret
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        match secrets.open(secret) {
            Ok(opened) if !opened.is_empty() => {
                request = request.header("x-ferrogate-signature", &signature(&opened, &body));
            }
            Ok(_) => {}
            Err(err) => {
                let outcome = Outcome {
                    status: None,
                    error: Some(format!("webhook secret unavailable: {err}")),
                };
                record_outcome(db, subscriber, &outcome).await;
                return outcome;
            }
        }
    }

    let outcome = match request.send().await {
        Ok(response) => {
            let status = response.status();
            let error = if status.is_success() {
                None
            } else {
                Some(format!("upstream returned {status}"))
            };
            Outcome {
                status: Some(i64::from(status.as_u16())),
                error,
            }
        }
        Err(err) => Outcome {
            status: None,
            error: Some(err.to_string()),
        },
    };

    record_outcome(db, subscriber, &outcome).await;
    outcome
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub status: Option<i64>,
    pub error: Option<String>,
}

impl Outcome {
    pub fn delivered(&self) -> bool {
        self.error.is_none()
    }
}

pub fn signature(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac accepts any key");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

pub fn parse_events(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

pub fn subscribes(raw: &str, event: &str) -> bool {
    parse_events(raw).iter().any(|name| name == event)
}

async fn active_subscribers(
    db: &DatabaseConnection,
    event: &str,
) -> Result<Vec<webhook::Model>, sea_orm::DbErr> {
    let rows = webhook::Entity::find()
        .filter(webhook::Column::IsActive.eq(1))
        .all(db)
        .await?;

    Ok(rows
        .into_iter()
        .filter(|row| subscribes(&row.events_json, event))
        .collect())
}

async fn record_outcome(db: &DatabaseConnection, subscriber: &webhook::Model, outcome: &Outcome) {
    let update = webhook::ActiveModel {
        id: Set(subscriber.id.clone()),
        last_status: Set(outcome.status),
        last_error: Set(outcome.error.clone()),
        updated_at: Set(timestamp()),
        ..Default::default()
    };

    if let Err(err) = update.update(db).await {
        tracing::warn!(webhook = %subscriber.id, error = %err, "failed to persist webhook outcome");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_stable_and_prefixed() {
        let one = signature("s3cret", b"{\"a\":1}");
        assert_eq!(one, signature("s3cret", b"{\"a\":1}"));
        assert!(one.starts_with("sha256="));
        assert_ne!(one, signature("other", b"{\"a\":1}"));
    }

    #[test]
    fn subscription_matching_reads_event_array() {
        let raw = r#"["request.blocked","ping"]"#;
        assert!(subscribes(raw, "ping"));
        assert!(!subscribes(raw, "budget.exceeded"));
        assert!(!subscribes("not json", "ping"));
    }
}
