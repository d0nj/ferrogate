use serde_json::Value;

use super::common::string_field;
use super::openai::json_payload;
use super::{
    DecisionAsset, DecisionChoiceOption, DecisionQuestion, DecisionQuestionKind,
    DecisionScoreLevel, UnifiedDecisionRequest,
};

pub fn from_openai_request(value: &Value) -> Result<UnifiedDecisionRequest, String> {
    let model = string_field(value, "model").ok_or("missing 'model'")?;
    let (input, raw_input) = parse_input(value.get("input"))?;
    Ok(UnifiedDecisionRequest {
        model,
        input,
        raw_input,
        questions: parse_questions(value.get("questions"))?,
        safety_identifier: string_field(value, "safety_identifier"),
        prompt_cache_key: string_field(value, "prompt_cache_key"),
    })
}

fn parse_input(input: Option<&Value>) -> Result<(Vec<DecisionAsset>, Option<Value>), String> {
    let mut assets = Vec::new();
    let Some(input) = input else {
        return Ok((assets, None));
    };
    match input {
        Value::String(text) => assets.push(DecisionAsset::new("text", text.clone())),
        Value::Array(entries) => {
            for entry in entries {
                match entry {
                    Value::String(text) => assets.push(DecisionAsset::new("text", text.clone())),
                    Value::Object(map) => match map.get("content") {
                        Some(Value::String(text)) => {
                            assets.push(DecisionAsset::new("text", text.clone()))
                        }
                        Some(Value::Array(parts)) => {
                            for part in parts {
                                assets.push(parse_part(part)?);
                            }
                        }
                        _ => assets.push(parse_part(entry)?),
                    },
                    other => return Err(format!("invalid input entry: {other}")),
                }
            }
        }
        Value::Object(_) => return Ok((assets, Some(input.clone()))),
        other => return Err(format!("invalid 'input': {other}")),
    }
    Ok((assets, None))
}

fn parse_part(part: &Value) -> Result<DecisionAsset, String> {
    let kind = part.get("type").and_then(Value::as_str).unwrap_or("text");
    Ok(match kind {
        "input_text" | "text" => DecisionAsset::new(
            "text",
            part.get("text").and_then(Value::as_str).unwrap_or_default(),
        ),
        "input_json" | "json" => {
            DecisionAsset::json(json_payload(part.get("json").or_else(|| part.get("value"))))
        }
        "input_image" | "image" => {
            let url = part
                .get("image_url")
                .and_then(|url| match url {
                    Value::String(url) => Some(url.as_str()),
                    Value::Object(map) => map.get("url").and_then(Value::as_str),
                    _ => None,
                })
                .or_else(|| part.get("url").and_then(Value::as_str))
                .ok_or("image part without 'image_url'")?;
            let asset = DecisionAsset::new("image", url);
            match string_field(part, "resolution") {
                Some(resolution) => asset.with_resolution(resolution),
                None => asset,
            }
        }
        "input_audio" | "input_video" | "input_file" => media_part(&kind[6..], part),
        "audio" | "video" | "document" => media_part(kind, part),
        other => return Err(format!("unsupported input part type '{other}'")),
    })
}

fn media_part(kind: &str, part: &Value) -> DecisionAsset {
    let url = part
        .get(format!("{kind}_url"))
        .or_else(|| part.get("url"))
        .or_else(|| part.get("file_url"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let asset = DecisionAsset::new(kind, url);
    match string_field(part, "mime_type") {
        Some(mime) => DecisionAsset {
            media_type: Some(mime),
            ..asset
        },
        None => asset,
    }
}

fn parse_questions(questions: Option<&Value>) -> Result<Vec<DecisionQuestion>, String> {
    let Some(questions) = questions else {
        return Ok(Vec::new());
    };
    let entries = questions.as_array().ok_or("'questions' must be an array")?;
    let mut parsed: Vec<DecisionQuestion> = Vec::with_capacity(entries.len());
    for entry in entries {
        let question = parse_question(entry)?;
        if parsed.iter().any(|seen| seen.name == question.name) {
            return Err(format!("duplicate question name '{}'", question.name));
        }
        parsed.push(question);
    }
    Ok(parsed)
}

fn parse_question(entry: &Value) -> Result<DecisionQuestion, String> {
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .ok_or("question without 'name'")?;
    let kind = entry
        .get("type")
        .and_then(Value::as_str)
        .and_then(DecisionQuestionKind::parse)
        .ok_or_else(|| format!("question '{name}' has an unsupported type"))?;
    let mut question = match kind {
        DecisionQuestionKind::Noul => DecisionQuestion {
            true_description: string_field(entry, "true_description"),
            false_description: string_field(entry, "false_description"),
            ..DecisionQuestion::noul(name)
        },
        DecisionQuestionKind::Choice => {
            let choices: Vec<DecisionChoiceOption> = entry
                .get("choices")
                .and_then(Value::as_array)
                .map(|options| options.iter().map(choice_option).collect())
                .unwrap_or_default();
            if choices.is_empty() {
                return Err(format!("choice question '{name}' has no options"));
            }
            DecisionQuestion::choice(name, choices)
        }
        DecisionQuestionKind::Score => {
            let levels = entry
                .get("levels")
                .and_then(Value::as_array)
                .map(|levels| levels.iter().map(DecisionScoreLevel::parse).collect())
                .unwrap_or_default();
            DecisionQuestion::score(name, levels)
        }
        DecisionQuestionKind::Box => DecisionQuestion::boxed(name),
    };
    question.instructions = string_field(entry, "instructions");
    Ok(question)
}

fn choice_option(option: &Value) -> DecisionChoiceOption {
    let Some(map) = option.as_object() else {
        return DecisionChoiceOption::new(option.clone());
    };
    let value = map
        .get("value")
        .cloned()
        .unwrap_or_else(|| Value::String(option.to_string()));
    match map.get("description").and_then(Value::as_str) {
        Some(description) => DecisionChoiceOption::new(value).with_description(description),
        None => DecisionChoiceOption::new(value),
    }
}
