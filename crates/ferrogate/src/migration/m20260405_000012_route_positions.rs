use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum RoutingRules {
    Table,
    Alias,
    Position,
    Priority,
    IsActive,
}

const ALIAS_INDEX: &str = "idx_routing_alias_active";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.add_position(manager).await?;
        self.backfill_positions(manager).await?;

        manager
            .drop_index(Index::drop().name(ALIAS_INDEX).to_owned())
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(RoutingRules::Table)
                    .drop_column(RoutingRules::Priority)
                    .to_owned(),
            )
            .await?;

        self.create_alias_index(manager, RoutingRules::Position)
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(RoutingRules::Table)
                    .add_column(integer(RoutingRules::Priority).default(0))
                    .to_owned(),
            )
            .await?;
        manager
            .exec_stmt(
                Query::update()
                    .table(RoutingRules::Table)
                    .value(RoutingRules::Priority, Expr::col(RoutingRules::Position))
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(Index::drop().name(ALIAS_INDEX).to_owned())
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(RoutingRules::Table)
                    .drop_column(RoutingRules::Position)
                    .to_owned(),
            )
            .await?;

        self.create_alias_index(manager, RoutingRules::Priority)
            .await
    }
}

impl Migration {
    async fn add_position(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(RoutingRules::Table)
                    .add_column(integer(RoutingRules::Position).default(0))
                    .to_owned(),
            )
            .await
    }

    async fn backfill_positions(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        manager
            .exec_stmt(
                Query::update()
                    .table(RoutingRules::Table)
                    .value(RoutingRules::Position, Expr::col(RoutingRules::Priority))
                    .to_owned(),
            )
            .await
    }

    async fn create_alias_index(
        &self,
        manager: &SchemaManager<'_>,
        order: RoutingRules,
    ) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name(ALIAS_INDEX)
                    .table(RoutingRules::Table)
                    .col(RoutingRules::Alias)
                    .col(RoutingRules::IsActive)
                    .col(order)
                    .if_not_exists()
                    .to_owned(),
            )
            .await
    }
}
