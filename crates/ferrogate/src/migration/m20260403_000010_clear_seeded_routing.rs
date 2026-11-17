use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use sea_orm_migration::prelude::*;

use crate::entities::routing_rule;

#[derive(DeriveMigrationName)]
pub struct Migration;

const SEEDED: &[&str] = &[
    "rule-default-openai",
    "rule-default-anthropic",
    "rule-default-gemini",
    "rule-smart-openai",
    "rule-smart-anthropic",
    "rule-smart-gemini",
    "rule-fast-openai",
    "rule-fast-anthropic",
    "rule-fast-gemini",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        routing_rule::Entity::delete_many()
            .filter(
                routing_rule::Column::Id.is_in(SEEDED.iter().map(std::string::ToString::to_string)),
            )
            .exec(db)
            .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
