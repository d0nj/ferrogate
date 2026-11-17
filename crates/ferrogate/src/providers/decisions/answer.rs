use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum DecisionAnswer {
    Predicate {
        probability: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
    },
    Choice {
        choice: Value,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        probabilities: Vec<(Value, f64)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
    },
    Score {
        score: f64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        probabilities: Vec<(String, Option<String>, f64)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        legend: Option<Map<String, Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
    },
    Box {
        boxes: Vec<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
    },
    Refusal,
}

impl DecisionAnswer {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Predicate { probability, .. } => Some(*probability >= 0.5),
            Self::Choice { choice, .. } => match choice {
                Value::Bool(flag) => Some(*flag),
                Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
                    "true" | "yes" => Some(true),
                    "false" | "no" => Some(false),
                    _ => None,
                },
                _ => None,
            },
            Self::Score { score, .. } => Some(*score != 0.0),
            Self::Box { boxes, .. } => Some(!boxes.is_empty()),
            Self::Refusal => None,
        }
    }

    pub fn as_probability(&self) -> Option<f64> {
        match self {
            Self::Predicate { probability, .. } => Some(*probability),
            _ => None,
        }
    }

    pub fn as_choice(&self) -> Option<&str> {
        match self {
            Self::Choice { choice, .. } => match choice {
                Value::String(text) => Some(text.as_str()),
                Value::Bool(true) => Some("true"),
                Value::Bool(false) => Some("false"),
                _ => None,
            },
            Self::Predicate { probability, .. } => {
                Some(if *probability >= 0.5 { "true" } else { "false" })
            }
            _ => None,
        }
    }

    pub fn as_score(&self) -> Option<f64> {
        match self {
            Self::Score { score, .. } => Some(*score),
            Self::Predicate { probability, .. } => Some(*probability),
            Self::Choice { choice, .. } => choice.as_str()?.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn as_boxes(&self) -> Option<&[Value]> {
        match self {
            Self::Box { boxes, .. } => Some(boxes.as_slice()),
            _ => None,
        }
    }

    pub fn chosen(&self) -> Option<&Value> {
        match self {
            Self::Choice { choice, .. } => Some(choice),
            _ => None,
        }
    }

    pub fn score_value(&self) -> Option<f64> {
        match self {
            Self::Score { score, .. } => Some(*score),
            _ => None,
        }
    }

    pub fn confidence(&self) -> Option<f64> {
        match self {
            Self::Predicate { confidence, .. }
            | Self::Choice { confidence, .. }
            | Self::Score { confidence, .. }
            | Self::Box { confidence, .. } => *confidence,
            Self::Refusal => None,
        }
    }

    pub fn is_refusal(&self) -> bool {
        matches!(self, Self::Refusal)
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Predicate { probability, .. } => json!(*probability >= 0.5),
            Self::Choice { choice, .. } => choice.clone(),
            Self::Score { score, .. } => json!(score),
            Self::Box { boxes, .. } => Value::Array(boxes.clone()),
            Self::Refusal => Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionResult {
    pub name: String,
    pub answer: DecisionAnswer,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}

impl DecisionUsage {
    pub fn new(input_tokens: u32, output_tokens: u32, total_tokens: Option<u32>) -> Self {
        Self {
            input_tokens,
            output_tokens,
            total_tokens: total_tokens
                .unwrap_or_else(|| input_tokens.saturating_add(output_tokens)),
        }
    }

    pub(crate) fn from_object(
        usage: &Map<String, Value>,
        inputs: &[&str],
        outputs: &[&str],
    ) -> Self {
        let first = |keys: &[&str]| {
            keys.iter()
                .find_map(|key| usage.get(*key).and_then(Value::as_u64))
                .unwrap_or(0) as u32
        };
        let total = usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .map(|value| value as u32);
        Self::new(first(inputs), first(outputs), total)
    }

    pub(crate) fn from_value(
        usage: Option<&Value>,
        inputs: &[&str],
        outputs: &[&str],
    ) -> Option<Self> {
        usage
            .and_then(Value::as_object)
            .map(|usage| Self::from_object(usage, inputs, outputs))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnifiedDecisionResponse {
    pub model: String,
    pub answers: Vec<DecisionResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<DecisionUsage>,
}

impl UnifiedDecisionResponse {
    pub fn answer_map(&self) -> Map<String, Value> {
        self.answers
            .iter()
            .map(|result| (result.name.clone(), result.answer.to_json()))
            .collect()
    }

    pub fn answer(&self, name: &str) -> Option<&DecisionAnswer> {
        self.answers
            .iter()
            .find(|result| result.name == name)
            .map(|result| &result.answer)
    }
}
