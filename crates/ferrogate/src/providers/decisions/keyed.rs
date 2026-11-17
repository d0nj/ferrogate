use serde_json::{json, Map, Value};

use super::common::{
    boxes_of, is_refusal, probability_map, probability_object, probability_of, score_probabilities,
    score_probability_object, string_field,
};
use super::{
    DecisionAnswer, DecisionChoiceOption, DecisionQuestion, DecisionQuestionKind, DecisionResult,
    DecisionScoreLevel, UnifiedDecisionRequest,
};

#[derive(Clone, Copy)]
pub(crate) enum Criteria {
    TypeSafe,
    Gemini,
}

impl Criteria {
    fn true_key(self) -> &'static str {
        match self {
            Self::TypeSafe => "true",
            Self::Gemini => "true_description",
        }
    }

    fn false_key(self) -> &'static str {
        match self {
            Self::TypeSafe => "false",
            Self::Gemini => "false_description",
        }
    }
}

fn criteria_options(
    name: &str,
    criteria: Option<&Value>,
) -> Result<Vec<DecisionChoiceOption>, String> {
    let choices: Vec<DecisionChoiceOption> = criteria
        .and_then(Value::as_object)
        .map(|criteria| {
            criteria
                .iter()
                .map(|(value, description)| {
                    let option = DecisionChoiceOption::text(value.clone());
                    match description.as_str() {
                        Some(text) if text != value => option.with_description(text),
                        _ => option,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if choices.is_empty() {
        return Err(format!("choice question '{name}' has no options"));
    }
    Ok(choices)
}

fn levels_of(criteria: Option<&Value>) -> Vec<DecisionScoreLevel> {
    criteria
        .and_then(Value::as_array)
        .map(|levels| levels.iter().map(DecisionScoreLevel::parse).collect())
        .unwrap_or_default()
}

pub(crate) fn question_entry(question: &DecisionQuestion, dialect: Criteria) -> Value {
    let mut entry = Map::new();
    entry.insert("type".into(), json!(question.kind.as_str()));
    entry.insert(
        "instructions".into(),
        json!(question.resolved_instructions()),
    );
    match question.kind {
        DecisionQuestionKind::Noul => {
            let (when_true, when_false) = question.resolved_descriptions();
            entry.insert(
                "criteria".into(),
                json!({ dialect.true_key(): when_true, dialect.false_key(): when_false }),
            );
        }
        DecisionQuestionKind::Choice => {
            let criteria: Map<String, Value> = question
                .choices
                .iter()
                .map(|option| {
                    let (key, text) = option.criteria();
                    (key, json!(text))
                })
                .collect();
            entry.insert("criteria".into(), Value::Object(criteria));
        }
        DecisionQuestionKind::Score => {
            let criteria: Vec<Value> = question
                .levels
                .iter()
                .map(|level| json!(level.criteria()))
                .collect();
            entry.insert("criteria".into(), Value::Array(criteria));
        }
        DecisionQuestionKind::Box => {}
    }
    Value::Object(entry)
}

pub(crate) fn keyed_questions(
    req: &UnifiedDecisionRequest,
    dialect: Criteria,
) -> Map<String, Value> {
    req.questions
        .iter()
        .map(|question| (question.name.clone(), question_entry(question, dialect)))
        .collect()
}

pub(crate) fn parse_questions(
    questions: Option<&Value>,
    dialect: Criteria,
) -> Result<Vec<DecisionQuestion>, String> {
    let Some(questions) = questions else {
        return Ok(Vec::new());
    };
    let map = questions
        .as_object()
        .ok_or("'questions' must be an object")?;
    let mut parsed = Vec::with_capacity(map.len());
    for (name, entry) in map {
        let kind = entry
            .get("type")
            .and_then(Value::as_str)
            .and_then(DecisionQuestionKind::parse)
            .ok_or_else(|| format!("question '{name}' has an unsupported type"))?;
        let criteria = entry.get("criteria");
        let mut question = match kind {
            DecisionQuestionKind::Noul => DecisionQuestion {
                true_description: criteria.and_then(|c| string_field(c, dialect.true_key())),
                false_description: criteria.and_then(|c| string_field(c, dialect.false_key())),
                ..DecisionQuestion::noul(name)
            },
            DecisionQuestionKind::Choice => {
                DecisionQuestion::choice(name, criteria_options(name, criteria)?)
            }
            DecisionQuestionKind::Score => DecisionQuestion::score(name, levels_of(criteria)),
            DecisionQuestionKind::Box => DecisionQuestion::boxed(name),
        };
        question.instructions = string_field(entry, "instructions");
        parsed.push(question);
    }
    Ok(parsed)
}

pub(crate) fn parse_answers(value: &Value) -> Result<Vec<DecisionResult>, String> {
    let answers = value
        .get("answers")
        .and_then(Value::as_object)
        .ok_or("missing 'answers' object")?;
    Ok(answers
        .iter()
        .map(|(name, entry)| DecisionResult {
            name: name.clone(),
            answer: parse_answer(entry),
        })
        .collect())
}

fn parse_answer(entry: &Value) -> DecisionAnswer {
    if is_refusal(entry) {
        return DecisionAnswer::Refusal;
    }
    let confidence = entry.get("confidence").and_then(Value::as_f64);
    let value = entry.get("value");
    let kind = entry
        .get("type")
        .and_then(Value::as_str)
        .and_then(DecisionQuestionKind::parse)
        .unwrap_or(DecisionQuestionKind::Noul);
    match kind {
        DecisionQuestionKind::Noul => DecisionAnswer::Predicate {
            probability: probability_of(entry.get("noul").or(value)),
            confidence,
        },
        DecisionQuestionKind::Choice => DecisionAnswer::Choice {
            choice: entry
                .get("choice")
                .or(value)
                .map(unwrap_value)
                .unwrap_or(Value::Null),
            probabilities: probability_map(entry.get("probabilities")),
            confidence,
        },
        DecisionQuestionKind::Score => DecisionAnswer::Score {
            score: entry
                .get("score")
                .or(value)
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            probabilities: score_probabilities(entry.get("probabilities")),
            legend: entry.get("legend").and_then(Value::as_object).cloned(),
            confidence,
        },
        DecisionQuestionKind::Box => DecisionAnswer::Box {
            boxes: boxes_of(entry.get("box").or_else(|| entry.get("boxes"))),
            confidence,
        },
    }
}

fn unwrap_value(choice: &Value) -> Value {
    match choice {
        Value::Object(map) => map.get("value").cloned().unwrap_or(Value::Null),
        other => other.clone(),
    }
}

pub(crate) fn answer_value(result: &DecisionResult) -> Value {
    let mut entry = Map::new();
    let confidence = result.answer.confidence();
    match &result.answer {
        DecisionAnswer::Predicate { probability, .. } => {
            entry.insert("type".into(), json!("noul"));
            entry.insert("noul".into(), json!(probability));
            entry.insert("value".into(), json!(*probability >= 0.5));
        }
        DecisionAnswer::Choice {
            choice,
            probabilities,
            ..
        } => {
            entry.insert("type".into(), json!("choice"));
            entry.insert("choice".into(), choice.clone());
            entry.insert("probabilities".into(), probability_object(probabilities));
        }
        DecisionAnswer::Score {
            score,
            probabilities,
            legend,
            ..
        } => {
            entry.insert("type".into(), json!("score"));
            entry.insert("score".into(), json!(score));
            entry.insert(
                "probabilities".into(),
                score_probability_object(probabilities),
            );
            if let Some(legend) = legend {
                entry.insert("legend".into(), Value::Object(legend.clone()));
            }
        }
        DecisionAnswer::Box { boxes, .. } => {
            entry.insert("type".into(), json!("box"));
            entry.insert("box".into(), Value::Array(boxes.clone()));
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
