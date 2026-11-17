use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const CONTENT_TYPE: &str = "application/json";

#[derive(Debug, Clone)]
pub struct TracesEndpoint {
    pub url: String,
}

impl TracesEndpoint {
    pub fn parse(raw: &str) -> Option<Self> {
        let base = raw.trim().trim_end_matches('/');
        if base.is_empty() {
            return None;
        }

        let url = match base.ends_with("/v1/traces") {
            true => base.to_string(),
            false => format!("{base}/v1/traces"),
        };

        match url.starts_with("http://") || url.starts_with("https://") {
            true => Some(Self { url }),
            false => None,
        }
    }
}

pub fn encode(span: &Value) -> Value {
    json!({
        "resourceSpans": [{
            "resource": {
                "attributes": [string_attr("service.name", "ferrogate")]
            },
            "scopeSpans": [{
                "scope": { "name": "ferrogate.request" },
                "spans": [span_record(span)]
            }]
        }]
    })
}

fn span_record(span: &Value) -> Value {
    let (start_ns, end_ns) = span_times(span);
    let status = match span["error"].as_str() {
        Some(message) => json!({ "code": 2, "message": message }),
        None => {
            json!({ "code": if span["status_code"].as_i64().unwrap_or(0) >= 500 { 2 } else { 0 } })
        }
    };

    json!({
        "traceId": hex_id(16),
        "spanId": hex_id(8),
        "name": span["name"].as_str().unwrap_or("request"),
        "kind": 3,
        "startTimeUnixNano": start_ns.to_string(),
        "endTimeUnixNano": end_ns.to_string(),
        "attributes": span["attributes"],
        "status": status,
    })
}

fn span_times(span: &Value) -> (i64, i64) {
    let started = span["started_at"]
        .as_str()
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|ts| ts.timestamp_nanos_opt().unwrap_or(0))
        .unwrap_or_else(now_nanos);

    let duration_ns = (span["duration_ms"].as_f64().unwrap_or(0.0) * 1_000_000.0).max(0.0) as i64;
    (started, started.saturating_add(duration_ns))
}

fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as i64)
        .unwrap_or(0)
}

fn hex_id(bytes: usize) -> String {
    let random: [u8; 16] = rand::random();
    hex::encode(&random[..bytes])
}

pub fn string_attr(key: &str, value: &str) -> Value {
    json!({ "key": key, "value": { "stringValue": value } })
}

pub fn int_attr(key: &str, value: i64) -> Value {
    json!({ "key": key, "value": { "intValue": value.to_string() } })
}

pub fn send(client: &reqwest::Client, endpoint: &TracesEndpoint, payload: Value) {
    let client = client.clone();
    let url = endpoint.url.clone();
    tokio::spawn(async move {
        let result = client
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, CONTENT_TYPE)
            .json(&payload)
            .send()
            .await;

        match result {
            Ok(response) if !response.status().is_success() => {
                tracing::debug!(status = %response.status(), "otlp trace export rejected");
            }
            Ok(_) => {}
            Err(err) => tracing::debug!(error = %err, "otlp trace export failed"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_signal_path_once() {
        let base = TracesEndpoint::parse("http://collector:4318").expect("valid");
        assert_eq!(base.url, "http://collector:4318/v1/traces");

        let full = TracesEndpoint::parse("http://collector:4318/v1/traces/").expect("valid");
        assert_eq!(full.url, "http://collector:4318/v1/traces");
    }

    #[test]
    fn rejects_non_http_endpoints() {
        assert!(TracesEndpoint::parse("").is_none());
        assert!(TracesEndpoint::parse("collector:4318").is_none());
    }

    #[test]
    fn encodes_resource_span_shape() {
        let span = json!({
            "name": "/v1/chat/completions",
            "attributes": [string_attr("gen_ai.request.model", "gpt-4o")],
            "error": null,
            "status_code": 200,
            "started_at": "2026-01-01T00:00:00.000Z",
            "duration_ms": 12.0,
        });
        let payload = encode(&span);
        let record = &payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
        assert_eq!(record["traceId"].as_str().map(str::len), Some(32));
        assert_eq!(record["spanId"].as_str().map(str::len), Some(16));
        assert_eq!(record["startTimeUnixNano"], json!("1767225600000000000"));
    }
}
