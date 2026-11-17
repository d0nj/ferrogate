use crate::{
    providers::{adapter_for, EmbeddingRequest, EmbeddingResponse},
    AppState,
};

pub async fn embed_upstream(state: &AppState, model: &str, text: &str) -> Option<Vec<f32>> {
    let request = EmbeddingRequest {
        model: model.to_string(),
        input: serde_json::Value::String(text.to_string()),
        encoding_format: None,
        dimensions: None,
        user: None,
    };

    let targets = state
        .routing
        .resolve_targets_with_pricing(model, &state.pricing)
        .await
        .ok()?;

    for target in targets {
        let response = adapter_for(target.kind)
            .embed(&state.http_client, &target.endpoint(), &request)
            .await;

        match response {
            Ok(response) => {
                if let Some(vector) = first_vector(&response) {
                    return Some(vector);
                }
            }
            Err(err) if err.is_retryable() => continue,
            Err(err) => {
                tracing::debug!(error = %err, model, "semantic embedding rejected");
                return None;
            }
        }
    }
    None
}

fn first_vector(response: &EmbeddingResponse) -> Option<Vec<f32>> {
    let value = response.data.first()?.embedding.as_array()?;
    value
        .iter()
        .map(|component| component.as_f64().map(|number| number as f32))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{EmbeddingData, EmbeddingUsage};
    use serde_json::json;

    fn response(embedding: serde_json::Value) -> EmbeddingResponse {
        EmbeddingResponse {
            object: "list".to_string(),
            data: vec![EmbeddingData {
                object: "embedding".to_string(),
                embedding,
                index: 0,
            }],
            model: "m".to_string(),
            usage: EmbeddingUsage {
                prompt_tokens: 0,
                total_tokens: 0,
            },
        }
    }

    #[test]
    fn reads_the_float_vector() {
        let vector = first_vector(&response(json!([0.5, -0.25, 1.0])));
        assert_eq!(vector, Some(vec![0.5, -0.25, 1.0]));
    }

    #[test]
    fn ignores_a_base64_payload() {
        assert_eq!(first_vector(&response(json!("AAAA"))), None);
    }

    #[test]
    fn tolerates_an_empty_result() {
        let empty = EmbeddingResponse {
            object: "list".to_string(),
            data: Vec::new(),
            model: "m".to_string(),
            usage: EmbeddingUsage {
                prompt_tokens: 0,
                total_tokens: 0,
            },
        };
        assert_eq!(first_vector(&empty), None);
    }
}
