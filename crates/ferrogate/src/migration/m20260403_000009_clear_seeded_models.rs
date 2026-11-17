use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use sea_orm_migration::prelude::*;

use crate::entities::provider;

#[derive(DeriveMigrationName)]
pub struct Migration;

const SEEDED: &[&str] = &[
    "prov-openai",
    "prov-anthropic",
    "prov-gemini",
    "prov-meta",
    "prov-nvidia",
    "prov-typesafe",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        provider::Entity::update_many()
            .col_expr(provider::Column::ModelsJson, Expr::value("[]"))
            .filter(provider::Column::Id.is_in(SEEDED.iter().map(std::string::ToString::to_string)))
            .exec(db)
            .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
