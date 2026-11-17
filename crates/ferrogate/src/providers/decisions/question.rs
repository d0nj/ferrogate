use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::asset::DecisionAsset;
use super::kind::{DecisionChoiceOption, DecisionQuestionKind, DecisionScoreLevel};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionQuestion {
    pub name: String,
    pub kind: DecisionQuestionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub true_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub false_description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<DecisionChoiceOption>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<DecisionScoreLevel>,
}

impl DecisionQuestion {
    fn bare(name: impl Into<String>, kind: DecisionQuestionKind) -> Self {
        Self {
            name: name.into(),
            kind,
            instructions: None,
            true_description: None,
            false_description: None,
            choices: Vec::new(),
            levels: Vec::new(),
        }
    }

    pub fn noul(name: impl Into<String>) -> Self {
        Self::bare(name, DecisionQuestionKind::Noul)
    }

    pub fn choice(name: impl Into<String>, choices: Vec<DecisionChoiceOption>) -> Self {
        Self {
            choices,
            ..Self::bare(name, DecisionQuestionKind::Choice)
        }
    }

    pub fn score(name: impl Into<String>, levels: Vec<DecisionScoreLevel>) -> Self {
        Self {
            levels,
            ..Self::bare(name, DecisionQuestionKind::Score)
        }
    }

    pub fn boxed(name: impl Into<String>) -> Self {
        Self::bare(name, DecisionQuestionKind::Box)
    }

    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }

    pub fn with_descriptions(
        mut self,
        when_true: impl Into<String>,
        when_false: impl Into<String>,
    ) -> Self {
        self.true_description = Some(when_true.into());
        self.false_description = Some(when_false.into());
        self
    }

    pub fn resolved_instructions(&self) -> &str {
        self.instructions.as_deref().unwrap_or(&self.name)
    }

    pub fn resolved_descriptions(&self) -> (String, String) {
        let when_true = self
            .true_description
            .clone()
            .or_else(|| self.instructions.clone())
            .unwrap_or_else(|| format!("The condition '{}' is satisfied.", self.name));
        let when_false = self
            .false_description
            .clone()
            .unwrap_or_else(|| format!("The condition '{}' is not satisfied.", self.name));
        (when_true, when_false)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnifiedDecisionRequest {
    pub model: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<DecisionAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<DecisionQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

impl UnifiedDecisionRequest {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            input: Vec::new(),
            raw_input: None,
            questions: Vec::new(),
            safety_identifier: None,
            prompt_cache_key: None,
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.input.push(DecisionAsset::new("text", text));
        self
    }

    pub fn with_asset(mut self, asset: DecisionAsset) -> Self {
        self.input.push(asset);
        self
    }

    pub fn with_question(mut self, question: DecisionQuestion) -> Self {
        self.questions.push(question);
        self
    }

    pub fn text_input(&self) -> String {
        let parts: Vec<&str> = self
            .input
            .iter()
            .filter(|asset| asset.kind == "text")
            .map(|asset| asset.source.as_str())
            .collect();
        parts.join("\n\n")
    }

    pub fn binary_assets(&self) -> impl Iterator<Item = &DecisionAsset> {
        self.input.iter().filter(|asset| asset.kind != "text")
    }

    pub fn needs_image_processing(&self) -> bool {
        self.input
            .iter()
            .any(|asset| asset.kind == "image" || asset.kind == "video")
            || self
                .questions
                .iter()
                .any(|question| question.kind == DecisionQuestionKind::Box)
    }
}
