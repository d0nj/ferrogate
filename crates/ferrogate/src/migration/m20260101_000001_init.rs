use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Providers::Table)
                    .if_not_exists()
                    .col(string(Providers::Id).primary_key())
                    .col(string_uniq(Providers::Name))
                    .col(string(Providers::ProviderType))
                    .col(string(Providers::BaseUrl))
                    .col(string(Providers::ApiKey).default(""))
                    .col(string(Providers::ModelsJson).default("[]"))
                    .col(integer(Providers::IsActive).default(1))
                    .col(integer(Providers::TimeoutMs).default(60_000))
                    .col(string(Providers::CreatedAt))
                    .col(string(Providers::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(RoutingRules::Table)
                    .if_not_exists()
                    .col(string(RoutingRules::Id).primary_key())
                    .col(string(RoutingRules::Alias))
                    .col(string(RoutingRules::ProviderId))
                    .col(string(RoutingRules::UpstreamModel))
                    .col(integer(RoutingRules::Weight).default(100))
                    .col(integer(RoutingRules::Priority).default(0))
                    .col(integer(RoutingRules::IsActive).default(1))
                    .col(string(RoutingRules::CreatedAt))
                    .col(string(RoutingRules::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-routing_rules-provider_id")
                            .from(RoutingRules::Table, RoutingRules::ProviderId)
                            .to(Providers::Table, Providers::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_routing_alias_active")
                    .table(RoutingRules::Table)
                    .col(RoutingRules::Alias)
                    .col(RoutingRules::IsActive)
                    .col(RoutingRules::Priority)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(VirtualKeys::Table)
                    .if_not_exists()
                    .col(string(VirtualKeys::Id).primary_key())
                    .col(string(VirtualKeys::Name))
                    .col(string(VirtualKeys::KeyPrefix))
                    .col(string_uniq(VirtualKeys::KeyHash))
                    .col(double_null(VirtualKeys::BudgetUsd))
                    .col(double(VirtualKeys::SpentUsd).default(0.0))
                    .col(integer_null(VirtualKeys::RpmLimit))
                    .col(integer_null(VirtualKeys::TpmLimit))
                    .col(string(VirtualKeys::AllowedModelsJson).default("[]"))
                    .col(integer(VirtualKeys::IsActive).default(1))
                    .col(string(VirtualKeys::CreatedAt))
                    .col(string_null(VirtualKeys::LastUsedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(ModelPricing::Table)
                    .if_not_exists()
                    .col(string(ModelPricing::Model).primary_key())
                    .col(string(ModelPricing::ProviderType))
                    .col(double(ModelPricing::InputCostPer1M))
                    .col(double(ModelPricing::OutputCostPer1M))
                    .col(string(ModelPricing::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(RequestLogs::Table)
                    .if_not_exists()
                    .col(string(RequestLogs::Id).primary_key())
                    .col(string(RequestLogs::CreatedAt))
                    .col(string_null(RequestLogs::VirtualKeyId))
                    .col(string_null(RequestLogs::VirtualKeyName))
                    .col(string(RequestLogs::RequestedModel))
                    .col(string_null(RequestLogs::RoutedProviderId))
                    .col(string_null(RequestLogs::RoutedProviderName))
                    .col(string_null(RequestLogs::RoutedModel))
                    .col(integer(RequestLogs::StatusCode))
                    .col(integer(RequestLogs::IsStreaming).default(0))
                    .col(integer(RequestLogs::InputTokens).default(0))
                    .col(integer(RequestLogs::OutputTokens).default(0))
                    .col(double(RequestLogs::CostUsd).default(0.0))
                    .col(double_null(RequestLogs::TtftMs))
                    .col(double(RequestLogs::LatencyMs).default(0.0))
                    .col(integer(RequestLogs::FallbackCount).default(0))
                    .col(integer(RequestLogs::CacheHit).default(0))
                    .col(string(RequestLogs::RoutingTraceJson).default("[]"))
                    .col(text_null(RequestLogs::RequestMessagesJson))
                    .col(text_null(RequestLogs::ResponseText))
                    .col(text_null(RequestLogs::ErrorMessage))
                    .to_owned(),
            )
            .await?;

        for (name, column) in [
            ("idx_logs_created_at", RequestLogs::CreatedAt),
            ("idx_logs_key", RequestLogs::VirtualKeyId),
            ("idx_logs_status", RequestLogs::StatusCode),
            ("idx_logs_model", RequestLogs::RequestedModel),
        ] {
            manager
                .create_index(
                    Index::create()
                        .name(name)
                        .table(RequestLogs::Table)
                        .col(column)
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
        }

        manager
            .create_table(
                Table::create()
                    .table(AppSettings::Table)
                    .if_not_exists()
                    .col(string(AppSettings::Key).primary_key())
                    .col(string(AppSettings::Value))
                    .col(string(AppSettings::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            AppSettings::Table.into_iden(),
            RequestLogs::Table.into_iden(),
            ModelPricing::Table.into_iden(),
            VirtualKeys::Table.into_iden(),
            RoutingRules::Table.into_iden(),
            Providers::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).if_exists().to_owned())
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Providers {
    Table,
    Id,
    Name,
    ProviderType,
    BaseUrl,
    ApiKey,
    ModelsJson,
    IsActive,
    TimeoutMs,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum RoutingRules {
    Table,
    Id,
    Alias,
    ProviderId,
    UpstreamModel,
    Weight,
    Priority,
    IsActive,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum VirtualKeys {
    Table,
    Id,
    Name,
    KeyPrefix,
    KeyHash,
    BudgetUsd,
    SpentUsd,
    RpmLimit,
    TpmLimit,
    AllowedModelsJson,
    IsActive,
    CreatedAt,
    LastUsedAt,
}

#[derive(DeriveIden)]
enum ModelPricing {
    Table,
    Model,
    ProviderType,
    #[sea_orm(iden = "input_cost_per_1m")]
    InputCostPer1M,
    #[sea_orm(iden = "output_cost_per_1m")]
    OutputCostPer1M,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum RequestLogs {
    Table,
    Id,
    CreatedAt,
    VirtualKeyId,
    VirtualKeyName,
    RequestedModel,
    RoutedProviderId,
    RoutedProviderName,
    RoutedModel,
    StatusCode,
    IsStreaming,
    InputTokens,
    OutputTokens,
    CostUsd,
    TtftMs,
    LatencyMs,
    FallbackCount,
    CacheHit,
    RoutingTraceJson,
    RequestMessagesJson,
    ResponseText,
    ErrorMessage,
}

#[derive(DeriveIden)]
enum AppSettings {
    Table,
    Key,
    Value,
    UpdatedAt,
}
