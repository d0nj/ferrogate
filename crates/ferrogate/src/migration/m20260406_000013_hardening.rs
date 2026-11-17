use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use sea_orm_migration::prelude::*;

use crate::{
    auth::password::verify_password,
    crypto::SecretBox,
    entities::{user, virtual_key, webhook},
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        add_column(manager, "users", "must_change_password").await?;
        add_column(manager, "semantic_cache_entries", "virtual_key_id").await?;

        let db = manager.get_connection();

        virtual_key::Entity::update_many()
            .col_expr(virtual_key::Column::IsActive, Expr::value(0))
            .filter(virtual_key::Column::Id.eq("key-demo-local"))
            .exec(db)
            .await?;

        if let Some(row) = user::Entity::find()
            .filter(user::Column::Email.eq("admin@ferrogate.local"))
            .one(db)
            .await?
        {
            if verify_password("ferrogate", &row.password_hash) {
                let mut active: user::ActiveModel = row.into();
                active.must_change_password = Set(1);
                active.update(db).await?;
            }
        }

        let secrets = SecretBox::from_env().map_err(|err| DbErr::Custom(err.to_string()))?;
        for row in webhook::Entity::find().all(db).await? {
            let Some(secret) = row.secret.clone() else {
                continue;
            };
            if secret.is_empty() || SecretBox::is_sealed(&secret) {
                continue;
            }
            let sealed = secrets
                .seal(&secret)
                .map_err(|err| DbErr::Custom(err.to_string()))?;
            webhook::Entity::update_many()
                .col_expr(webhook::Column::Secret, Expr::value(sealed))
                .filter(webhook::Column::Id.eq(row.id))
                .exec(db)
                .await?;
        }

        let _ = db
            .execute_unprepared("DELETE FROM semantic_cache_entries")
            .await;
        let _ = db.execute_unprepared("DELETE FROM sessions").await;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

async fn add_column(manager: &SchemaManager<'_>, table: &str, column: &str) -> Result<(), DbErr> {
    let ddl = match (table, column) {
        ("users", "must_change_password") => {
            "ALTER TABLE users ADD COLUMN must_change_password INTEGER NOT NULL DEFAULT 0"
        }
        ("semantic_cache_entries", "virtual_key_id") => {
            "ALTER TABLE semantic_cache_entries ADD COLUMN virtual_key_id TEXT"
        }
        _ => return Ok(()),
    };
    match manager.get_connection().execute_unprepared(ddl).await {
        Ok(_) => Ok(()),
        Err(err)
            if err
                .to_string()
                .to_ascii_lowercase()
                .contains("duplicate column") =>
        {
            Ok(())
        }
        Err(err) => Err(err),
    }
}
