use sea_orm::{sea_query::OnConflict, ConnectionTrait, DbBackend, EntityTrait, Set, Statement};
use sea_orm_migration::prelude::*;

use crate::entities::{app_setting, model_pricing, provider, routing_rule, virtual_key};

const SEEDED_AT: &str = "2026-01-01T00:00:00Z";

const DEMO_KEY_HASH: &str = "e535a187c1ca4492aa1e9406c799219a1d99ae34b3980dfdd37ff9619380d51b";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        model_pricing::Entity::insert_many(pricing_rows())
            .on_conflict(
                OnConflict::column(model_pricing::Column::Model)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(db)
            .await?;

        provider::Entity::insert_many(provider_rows())
            .on_conflict(
                OnConflict::column(provider::Column::Id)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(db)
            .await?;

        for statement in routing_rule_seed_statements() {
            db.execute_raw(statement).await?;
        }

        virtual_key::Entity::insert(virtual_key::ActiveModel {
            id: Set("key-demo-local".to_owned()),
            name: Set("Local Demo Key".to_owned()),
            key_prefix: Set("sk-fg-demo...".to_owned()),
            key_hash: Set(DEMO_KEY_HASH.to_owned()),
            budget_usd: Set(Some(50.0)),
            spent_usd: Set(0.0),
            rpm_limit: Set(Some(120)),
            tpm_limit: Set(Some(200_000)),
            allowed_models_json: Set("[]".to_owned()),
            is_active: Set(0),
            created_at: Set(SEEDED_AT.to_owned()),
            last_used_at: Set(None),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::column(virtual_key::Column::Id)
                .do_nothing()
                .to_owned(),
        )
        .exec(db)
        .await?;

        app_setting::Entity::insert(app_setting::ActiveModel {
            key: Set("redact_prompts".to_owned()),
            value: Set("false".to_owned()),
            updated_at: Set(SEEDED_AT.to_owned()),
        })
        .on_conflict(
            OnConflict::column(app_setting::Column::Key)
                .do_nothing()
                .to_owned(),
        )
        .exec(db)
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        app_setting::Entity::delete_by_id("redact_prompts")
            .exec(db)
            .await?;
        virtual_key::Entity::delete_by_id("key-demo-local")
            .exec(db)
            .await?;
        routing_rule::Entity::delete_many().exec(db).await?;
        provider::Entity::delete_many().exec(db).await?;
        model_pricing::Entity::delete_many().exec(db).await?;
        Ok(())
    }
}

fn pricing_rows() -> Vec<model_pricing::ActiveModel> {
    [
        ("gpt-4o", "openai", 2.50, 10.00),
        ("gpt-4o-mini", "openai", 0.15, 0.60),
        ("gpt-4.1", "openai", 2.00, 8.00),
        ("gpt-4.1-mini", "openai", 0.40, 1.60),
        ("o3-mini", "openai", 1.10, 4.40),
        ("claude-sonnet-4-20250514", "anthropic", 3.00, 15.00),
        ("claude-3-5-haiku-latest", "anthropic", 0.80, 4.00),
        ("claude-opus-4-20250514", "anthropic", 15.00, 75.00),
        ("gemini-2.5-pro", "gemini", 1.25, 10.00),
        ("gemini-2.5-flash", "gemini", 0.30, 2.50),
        ("gemini-2.0-flash", "gemini", 0.10, 0.40),
        ("llama-3.3-70b-versatile", "groq", 0.59, 0.79),
    ]
    .into_iter()
    .map(
        |(model, provider_type, input, output)| model_pricing::ActiveModel {
            model: Set(model.to_owned()),
            provider_type: Set(provider_type.to_owned()),
            input_cost_per_1m: Set(input),
            output_cost_per_1m: Set(output),
            updated_at: Set(SEEDED_AT.to_owned()),
        },
    )
    .collect()
}

fn provider_rows() -> Vec<provider::ActiveModel> {
    [
        (
            "prov-openai",
            "OpenAI",
            "openai",
            "https://api.openai.com",
            r#"["gpt-4o","gpt-4o-mini","gpt-4.1","gpt-4.1-mini","o3-mini"]"#,
        ),
        (
            "prov-anthropic",
            "Anthropic",
            "anthropic",
            "https://api.anthropic.com",
            r#"["claude-sonnet-4-20250514","claude-3-5-haiku-latest","claude-opus-4-20250514"]"#,
        ),
        (
            "prov-gemini",
            "Google Gemini",
            "gemini",
            "https://generativelanguage.googleapis.com",
            r#"["gemini-2.5-pro","gemini-2.5-flash","gemini-2.0-flash"]"#,
        ),
    ]
    .into_iter()
    .map(
        |(id, name, provider_type, base_url, models_json)| provider::ActiveModel {
            id: Set(id.to_owned()),
            name: Set(name.to_owned()),
            provider_type: Set(provider_type.to_owned()),
            base_url: Set(base_url.to_owned()),
            api_key: Set(String::new()),
            models_json: Set(models_json.to_owned()),
            is_active: Set(0),
            timeout_ms: Set(60_000),
            created_at: Set(SEEDED_AT.to_owned()),
            updated_at: Set(SEEDED_AT.to_owned()),
            ..Default::default()
        },
    )
    .collect()
}

const ROUTING_RULES: &[(&str, &str, &str, &str, i64)] = &[
    (
        "rule-default-openai",
        "default",
        "prov-openai",
        "gpt-4o-mini",
        0,
    ),
    (
        "rule-default-anthropic",
        "default",
        "prov-anthropic",
        "claude-3-5-haiku-latest",
        1,
    ),
    (
        "rule-default-gemini",
        "default",
        "prov-gemini",
        "gemini-2.5-flash",
        2,
    ),
    ("rule-smart-openai", "smart", "prov-openai", "gpt-4.1", 0),
    (
        "rule-smart-anthropic",
        "smart",
        "prov-anthropic",
        "claude-sonnet-4-20250514",
        1,
    ),
    (
        "rule-smart-gemini",
        "smart",
        "prov-gemini",
        "gemini-2.5-pro",
        2,
    ),
    ("rule-fast-openai", "fast", "prov-openai", "gpt-4o-mini", 0),
    (
        "rule-fast-anthropic",
        "fast",
        "prov-anthropic",
        "claude-3-5-haiku-latest",
        1,
    ),
    (
        "rule-fast-gemini",
        "fast",
        "prov-gemini",
        "gemini-2.0-flash",
        2,
    ),
];

fn routing_rule_seed_statements() -> Vec<Statement> {
    ROUTING_RULES
        .iter()
        .map(|(id, alias, provider_id, upstream_model, priority)| {
            Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "INSERT INTO routing_rules \
                 (id, alias, provider_id, upstream_model, weight, priority, is_active, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, 100, ?, 1, ?, ?) \
                 ON CONFLICT (id) DO NOTHING",
                [
                    (*id).into(),
                    (*alias).into(),
                    (*provider_id).into(),
                    (*upstream_model).into(),
                    (*priority).into(),
                    SEEDED_AT.into(),
                    SEEDED_AT.into(),
                ],
            )
        })
        .collect()
}
