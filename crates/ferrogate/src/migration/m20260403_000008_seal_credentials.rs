use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use sea_orm_migration::prelude::*;

use crate::crypto::SecretBox;
use crate::entities::{provider, provider_key};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let secrets = SecretBox::from_env().map_err(|err| DbErr::Custom(err.to_string()))?;

        for row in provider::Entity::find().all(db).await? {
            if row.api_key.is_empty() || SecretBox::is_sealed(&row.api_key) {
                continue;
            }
            let sealed = secrets
                .seal(&row.api_key)
                .map_err(|err| DbErr::Custom(err.to_string()))?;

            provider::Entity::update_many()
                .col_expr(provider::Column::ApiKey, Expr::value(sealed))
                .filter(provider::Column::Id.eq(row.id))
                .exec(db)
                .await?;
        }

        for row in provider_key::Entity::find().all(db).await? {
            if row.api_key.is_empty() || SecretBox::is_sealed(&row.api_key) {
                continue;
            }
            let sealed = secrets
                .seal(&row.api_key)
                .map_err(|err| DbErr::Custom(err.to_string()))?;

            provider_key::Entity::update_many()
                .col_expr(provider_key::Column::ApiKey, Expr::value(sealed))
                .filter(provider_key::Column::Id.eq(row.id))
                .exec(db)
                .await?;
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
