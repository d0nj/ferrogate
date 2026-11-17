use serde_json::{json, Map, Value};

use super::common::{string_field, take_error};
use super::keyed::{answer_value, keyed_questions, parse_answers, parse_questions, Criteria};
use super::{DecisionAsset, DecisionUsage, UnifiedDecisionRequest, UnifiedDecisionResponse};

pub fn to_gemini_body(req: &UnifiedDecisionRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert("input".into(), input_value(req));
    body.insert(
        "questions".into(),
        Value::Object(keyed_questions(req, Criteria::Gemini)),
    );
    if let Some(identifier) = &req.safety_identifier {
        body.insert("safety_identifier".into(), json!(identifier));
    }
    Value::Object(body)
}

fn input_value(req: &UnifiedDecisionRequest) -> Value {
    match (&req.raw_input, req.input.is_empty()) {
        (Some(raw), true) => raw.clone(),
        _ => Value::Array(req.input.iter().map(gemini_part).collect()),
    }
}

fn gemini_part(asset: &DecisionAsset) -> Value {
    let mut part = match asset.kind.as_str() {
        "text" => json!({ "type": "text", "text": asset.source }),
        "json" => json!({
            "type": "text",
            "text": match asset.json_data.clone().unwrap_or(Value::Null) {
                Value::String(text) => text,
                other => other.to_string(),
            },
        }),
        kind => match asset.data_url_payload() {
            Some((mime, data)) => json!({ "type": kind, "data": data, "mime_type": mime }),
            None => json!({
                "type": kind,
                "uri": asset.source,
                "mime_type": asset.resolved_media_type(),
            }),
        },
    };
    if let Some(resolution) = &asset.resolution {
        part["resolution"] = json!(resolution);
    }
    part
}

pub fn from_gemini_request(value: &Value) -> Result<UnifiedDecisionRequest, String> {
    let model = string_field(value, "model").ok_or("missing 'model'")?;
    let input = match value.get("input") {
        Some(input) => gemini_assets(input)?,
        None => Vec::new(),
    };
    Ok(UnifiedDecisionRequest {
        model,
        input,
        raw_input: None,
        questions: parse_questions(value.get("questions"), Criteria::Gemini)?,
        safety_identifier: string_field(value, "safety_identifier"),
        prompt_cache_key: string_field(value, "prompt_cache_key"),
    })
}

fn gemini_assets(input: &Value) -> Result<Vec<DecisionAsset>, String> {
    let mut assets = Vec::new();
    match input {
        Value::String(text) => assets.push(DecisionAsset::new("text", text.clone())),
        Value::Object(_) => match input.get("parts").and_then(Value::as_array) {
            Some(parts) => assets.extend(parts.iter().map(gemini_asset)),
            None => assets.push(gemini_asset(input)),
        },
        Value::Array(items) => {
            for item in items {
                match item.get("parts").and_then(Value::as_array) {
                    Some(parts) => assets.extend(parts.iter().map(gemini_asset)),
                    None => assets.push(gemini_asset(item)),
                }
            }
        }
        other => return Err(format!("invalid 'input': {other}")),
    }
    Ok(assets)
}

fn gemini_asset(part: &Value) -> DecisionAsset {
    if let Some(text) = part.get("text").and_then(Value::as_str) {
        return DecisionAsset::new("text", text);
    }
    let kind = part.get("type").and_then(Value::as_str).unwrap_or("image");
    let media_type = string_field(part, "mime_type");
    let source = match string_field(part, "data") {
        Some(data) => {
            let mime = media_type.as_deref().unwrap_or("application/octet-stream");
            format!("data:{mime};base64,{data}")
        }
        None => string_field(part, "uri").unwrap_or_default(),
    };
    DecisionAsset {
        media_type,
        resolution: string_field(part, "resolution"),
        ..DecisionAsset::new(kind, source)
    }
}

pub fn from_gemini_response(
    value: &Value,
    requested_model: &str,
) -> Result<UnifiedDecisionResponse, String> {
    take_error(value)?;
    Ok(UnifiedDecisionResponse {
        model: string_field(value, "model").unwrap_or_else(|| requested_model.to_string()),
        answers: parse_answers(value)?,
        usage: DecisionUsage::from_value(
            value.get("usage"),
            &["total_input_tokens", "input_tokens", "prompt_tokens"],
            &["total_output_tokens", "output_tokens", "completion_tokens"],
        ),
    })
}

pub fn to_gemini_response(resp: &UnifiedDecisionResponse) -> Value {
    let answers: Map<String, Value> = resp
        .answers
        .iter()
        .map(|result| {
            let mut entry = answer_value(result);
            if let Some(map) = entry.as_object_mut() {
                if let Some(boxes) = map.remove("box") {
                    map.insert("boxes".into(), boxes);
                }
            }
            (result.name.clone(), entry)
        })
        .collect();
    let usage = resp.usage.clone().unwrap_or_default();
    json!({
        "answers": answers,
        "usage": {
            "total_input_tokens": usage.input_tokens,
            "total_output_tokens": usage.output_tokens,
            "total_tokens": usage.total_tokens,
        },
    })
}
