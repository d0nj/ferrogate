use serde_json::{json, Map, Value};

use super::DecisionUsage;

static INPUT_KEYS: &[&str] = &["input_tokens", "prompt_tokens"];
static OUTPUT_KEYS: &[&str] = &["output_tokens", "completion_tokens"];

pub(crate) fn label(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub(crate) fn usage_from(value: &Value) -> Option<DecisionUsage> {
    DecisionUsage::from_value(value.get("usage"), INPUT_KEYS, OUTPUT_KEYS)
}

pub(crate) fn media_type_of(source: &str) -> Option<String> {
    super::super::support::parse_data_url(source).map(|parsed| parsed.mime.to_string())
}

pub(crate) fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn take_error(value: &Value) -> Result<(), String> {
    match value.get("error") {
        Some(error) => Err(error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string())),
        None => Ok(()),
    }
}

pub(crate) fn boxes_of(raw: Option<&Value>) -> Vec<Value> {
    match raw {
        Some(Value::Array(items)) if items.iter().all(Value::is_number) => {
            vec![Value::Array(items.clone())]
        }
        Some(Value::Array(items)) => items.clone(),
        Some(other) => vec![other.clone()],
        None => Vec::new(),
    }
}

pub(crate) fn is_refusal(entry: &Value) -> bool {
    match entry.get("refusal") {
        Some(flag) => flag == true || flag == "refusal",
        None => entry.get("type").and_then(Value::as_str) == Some("refusal"),
    }
}

pub(crate) fn probability_of(raw: Option<&Value>) -> f64 {
    match raw {
        Some(Value::Bool(flag)) => {
            if *flag {
                1.0
            } else {
                0.0
            }
        }
        Some(other) => other.as_f64().unwrap_or(0.0),
        None => 0.0,
    }
}

pub(crate) fn probability_map(raw: Option<&Value>) -> Vec<(Value, f64)> {
    raw.and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| Some((Value::String(key.clone()), value.as_f64()?)))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn score_probabilities(raw: Option<&Value>) -> Vec<(String, Option<String>, f64)> {
    raw.and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| match value {
                    Value::Object(inner) => Some((
                        key.clone(),
                        string_field(&Value::Object(inner.clone()), "description"),
                        inner.get("probability")?.as_f64()?,
                    )),
                    other => Some((key.clone(), None, other.as_f64()?)),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn probability_object(probabilities: &[(Value, f64)]) -> Value {
    Value::Object(
        probabilities
            .iter()
            .map(|(value, probability)| (label(value), json!(probability)))
            .collect(),
    )
}

pub(crate) fn score_probability_object(probabilities: &[(String, Option<String>, f64)]) -> Value {
    let map: Map<String, Value> = probabilities
        .iter()
        .map(|(label, description, probability)| {
            let value = match description {
                Some(description) => {
                    json!({ "probability": probability, "description": description })
                }
                None => json!(probability),
            };
            (label.clone(), value)
        })
        .collect();
    Value::Object(map)
}
