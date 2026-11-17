use serde_json::{json, Map, Value};

use super::common::{string_field, take_error, usage_from};
use super::keyed::{answer_value, keyed_questions, parse_answers, parse_questions, Criteria};
use super::{DecisionAsset, UnifiedDecisionRequest, UnifiedDecisionResponse};

pub fn to_typesafe_body(req: &UnifiedDecisionRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert(
        "state".into(),
        req.raw_input
            .clone()
            .unwrap_or_else(|| json!(req.text_input())),
    );
    let assets = keyed_assets(req);
    if !assets.is_empty() {
        body.insert("assets".into(), Value::Object(assets));
    }
    body.insert(
        "questions".into(),
        Value::Object(keyed_questions(req, Criteria::TypeSafe)),
    );
    if let Some(identifier) = &req.safety_identifier {
        body.insert("safety_identifier".into(), json!(identifier));
    }
    Value::Object(body)
}

fn keyed_assets(req: &UnifiedDecisionRequest) -> Map<String, Value> {
    let mut assets = Map::new();
    for (index, asset) in req.binary_assets().enumerate() {
        if asset.kind == "json" {
            continue;
        }
        let entry = match asset.data_url_payload() {
            Some((mime, data)) => {
                json!({ "type": asset.kind, "data_base64": data, "format": mime })
            }
            None => json!({ "type": asset.kind, "url": asset.source }),
        };
        assets.insert(asset.key(index), entry);
    }
    assets
}

pub fn from_typesafe_request(value: &Value) -> Result<UnifiedDecisionRequest, String> {
    let model = string_field(value, "model").ok_or("missing 'model'")?;
    let (mut input, raw_input) = match value.get("state") {
        Some(Value::String(text)) => (vec![DecisionAsset::new("text", text.clone())], None),
        Some(state @ Value::Object(_)) => (Vec::new(), Some(state.clone())),
        Some(Value::Null) | None => (Vec::new(), None),
        Some(other) => return Err(format!("invalid 'state': {other}")),
    };
    if let Some(assets) = value.get("assets").and_then(Value::as_object) {
        input.extend(assets.iter().map(|(id, asset)| parse_asset(id, asset)));
    }
    Ok(UnifiedDecisionRequest {
        model,
        input,
        raw_input,
        questions: parse_questions(value.get("questions"), Criteria::TypeSafe)?,
        safety_identifier: string_field(value, "safety_identifier"),
        prompt_cache_key: string_field(value, "prompt_cache_key"),
    })
}

fn parse_asset(id: &str, asset: &Value) -> DecisionAsset {
    let kind = asset.get("type").and_then(Value::as_str).unwrap_or("image");
    let format = string_field(asset, "format");
    let source = match string_field(asset, "data_base64") {
        Some(data) => {
            let mime = format.as_deref().unwrap_or("application/octet-stream");
            format!("data:{mime};base64,{data}")
        }
        None => string_field(asset, "url").unwrap_or_default(),
    };
    DecisionAsset {
        media_type: format,
        ..DecisionAsset::new(kind, source).with_id(id)
    }
}

pub fn from_typesafe_response(
    value: &Value,
    requested_model: &str,
) -> Result<UnifiedDecisionResponse, String> {
    take_error(value)?;
    Ok(UnifiedDecisionResponse {
        model: string_field(value, "model").unwrap_or_else(|| requested_model.to_string()),
        answers: parse_answers(value)?,
        usage: usage_from(value),
    })
}

pub fn to_typesafe_response(resp: &UnifiedDecisionResponse) -> Value {
    let answers: Map<String, Value> = resp
        .answers
        .iter()
        .map(|result| (result.name.clone(), answer_value(result)))
        .collect();
    let usage = resp.usage.clone().unwrap_or_default();
    json!({
        "model": resp.model,
        "answers": answers,
        "usage": { "input_tokens": usage.input_tokens, "output_tokens": usage.output_tokens },
    })
}
