use sea_orm::{sea_query::OnConflict, EntityTrait, Set};
use sea_orm_migration::prelude::*;

use crate::entities::{model_pricing, provider};

const SEEDED_AT: &str = "2026-04-02T00:00:00Z";

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

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let _ = manager;
        Ok(())
    }
}

fn pricing_rows() -> Vec<model_pricing::ActiveModel> {
    [
        ("text-embedding-3-small", "openai", 0.02, 0.0),
        ("text-embedding-3-large", "openai", 0.13, 0.0),
        ("text-embedding-004", "gemini", 0.0, 0.0),
        ("nvidia/nv-embedqa-e5-v5", "openai", 0.0, 0.0),
        ("Llama-3.3-70B-Instruct", "openai", 0.60, 0.70),
        ("meta/llama-3.3-70b-instruct", "openai", 0.60, 0.70),
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
            "prov-meta",
            "Meta Models API",
            "https://api.meta.ai/v1",
            r#"["Llama-4-Maverick-17B-128E-Instruct-FP8","Llama-4-Scout-17B-16E-Instruct","Llama-3.3-70B-Instruct"]"#,
        ),
        (
            "prov-nvidia",
            "NVIDIA NIM",
            "https://integrate.api.nvidia.com/v1",
            r#"["meta/llama-3.3-70b-instruct","nvidia/llama-3.3-nemotron-super-49b-v1","deepseek-ai/deepseek-r1","nvidia/nv-embedqa-e5-v5"]"#,
        ),
    ]
    .into_iter()
    .map(|(id, name, base_url, models_json)| provider::ActiveModel {
        id: Set(id.to_owned()),
        name: Set(name.to_owned()),
        provider_type: Set("openai".to_owned()),
        base_url: Set(base_url.to_owned()),
        api_key: Set(String::new()),
        models_json: Set(models_json.to_owned()),
        is_active: Set(0),
        timeout_ms: Set(60_000),
        created_at: Set(SEEDED_AT.to_owned()),
        updated_at: Set(SEEDED_AT.to_owned()),
        ..Default::default()
    })
    .collect()
}
