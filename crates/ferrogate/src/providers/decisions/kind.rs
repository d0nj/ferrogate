use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::common::label;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionQuestionKind {
    Noul,
    Choice,
    Score,
    Box,
}

impl DecisionQuestionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Noul => "noul",
            Self::Choice => "choice",
            Self::Score => "score",
            Self::Box => "box",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "noul" | "predicate" | "boolean" => Some(Self::Noul),
            "choice" => Some(Self::Choice),
            "score" => Some(Self::Score),
            "box" => Some(Self::Box),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionChoiceOption {
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl DecisionChoiceOption {
    pub fn new(value: Value) -> Self {
        Self {
            value,
            description: None,
        }
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self::new(Value::String(value.into()))
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub(crate) fn key(&self) -> String {
        label(&self.value)
    }

    pub(crate) fn criteria(&self) -> (String, String) {
        let key = self.key();
        (key.clone(), self.description.clone().unwrap_or(key))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionScoreLevel {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl DecisionScoreLevel {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub(crate) fn criteria(&self) -> String {
        self.description
            .clone()
            .unwrap_or_else(|| self.label.clone())
    }

    pub(crate) fn parse(level: &Value) -> Self {
        let parsed = match level {
            Value::Object(map) => Self::new(
                map.get("label")
                    .or_else(|| map.get("value"))
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            other => Self::new(label(other)),
        };
        match level.get("description").and_then(Value::as_str) {
            Some(description) => parsed.with_description(description),
            None => parsed,
        }
    }
}
