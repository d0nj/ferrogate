use sea_orm::DbErr;
use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Sessions::Table)
                    .if_not_exists()
                    .col(string(Sessions::Token).primary_key())
                    .col(string(Sessions::UserId))
                    .col(string(Sessions::Email))
                    .col(string(Sessions::Role))
                    .col(big_integer(Sessions::IssuedAt))
                    .col(big_integer(Sessions::ExpiresAt))
                    .col(string(Sessions::NodeId))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_user_id")
                    .table(Sessions::Table)
                    .col(Sessions::UserId)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_expires_at")
                    .table(Sessions::Table)
                    .col(Sessions::ExpiresAt)
                    .if_not_exists()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Sessions::Table).if_exists().to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Token,
    UserId,
    Email,
    Role,
    IssuedAt,
    ExpiresAt,
    NodeId,
}
