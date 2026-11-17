use serde_json::{json, Map, Value};

use super::common::{
    boxes_of, is_refusal, probability_map, score_probabilities, string_field, take_error,
    usage_from,
};
use super::{DecisionAnswer, DecisionQuestionKind, DecisionResult, UnifiedDecisionResponse};

pub fn from_openai_response(
    value: &Value,
    requested_model: &str,
) -> Result<UnifiedDecisionResponse, String> {
    take_error(value)?;
    let answers = value
        .get("answers")
        .and_then(Value::as_array)
        .ok_or("missing 'answers' array")?;
    let mut results = Vec::with_capacity(answers.len());
    for entry in answers {
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .ok_or("answer without 'name'")?;
        let kind = entry
            .get("type")
            .and_then(Value::as_str)
            .and_then(DecisionQuestionKind::parse)
            .unwrap_or(DecisionQuestionKind::Noul);
        results.push(DecisionResult {
            name: name.to_string(),
            answer: parse_answer(&kind, entry),
        });
    }
    Ok(UnifiedDecisionResponse {
        model: string_field(value, "model").unwrap_or_else(|| requested_model.to_string()),
        answers: results,
        usage: usage_from(value),
    })
}

fn parse_answer(kind: &DecisionQuestionKind, entry: &Value) -> DecisionAnswer {
    if is_refusal(entry) {
        return DecisionAnswer::Refusal;
    }
    let confidence = entry.get("confidence").and_then(Value::as_f64);
    match kind {
        DecisionQuestionKind::Noul => DecisionAnswer::Predicate {
            probability: entry
                .get("probability")
                .and_then(Value::as_f64)
                .or_else(|| {
                    entry
                        .get("selected")
                        .and_then(Value::as_bool)
                        .map(|selected| if selected { 1.0 } else { 0.0 })
                })
                .unwrap_or(0.0),
            confidence,
        },
        DecisionQuestionKind::Choice => DecisionAnswer::Choice {
            choice: entry
                .get("choice")
                .map(|choice| match choice {
                    Value::Object(map) => map.get("value").cloned().unwrap_or(Value::Null),
                    other => other.clone(),
                })
                .unwrap_or(Value::Null),
            probabilities: probability_map(entry.get("probabilities")),
            confidence,
        },
        DecisionQuestionKind::Score => DecisionAnswer::Score {
            score: entry
                .get("score")
                .or_else(|| entry.get("value"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            probabilities: score_probabilities(entry.get("probabilities")),
            legend: entry.get("legend").and_then(Value::as_object).cloned(),
            confidence,
        },
        DecisionQuestionKind::Box => DecisionAnswer::Box {
            boxes: boxes_of(entry.get("boxes").or_else(|| entry.get("box"))),
            confidence,
        },
    }
}

pub fn to_openai_response(resp: &UnifiedDecisionResponse) -> Value {
    let usage = resp.usage.clone().unwrap_or_default();
    json!({
        "model": resp.model,
        "answers": resp.answers.iter().map(answer_value).collect::<Vec<Value>>(),
        "usage": {
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "total_tokens": usage.total_tokens,
        },
    })
}

fn answer_value(result: &DecisionResult) -> Value {
    let mut entry = Map::new();
    entry.insert("name".into(), json!(result.name));
    let confidence = result.answer.confidence();
    match &result.answer {
        DecisionAnswer::Predicate { probability, .. } => {
            entry.insert("type".into(), json!("predicate"));
            entry.insert("selected".into(), json!(*probability >= 0.5));
            entry.insert("probability".into(), json!(probability));
        }
        DecisionAnswer::Choice {
            choice,
            probabilities,
            ..
        } => {
            entry.insert("type".into(), json!("choice"));
            entry.insert("choice".into(), choice.clone());
            let items: Vec<Value> = probabilities
                .iter()
                .map(|(value, probability)| json!({ "value": value, "probability": probability }))
                .collect();
            entry.insert("probabilities".into(), Value::Array(items));
        }
        DecisionAnswer::Score {
            score,
            probabilities,
            legend,
            ..
        } => {
            entry.insert("type".into(), json!("score"));
            entry.insert("score".into(), json!(score));
            let items: Vec<Value> = probabilities
                .iter()
                .map(|(label, description, probability)| match description {
                    Some(description) => json!({
                        "label": label,
                        "description": description,
                        "probability": probability,
                    }),
                    None => json!({ "label": label, "probability": probability }),
                })
                .collect();
            entry.insert("probabilities".into(), Value::Array(items));
            if let Some(legend) = legend {
                entry.insert("legend".into(), Value::Object(legend.clone()));
            }
        }
        DecisionAnswer::Box { boxes, .. } => {
            entry.insert("type".into(), json!("box"));
            entry.insert("boxes".into(), Value::Array(boxes.clone()));
        }
        DecisionAnswer::Refusal => {
            entry.insert("type".into(), json!("refusal"));
            entry.insert("refusal".into(), json!(true));
        }
    }
    if let Some(confidence) = confidence {
        entry.insert("confidence".into(), json!(confidence));
    }
    Value::Object(entry)
}
