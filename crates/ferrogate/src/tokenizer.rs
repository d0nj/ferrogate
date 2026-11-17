use std::sync::LazyLock;
use std::time::Duration;

use moka::future::Cache;
use sea_orm::{DatabaseConnection, EntityTrait};
use tiktoken_rs::tokenizer::{get_tokenizer, Tokenizer};
use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton, CoreBPE};

use crate::{entities::model_pricing, providers::ChatMessage};

const PRICING_TTL_SECS: u64 = 60;
const PRICING_CACHE_CAPACITY: u64 = 4_096;

const CHARS_PER_TOKEN: u32 = 4;
const TOKENS_PER_MESSAGE: u32 = 3;
const TOKENS_PER_NAME: u32 = 1;
const REPLY_PRIMING_TOKENS: u32 = 3;

fn bpe_for(model: &str) -> Option<&'static CoreBPE> {
    if let Some(tokenizer) = get_tokenizer(model) {
        return Some(match tokenizer {
            Tokenizer::O200kBase | Tokenizer::O200kHarmony => o200k_base_singleton(),
            Tokenizer::Cl100kBase => cl100k_base_singleton(),
            Tokenizer::R50kBase | Tokenizer::P50kBase | Tokenizer::P50kEdit | Tokenizer::Gpt2 => {
                fallback_bpe()?
            }
        });
    }

    fallback_bpe()
}

static FALLBACK_BPE: LazyLock<Option<CoreBPE>> =
    LazyLock::new(|| match tiktoken_rs::cl100k_base() {
        Ok(bpe) => Some(bpe),
        Err(err) => {
            tracing::warn!(error = %err, "cl100k_base unavailable; using heuristic token counts");
            None
        }
    });

fn fallback_bpe() -> Option<&'static CoreBPE> {
    FALLBACK_BPE.as_ref()
}

pub fn count_text_tokens(model: &str, text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }

    match bpe_for(model) {
        Some(bpe) => bpe.count_with_special_tokens(text) as u32,
        None => heuristic_tokens(text),
    }
}

fn heuristic_tokens(text: &str) -> u32 {
    let chars = text.chars().count() as u32;
    chars.div_ceil(CHARS_PER_TOKEN).max(1)
}

pub fn count_prompt_tokens(model: &str, messages: &[ChatMessage]) -> u32 {
    let mut total = REPLY_PRIMING_TOKENS;

    for message in messages {
        total = total.saturating_add(TOKENS_PER_MESSAGE);
        total = total.saturating_add(count_text_tokens(model, &message.role));

        let content = crate::providers::message_content_to_text(&message.content);
        if !content.is_empty() {
            total = total.saturating_add(count_text_tokens(model, &content));
        }

        if let Some(name) = &message.name {
            total = total.saturating_add(count_text_tokens(model, name));
            total = total.saturating_add(TOKENS_PER_NAME);
        }

        if let Some(tool_calls) = &message.tool_calls {
            total = total.saturating_add(count_serialized_wire_tokens(model, tool_calls));
        }
        if let Some(tool_call_id) = &message.tool_call_id {
            total = total.saturating_add(count_text_tokens(model, tool_call_id));
        }
    }

    total
}

fn count_serialized_wire_tokens(model: &str, value: &serde_json::Value) -> u32 {
    match serde_json::to_string(value) {
        Ok(encoded) => count_text_tokens(model, &encoded),
        Err(_) => heuristic_tokens(&value.to_string()),
    }
}

pub const UNKNOWN_MODEL_COST_PER_1M: (f64, f64) = (5.0, 15.0);

#[derive(Clone)]
pub struct PricingRegistry {
    db: DatabaseConnection,
    cache: Cache<String, (f64, f64)>,
}

impl PricingRegistry {
    pub fn new(db: DatabaseConnection) -> Self {
        let cache = Cache::builder()
            .max_capacity(PRICING_CACHE_CAPACITY)
            .time_to_live(Duration::from_secs(PRICING_TTL_SECS))
            .build();

        Self { db, cache }
    }

    pub async fn invalidate_all(&self) {
        self.cache.invalidate_all();
        self.cache.run_pending_tasks().await;
    }

    pub async fn unit_costs(&self, model: &str) -> (f64, f64) {
        if let Some(costs) = self.cache.get(model).await {
            return costs;
        }

        let costs = match model_pricing::Entity::find_by_id(model).one(&self.db).await {
            Ok(Some(row)) => (row.input_cost_per_1m, row.output_cost_per_1m),
            Ok(None) => {
                tracing::warn!(model, "no pricing row; applying unknown-model estimate");
                return UNKNOWN_MODEL_COST_PER_1M;
            }
            Err(err) => {
                tracing::warn!(model, error = %err, "pricing lookup failed; applying estimate");
                return UNKNOWN_MODEL_COST_PER_1M;
            }
        };

        self.cache.insert(model.to_string(), costs).await;
        costs
    }

    pub async fn compute_cost_usd(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) -> f64 {
        let (input_per_1m, output_per_1m) = self.unit_costs(model).await;

        (input_tokens as f64 / 1_000_000.0) * input_per_1m
            + (output_tokens as f64 / 1_000_000.0) * output_per_1m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(role: &str, content: serde_json::Value) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content,
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }

    #[test]
    fn counts_tokens_for_known_models() {
        assert_eq!(count_text_tokens("gpt-4o", "hello world"), 2);
        assert_eq!(count_text_tokens("gpt-4o-mini", "hello world"), 2);
        assert_eq!(
            count_text_tokens("claude-3-5-haiku-latest", "hello world"),
            2
        );
    }

    #[test]
    fn empty_text_is_free() {
        assert_eq!(count_text_tokens("gpt-4o", ""), 0);
        assert_eq!(count_prompt_tokens("gpt-4o", &[]), REPLY_PRIMING_TOKENS);
    }

    #[test]
    fn unknown_models_fall_back_to_cl100k() {
        let known = count_text_tokens("gpt-4o", "The quick brown fox");
        let unknown = count_text_tokens("some-local-model", "The quick brown fox");
        assert_eq!(known, unknown);
        assert!(unknown > 0);
    }

    #[test]
    fn longer_text_costs_more() {
        let short = count_text_tokens("gpt-4o", "hello");
        let long = count_text_tokens("gpt-4o", &"hello ".repeat(100));
        assert!(long > short * 10, "short={short} long={long}");
    }

    #[test]
    fn prompt_count_includes_framing_overhead() {
        let messages = vec![
            message("system", json!("You are terse.")),
            message("user", json!("hello world")),
        ];

        let counted = count_prompt_tokens("gpt-4o", &messages);

        let expected = REPLY_PRIMING_TOKENS
            + 2 * TOKENS_PER_MESSAGE
            + count_text_tokens("gpt-4o", "system")
            + count_text_tokens("gpt-4o", "You are terse.")
            + count_text_tokens("gpt-4o", "user")
            + 2;

        assert_eq!(counted, expected);
    }

    #[test]
    fn prompt_count_handles_content_parts_and_tool_calls() {
        let with_parts = count_prompt_tokens(
            "gpt-4o",
            &[message(
                "user",
                json!([
                    { "type": "text", "text": "describe" },
                    { "type": "text", "text": "this" }
                ]),
            )],
        );

        let mut with_tools = message("assistant", json!(""));
        with_tools.name = Some("assistant".into());
        with_tools.tool_calls = Some(json!([{ "id": "call_1", "type": "function" }]));
        let with_tools = count_prompt_tokens("gpt-4o", &[with_tools]);

        assert!(with_parts > REPLY_PRIMING_TOKENS + TOKENS_PER_MESSAGE);
        assert!(with_tools > REPLY_PRIMING_TOKENS + TOKENS_PER_MESSAGE);
    }

    async fn memory_db() -> DatabaseConnection {
        use sea_orm::{ConnectOptions, Database};

        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        Database::connect(options).await.expect("in-memory sqlite")
    }

    async fn seed_pricing(db: &DatabaseConnection) {
        use sea_orm::{ActiveModelTrait, ConnectionTrait, Schema, Set};

        let schema = Schema::new(db.get_database_backend());
        let statement = schema.create_table_from_entity(model_pricing::Entity);
        db.execute(&statement).await.expect("create model_pricing");

        model_pricing::ActiveModel {
            model: Set("gpt-4o".to_owned()),
            provider_type: Set("openai".to_owned()),
            input_cost_per_1m: Set(2.5),
            output_cost_per_1m: Set(10.0),
            updated_at: Set("2026-01-01T00:00:00Z".to_owned()),
        }
        .insert(db)
        .await
        .expect("insert pricing row");
    }

    #[tokio::test]
    async fn pricing_registry_reads_table_and_caches_hits() {
        let db = memory_db().await;
        seed_pricing(&db).await;

        let registry = PricingRegistry::new(db.clone());

        assert_eq!(registry.unit_costs("gpt-4o").await, (2.5, 10.0));
        assert_eq!(
            registry.unit_costs("unknown-model").await,
            UNKNOWN_MODEL_COST_PER_1M
        );

        let cost = registry
            .compute_cost_usd("gpt-4o", 1_000_000, 1_000_000)
            .await;
        assert!((cost - 12.5).abs() < 1e-9, "got {cost}");

        let cost = registry.compute_cost_usd("gpt-4o", 1_000, 500).await;
        assert!(
            (cost - (0.001 * 2.5 + 0.0005 * 10.0)).abs() < 1e-9,
            "got {cost}"
        );

        use sea_orm::{ActiveModelTrait, EntityTrait, Set};
        let row = model_pricing::Entity::find_by_id("gpt-4o")
            .one(&db)
            .await
            .unwrap()
            .expect("seeded pricing row");
        let mut active: model_pricing::ActiveModel = row.into();
        active.output_cost_per_1m = Set(20.0);
        active.update(&db).await.unwrap();

        let cached = registry.compute_cost_usd("gpt-4o", 0, 1_000_000).await;
        assert!(
            (cached - 10.0).abs() < 1e-9,
            "expected cached 10.0, got {cached}"
        );

        registry.invalidate_all().await;

        let refreshed = registry.compute_cost_usd("gpt-4o", 0, 1_000_000).await;
        assert!(
            (refreshed - 20.0).abs() < 1e-9,
            "expected refreshed 20.0, got {refreshed}"
        );
    }

    #[tokio::test]
    async fn pricing_registry_survives_missing_table() {
        let registry = PricingRegistry::new(memory_db().await);
        let (input, output) = UNKNOWN_MODEL_COST_PER_1M;
        let expected = (100.0 / 1_000_000.0) * input + (100.0 / 1_000_000.0) * output;
        assert!((registry.compute_cost_usd("gpt-4o", 100, 100).await - expected).abs() < 1e-9);
    }
}
