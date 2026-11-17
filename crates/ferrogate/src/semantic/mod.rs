pub mod embed;
pub mod store;
pub mod upstream;

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::http::HeaderMap;
use sea_orm::{DatabaseConnection, EntityTrait};
use serde_json::Value;

use crate::{
    cache::{cache_key, is_cache_eligible},
    entities::app_setting,
    error::AppResult,
    providers::{message_content_to_text, ChatCompletionRequest, ChatCompletionResponse},
    telemetry::{RoutingAttempt, TelemetryEvent},
    AppState,
};

pub use store::Backend;

use store::{EntryStore, NewEntry};

pub const THRESHOLD_KEY: &str = "semantic_cache_threshold";

pub const EMBEDDING_MODEL_KEY: &str = "semantic_embedding_model";

pub const DEFAULT_THRESHOLD: f32 = 0.95;

pub const THRESHOLD_RANGE: std::ops::RangeInclusive<f32> = 0.5..=1.0;

pub const SEMANTIC_TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub const MAX_TEMPERATURE: f64 = 0.2;

pub struct SemanticCache {
    db: DatabaseConnection,
    entries: EntryStore,
    threshold_millis: Arc<AtomicU64>,
    hits: Arc<AtomicU64>,
    embedding_model: Arc<std::sync::Mutex<Option<Arc<str>>>>,
}

impl SemanticCache {
    pub fn new(
        db: DatabaseConnection,
        qdrant_url: Option<String>,
        embedding_model: Option<String>,
    ) -> Self {
        Self {
            entries: EntryStore::new(db.clone(), qdrant_url),
            db,
            threshold_millis: Arc::new(AtomicU64::new(to_micros(DEFAULT_THRESHOLD))),
            hits: Arc::new(AtomicU64::new(0)),
            embedding_model: Arc::new(std::sync::Mutex::new(embedding_model.map(Arc::from))),
        }
    }

    async fn embed_prompt(&self, state: &AppState, prompt: &str) -> Option<Vec<f32>> {
        let model = self.embedding_model();
        match model {
            Some(model) => upstream::embed_upstream(state, &model, prompt).await,
            None => None,
        }
    }

    pub fn embedding_model(&self) -> Option<Arc<str>> {
        self.embedding_model
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub async fn set_embedding_model(&self, model: Option<String>) -> AppResult<()> {
        {
            let mut slot = self
                .embedding_model
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *slot = model.map(Arc::from);
        }
        self.clear().await
    }

    pub async fn load_threshold(&self, db: &DatabaseConnection) {
        if let Some(value) = read_threshold(db).await {
            self.set_threshold(value);
        }
    }

    pub async fn load_embedding_model(&self, db: &DatabaseConnection) {
        let row = app_setting::Entity::find_by_id(EMBEDDING_MODEL_KEY)
            .one(db)
            .await
            .ok()
            .flatten();
        let Some(row) = row else {
            return;
        };
        let chosen = (!row.value.trim().is_empty()).then(|| row.value.trim().to_string());
        let mut slot = self
            .embedding_model
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = chosen.map(Arc::from);
    }

    pub fn set_threshold(&self, value: f32) {
        self.threshold_millis
            .store(to_micros(value), Ordering::Relaxed);
    }

    pub fn threshold(&self) -> f32 {
        self.threshold_millis.load(Ordering::Relaxed) as f32 / 1_000_000.0
    }

    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    pub fn backend(&self) -> Backend {
        self.entries.backend()
    }

    pub async fn entry_count(&self) -> AppResult<u64> {
        self.entries.count().await
    }

    pub async fn clear(&self) -> AppResult<()> {
        let name = self.collection();
        self.entries.clear(&name).await
    }

    pub fn collection(&self) -> String {
        match self.embedding_model().as_deref() {
            Some(model) => EntryStore::collection_name(Some(model), None),
            None => EntryStore::collection_name(None, None),
        }
    }

    pub async fn sweep(&self) -> AppResult<u64> {
        self.entries.sweep().await
    }

    pub async fn lookup(
        &self,
        state: &AppState,
        request: &ChatCompletionRequest,
        headers: &HeaderMap,
        request_id: &str,
        key_id: Option<&str>,
        verdict: &crate::guardrails::InputVerdict,
    ) -> Option<Resolution> {
        if !eligible(request, headers) {
            return None;
        }

        let prompt = last_user_text(request)?;
        let embedding = self.embed_prompt(state, &prompt).await?;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        let candidates = match self
            .entries
            .search(
                &request.model,
                key_id,
                &embedding,
                self.threshold(),
                &now,
                &self.collection(),
            )
            .await
        {
            Ok(candidates) => candidates,
            Err(err) => {
                tracing::warn!(error = %err, "semantic lookup failed; falling through");
                return None;
            }
        };
        let best = candidates.into_iter().next()?;

        let mut response = best.response;
        response.id = format!("chatcmpl-{}", uuid::Uuid::new_v4());

        self.note_hit(&best.id).await;
        self.hits.fetch_add(1, Ordering::Relaxed);

        let cost_usd = replayed_cost(state, &response).await;
        state.telemetry.record(self.hit_event(
            state, request, request_id, &response, cost_usd, key_id, verdict,
        ));

        Some(Resolution { response, cost_usd })
    }

    pub async fn store(
        &self,
        state: &AppState,
        request: &ChatCompletionRequest,
        headers: &HeaderMap,
        completion: &ChatCompletionResponse,
        provider_name: Option<&str>,
        key_id: Option<&str>,
    ) {
        if !eligible(request, headers) || completion.choices.is_empty() {
            return;
        }
        let Some(prompt) = last_user_text(request) else {
            return;
        };
        let Some(embedding) = self.embed_prompt(state, &prompt).await else {
            return;
        };

        let expires_at = (chrono::Utc::now() + chrono::Duration::from_std(SEMANTIC_TTL).unwrap())
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        let collection = self.collection();
        self.entries
            .ensure_collection(&collection, embedding.len())
            .await;

        self.entries
            .insert(
                NewEntry {
                    cache_key: &cache_key(&request.model, request, key_id),
                    model: &request.model,
                    virtual_key_id: key_id,
                    provider_name,
                    prompt_text: &prompt,
                    embedding: &embedding,
                    response: completion,
                    expires_at: &expires_at,
                },
                &collection,
            )
            .await
            .ok();
    }

    async fn note_hit(&self, id: &str) {
        let _ = sqlx::query("UPDATE semantic_cache_entries SET hits = hits + 1 WHERE id = ?1")
            .bind(id)
            .execute(self.db.get_sqlite_connection_pool())
            .await;
    }

    #[allow(clippy::too_many_arguments)]
    fn hit_event(
        &self,
        state: &AppState,
        request: &ChatCompletionRequest,
        request_id: &str,
        response: &ChatCompletionResponse,
        cost_usd: f64,
        key_id: Option<&str>,
        verdict: &crate::guardrails::InputVerdict,
    ) -> TelemetryEvent {
        let usage = response.usage.clone().unwrap_or_default();
        let mut event = TelemetryEvent::new(&request.model, false);
        event.request_id = Some(request_id.to_string());
        event.virtual_key_id = key_id.map(str::to_string);
        event.cache_hit = true;
        event.cache_type = Some("semantic".to_string());
        if !verdict.is_clean() {
            event.guardrail_action =
                crate::guardrails::action_label(&verdict.flags, &verdict.redactions);
        }
        event.input_tokens = usage.prompt_tokens;
        event.output_tokens = usage.completion_tokens;
        event.cost_usd = cost_usd;
        event.routed_provider_name = Some("cache".to_string());
        event.routed_model = Some(response.model.clone());
        event.routing_trace = vec![RoutingAttempt {
            provider_id: "cache".to_string(),
            provider_name: "cache".to_string(),
            model: response.model.clone(),
            status_code: 200,
            latency_ms: 0.0,
            error: None,
        }];
        event.response_text = if state.redact_prompts.load(Ordering::Relaxed) {
            None
        } else {
            response
                .choices
                .first()
                .map(|choice| message_content_to_text(&choice.message.content))
        };
        event
    }
}

pub struct Resolution {
    pub response: ChatCompletionResponse,
    pub cost_usd: f64,
}

fn eligible(request: &ChatCompletionRequest, headers: &HeaderMap) -> bool {
    if request.stream.unwrap_or(false) || !is_cache_eligible(request, headers) {
        return false;
    }
    if !request
        .messages
        .iter()
        .any(|message| message.role == "user")
    {
        return false;
    }
    match request.temperature {
        None => true,
        Some(temperature) => temperature <= MAX_TEMPERATURE,
    }
}

fn last_user_text(request: &ChatCompletionRequest) -> Option<String> {
    let text = request
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message_content_to_text(&message.content))
        .unwrap_or_default();

    let normalised = embed::normalise(&text);
    (!normalised.is_empty()).then_some(normalised)
}

async fn replayed_cost(state: &AppState, response: &ChatCompletionResponse) -> f64 {
    let usage = response.usage.clone().unwrap_or_default();
    state
        .pricing
        .compute_cost_usd(
            &response.model,
            usage.prompt_tokens,
            usage.completion_tokens,
        )
        .await
}

pub async fn read_threshold(db: &DatabaseConnection) -> Option<f32> {
    let row = app_setting::Entity::find_by_id(THRESHOLD_KEY)
        .one(db)
        .await
        .ok()
        .flatten()?;
    parse_threshold(&row.value)
}

pub fn parse_threshold(raw: &str) -> Option<f32> {
    let value = raw.trim().parse::<f32>().ok()?;
    THRESHOLD_RANGE.contains(&value).then_some(value)
}

pub const SEMANTIC_HIT_HEADER: &str = "semantic";

pub fn replay_body(response: &ChatCompletionResponse) -> Value {
    serde_json::to_value(response).unwrap_or(Value::Null)
}

fn to_micros(value: f32) -> u64 {
    (value.max(0.0) as f64 * 1_000_000.0).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_out_of_range_thresholds() {
        assert_eq!(parse_threshold("0.97"), Some(0.97));
        assert_eq!(parse_threshold("0.4"), None);
        assert_eq!(parse_threshold("1.2"), None);
        assert_eq!(parse_threshold("nope"), None);
    }

    #[test]
    fn temperature_gate_uses_last_user_message() {
        let request: ChatCompletionRequest = serde_json::from_value(serde_json::json!({
            "model": "gpt-4o",
            "temperature": 0.0,
            "messages": [
                { "role": "system", "content": "be terse" },
                { "role": "user", "content": "Hello   There" }
            ]
        }))
        .expect("request");
        assert_eq!(last_user_text(&request).as_deref(), Some("hello there"));
        assert!(eligible(&request, &HeaderMap::new()));
    }
}
