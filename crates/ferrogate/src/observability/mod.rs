mod otlp;

use serde_json::json;

use crate::{config::AppConfig, telemetry::TelemetryEvent};

#[derive(Debug, Clone)]
pub struct Observability {
    endpoint: Option<otlp::TracesEndpoint>,
}

impl Observability {
    pub fn from_config(config: &AppConfig) -> Self {
        let endpoint = config
            .otlp_endpoint
            .as_deref()
            .filter(|raw| {
                crate::egress::validate_strict_no_dns(raw, config.allow_private_upstreams).is_ok()
            })
            .and_then(otlp::TracesEndpoint::parse);

        match endpoint {
            Some(endpoint) => {
                tracing::info!(endpoint = %endpoint.url, "otlp trace export enabled");
                Self {
                    endpoint: Some(endpoint),
                }
            }
            None => Self { endpoint: None },
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.endpoint.is_some()
    }

    pub fn export(&self, client: &reqwest::Client, event: &TelemetryEvent) {
        let Some(endpoint) = self.endpoint.as_ref() else {
            return;
        };

        let payload = otlp::encode(&span_json(event));
        otlp::send(client, endpoint, payload);
    }
}

fn span_json(event: &TelemetryEvent) -> serde_json::Value {
    let mut attributes = Vec::with_capacity(7);
    attributes.push(otlp::string_attr(
        "gen_ai.request.model",
        &event.requested_model,
    ));
    if let Some(model) = event.routed_model.as_deref() {
        attributes.push(otlp::string_attr("gen_ai.response.model", model));
    }
    attributes.push(otlp::int_attr(
        "gen_ai.usage.input_tokens",
        i64::from(event.input_tokens),
    ));
    attributes.push(otlp::int_attr(
        "gen_ai.usage.output_tokens",
        i64::from(event.output_tokens),
    ));
    if let Some(key_id) = event.virtual_key_id.as_deref() {
        attributes.push(otlp::string_attr("ferrogate.virtual_key_id", key_id));
    }
    if let Some(provider) = event.routed_provider_name.as_deref() {
        attributes.push(otlp::string_attr("ferrogate.provider", provider));
    }
    attributes.push(otlp::int_attr(
        "ferrogate.fallback_count",
        i64::from(event.fallback_count),
    ));

    if event.status_code >= 400 || event.error_message.is_some() {
        attributes.push(otlp::string_attr(
            "otel.status_code",
            if event.status_code >= 500 {
                "ERROR"
            } else {
                "OK"
            },
        ));
    }

    json!({
        "name": event.endpoint,
        "attributes": attributes,
        "error": event.error_message,
        "status_code": event.status_code,
        "started_at": event.created_at,
        "duration_ms": event.latency_ms,
    })
}
