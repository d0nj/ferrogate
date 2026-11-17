use super::*;

fn event(provider: &str, model: &str) -> TelemetryEvent {
    let mut event = TelemetryEvent::new(model, false);
    event.routed_provider_name = Some(provider.to_string());
    event.routed_model = Some(model.to_string());
    event
}

#[test]
fn renders_type_line_per_family() {
    let metrics = Metrics::new();
    metrics.record(&event("openai", "gpt-4o"));
    let rendered = metrics.render();

    for family in [
        "ferrogate_requests_total",
        "ferrogate_request_duration_seconds",
        "ferrogate_ttft_seconds",
        "ferrogate_tokens_total",
        "ferrogate_spend_usd_total",
        "ferrogate_telemetry_drops_total",
        "ferrogate_audit_failures_total",
        "ferrogate_cache_hits_total",
        "ferrogate_cache_misses_total",
        "ferrogate_guardrail_blocks_total",
    ] {
        assert!(
            rendered.contains(&format!("# TYPE {family} ")),
            "missing TYPE line for {family}"
        );
    }
}

#[test]
fn counts_requests_and_histogram_buckets() {
    let metrics = Metrics::new();
    let mut first = event("openai", "gpt-4o");
    first.latency_ms = 120.0;
    metrics.record(&first);

    let mut second = event("openai", "gpt-4o");
    second.latency_ms = 900.0;
    metrics.record(&second);

    let rendered = metrics.render();
    assert!(rendered.contains(
        "ferrogate_requests_total{status=\"200\",provider=\"openai\",model=\"gpt-4o\"} 2"
    ));
    assert!(rendered
        .contains("ferrogate_request_duration_seconds_bucket{provider=\"openai\",le=\"0.25\"} 1"));
    assert!(rendered
        .contains("ferrogate_request_duration_seconds_bucket{provider=\"openai\",le=\"1\"} 2"));
    assert!(rendered.contains("ferrogate_request_duration_seconds_count{provider=\"openai\"} 2"));
}

#[test]
fn records_tokens_spend_and_cache() {
    let metrics = Metrics::new();
    let mut hit = event("openai", "gpt-4o");
    hit.input_tokens = 10;
    hit.output_tokens = 5;
    hit.cost_usd = 0.000_25;
    hit.virtual_key_id = Some("vk-1".to_string());
    hit.cache_hit = true;
    metrics.record(&hit);

    let mut miss = event("openai", "gpt-4o");
    miss.cache_type = Some("exact".to_string());
    metrics.record(&miss);

    let rendered = metrics.render();
    assert!(rendered.contains("ferrogate_tokens_total{direction=\"input\",provider=\"openai\"} 10"));
    assert!(rendered.contains("ferrogate_spend_usd_total{virtual_key=\"vk-1\"} 0.00025"));
    assert!(rendered.contains("ferrogate_cache_hits_total 1"));
    assert!(rendered.contains("ferrogate_cache_misses_total 1"));
}

#[test]
fn escapes_quotes_in_label_values() {
    let metrics = Metrics::new();
    metrics.record(&event("we\"ird", "m"));
    assert!(metrics.render().contains("provider=\"we\\\"ird\""));
}

#[test]
fn splits_guardrail_action_into_rule_series() {
    let mut block = event("openai", "gpt-4o");
    block.guardrail_action = Some("flag:pii,secrets;redact:pii".to_string());
    let metrics = Metrics::new();
    metrics.record(&block);

    let rendered = metrics.render();
    assert!(rendered.contains("ferrogate_guardrail_blocks_total{rule=\"pii\"} 2"));
    assert!(rendered.contains("ferrogate_guardrail_blocks_total{rule=\"secrets\"} 1"));
}
