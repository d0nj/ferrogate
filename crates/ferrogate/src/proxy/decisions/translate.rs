use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    providers::decisions::{
        from_gemini_request, from_openai_request, from_typesafe_request, to_gemini_response,
        to_openai_response, to_typesafe_response, UnifiedDecisionRequest, UnifiedDecisionResponse,
    },
    tokenizer::count_text_tokens,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    OpenAi,
    TypeSafe,
    Gemini,
}

impl Dialect {
    pub(crate) fn endpoint(self) -> &'static str {
        match self {
            Self::OpenAi => "/v1/decisions",
            Self::TypeSafe => "/v1/systemone",
            Self::Gemini => "/v1beta/decisions",
        }
    }

    pub(crate) fn decode(self, raw: &Value) -> AppResult<UnifiedDecisionRequest> {
        let decoded = match self {
            Self::OpenAi => from_openai_request(raw),
            Self::TypeSafe => from_typesafe_request(raw),
            Self::Gemini => from_gemini_request(raw),
        };
        decoded.map_err(|err| AppError::BadRequest(format!("invalid request body: {err}")))
    }

    pub(crate) fn encode(self, response: &UnifiedDecisionResponse) -> Value {
        match self {
            Self::OpenAi => to_openai_response(response),
            Self::TypeSafe => to_typesafe_response(response),
            Self::Gemini => to_gemini_response(response),
        }
    }

    pub(crate) fn input_tokens(self, request: &UnifiedDecisionRequest) -> u32 {
        let mut total = 0;
        for asset in &request.input {
            match (&asset.json_data, asset.source.as_str()) {
                (Some(payload), _) => {
                    total += count_text_tokens(&request.model, &payload.to_string())
                }
                (None, text) => total += count_text_tokens(&request.model, text),
            }
        }
        for question in &request.questions {
            total += count_text_tokens(&request.model, question.resolved_instructions());
            let (when_true, when_false) = question.resolved_descriptions();
            total += count_text_tokens(&request.model, &when_true);
            total += count_text_tokens(&request.model, &when_false);
            for choice in &question.choices {
                total += count_text_tokens(&request.model, &choice.value.to_string());
            }
        }
        total.max(1)
    }
}

pub(crate) fn typesafe_models(models: Vec<String>) -> Value {
    let models: Vec<Value> = models
        .into_iter()
        .map(|name| json!({ "name": name, "description": "", "release_date": "" }))
        .collect();
    json!({ "models": models })
}
