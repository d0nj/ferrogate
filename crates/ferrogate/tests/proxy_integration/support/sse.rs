//! SSE response parsing helpers.

use serde_json::Value;

/// Read an SSE body fully into `(event_names, data_payloads, content_type)`.
pub async fn collect_sse(response: reqwest::Response) -> (Vec<String>, Vec<String>, String) {
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "stream failed with {status}: {text}");

    let mut names = Vec::new();
    let mut data = Vec::new();
    let mut current_event: Option<String> = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            current_event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.trim().to_string());
            names.push(current_event.take().unwrap_or_default());
        }
    }

    (names, data, content_type)
}

/// Concatenate `choices[0].delta.content` from OpenAI-shaped chunks.
pub fn concat_deltas(frames: &[String]) -> String {
    let mut out = String::new();
    for frame in frames {
        let Ok(value) = serde_json::from_str::<Value>(frame) else {
            continue;
        };
        if let Some(text) = value["choices"][0]["delta"]["content"].as_str() {
            out.push_str(text);
        }
    }
    out
}
