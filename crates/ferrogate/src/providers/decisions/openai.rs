use serde_json::{json, Map, Value};

use super::{DecisionAsset, DecisionQuestion, DecisionQuestionKind, UnifiedDecisionRequest};

pub fn to_openai_body(req: &UnifiedDecisionRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert("input".into(), input_value(req));
    body.insert("questions".into(), questions_value(req));
    if let Some(identifier) = &req.safety_identifier {
        body.insert("safety_identifier".into(), json!(identifier));
    }
    if let Some(key) = &req.prompt_cache_key {
        body.insert("prompt_cache_key".into(), json!(key));
    }
    Value::Object(body)
}

fn input_value(req: &UnifiedDecisionRequest) -> Value {
    let parts: Vec<Value> = req.binary_assets().map(input_part).collect();
    let text = req.text_input();
    if parts.is_empty() {
        return req.raw_input.clone().unwrap_or(Value::String(text));
    }
    let mut content = Vec::with_capacity(parts.len() + 1);
    if !text.is_empty() {
        content.push(json!({ "type": "input_text", "text": text }));
    }
    content.extend(parts);
    json!([{ "role": "user", "content": content }])
}

fn input_part(asset: &DecisionAsset) -> Value {
    let url = &asset.source;
    match asset.kind.as_str() {
        "image" => json!({ "type": "input_image", "image_url": url }),
        "audio" => json!({ "type": "input_audio", "audio_url": url }),
        "video" => json!({ "type": "input_video", "video_url": url }),
        "document" => json!({
            "type": "input_file",
            "file_url": url,
            "mime_type": asset.resolved_media_type(),
        }),
        "json" => json!({ "type": "input_json", "json": asset.json_data.clone() }),
        other => json!({ "type": format!("input_{other}"), "url": url }),
    }
}

fn questions_value(req: &UnifiedDecisionRequest) -> Value {
    Value::Array(req.questions.iter().map(question_value).collect())
}

fn question_value(question: &DecisionQuestion) -> Value {
    let mut entry = Map::new();
    entry.insert("name".into(), json!(question.name));
    entry.insert(
        "instructions".into(),
        json!(question.resolved_instructions()),
    );
    match question.kind {
        DecisionQuestionKind::Noul => {
            let (when_true, when_false) = question.resolved_descriptions();
            entry.insert("type".into(), json!("predicate"));
            if question.true_description.is_some() || question.instructions.is_some() {
                entry.insert("true_description".into(), json!(when_true));
            }
            entry.insert("false_description".into(), json!(when_false));
        }
        DecisionQuestionKind::Choice => {
            let choices: Vec<Value> = question
                .choices
                .iter()
                .map(|option| match &option.description {
                    Some(description) => {
                        json!({ "value": option.value, "description": description })
                    }
                    None => json!({ "value": option.value }),
                })
                .collect();
            entry.insert("type".into(), json!("choice"));
            entry.insert("choices".into(), Value::Array(choices));
        }
        DecisionQuestionKind::Score => {
            let levels: Vec<Value> = question
                .levels
                .iter()
                .map(|level| match &level.description {
                    Some(description) => {
                        json!({ "label": level.label, "description": description })
                    }
                    None => json!({ "label": level.label }),
                })
                .collect();
            entry.insert("type".into(), json!("score"));
            entry.insert("levels".into(), Value::Array(levels));
        }
        DecisionQuestionKind::Box => {
            entry.insert("type".into(), json!("box"));
        }
    }
    Value::Object(entry)
}

pub(crate) fn json_payload(value: Option<&Value>) -> Value {
    value.cloned().unwrap_or(Value::Null)
}
