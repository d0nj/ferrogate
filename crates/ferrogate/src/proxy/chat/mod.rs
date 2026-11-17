mod dispatch;
mod finish;

pub(crate) use dispatch::run;
pub(crate) use finish::{completion_response, first_message_text, Finish};

use crate::{
    error::{AppError, AppResult},
    providers::ChatCompletionRequest,
};

pub(crate) fn parse_request(raw: serde_json::Value) -> AppResult<ChatCompletionRequest> {
    serde_json::from_value(raw)
        .map_err(|err| AppError::BadRequest(format!("invalid request body: {err}")))
}

pub(crate) fn validate_request(request: &ChatCompletionRequest) -> AppResult<()> {
    if request.model.trim().is_empty() && request.template.is_none() {
        return Err(AppError::BadRequest(
            "field 'model' must be a non-empty string".to_string(),
        ));
    }
    if request.messages.is_empty() && request.template.is_none() {
        return Err(AppError::BadRequest(
            "field 'messages' must contain at least one message".to_string(),
        ));
    }
    Ok(())
}

pub(crate) async fn prefetch_unit_costs(
    state: &crate::AppState,
    model: &str,
) -> Option<(f64, f64)> {
    Some(state.pricing.unit_costs(model).await)
}

pub(crate) fn cost_from_units(
    units: Option<(f64, f64)>,
    input_tokens: u32,
    output_tokens: u32,
) -> f64 {
    let (input_per_1m, output_per_1m) = units.unwrap_or((0.0, 0.0));
    (input_tokens as f64 / 1_000_000.0) * input_per_1m
        + (output_tokens as f64 / 1_000_000.0) * output_per_1m
}

pub(crate) fn chat_sse() -> crate::proxy::streaming::Formatter {
    use crate::providers::StreamEvent;
    use axum::response::sse::Event;

    Box::new(|event: &StreamEvent| match event {
        StreamEvent::Chunk { data, .. } => vec![Event::default()
            .json_data(data)
            .unwrap_or_else(|_| Event::default().data("{}"))],
        StreamEvent::Done => vec![Event::default().data("[DONE]")],
    })
}
