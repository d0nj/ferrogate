use sea_orm::{sea_query::OnConflict, EntityTrait, Set};
use sea_orm_migration::{prelude::*, schema::*};

use crate::entities::{guardrail_rule, model_pricing, provider};

const SEEDED_AT: &str = "2026-03-01T00:00:00Z";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        add_provider_columns(manager).await?;
        add_virtual_key_columns(manager).await?;
        add_request_log_columns(manager).await?;

        create_provider_keys(manager).await?;
        create_guardrail_rules(manager).await?;
        create_webhooks(manager).await?;
        create_prompt_templates(manager).await?;
        create_mcp_servers(manager).await?;
        create_semantic_cache(manager).await?;

        let db = manager.get_connection();

        model_pricing::Entity::insert_many(pricing_rows())
            .on_conflict(
                OnConflict::column(model_pricing::Column::Model)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(db)
            .await?;

        provider::Entity::insert(provider_template_row())
            .on_conflict(OnConflict::new().do_nothing().to_owned())
            .exec(db)
            .await?;

        guardrail_rule::Entity::insert_many(preset_guardrail_rows())
            .on_conflict(
                OnConflict::column(guardrail_rule::Column::Id)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(db)
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            SemanticCacheEntries::Table.into_iden(),
            McpServers::Table.into_iden(),
            PromptTemplates::Table.into_iden(),
            Webhooks::Table.into_iden(),
            GuardrailRules::Table.into_iden(),
            ProviderKeys::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).if_exists().to_owned())
                .await?;
        }
        Ok(())
    }
}

async fn add_column(
    manager: &SchemaManager<'_>,
    statement: sea_orm::sea_query::TableAlterStatement,
) -> Result<(), DbErr> {
    match manager.alter_table(statement).await {
        Ok(()) => Ok(()),
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

async fn add_provider_columns(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for column in [
        string_null(Providers::Region),
        ColumnDef::from(integer(Providers::Zdr).default(0)),
        ColumnDef::from(string(Providers::ExtraHeadersJson).default("{}")),
    ] {
        add_column(
            manager,
            Table::alter()
                .table(Providers::Table)
                .add_column(column)
                .to_owned(),
        )
        .await?;
    }
    Ok(())
}

async fn add_virtual_key_columns(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for column in [
        string_null(VirtualKeys::ExpiresAt),
        string_null(VirtualKeys::TeamName),
        ColumnDef::from(string(VirtualKeys::AllowedMcpJson).default("[]")),
        ColumnDef::from(string(VirtualKeys::TagsJson).default("{}")),
    ] {
        add_column(
            manager,
            Table::alter()
                .table(VirtualKeys::Table)
                .add_column(column)
                .to_owned(),
        )
        .await?;
    }
    Ok(())
}

async fn add_request_log_columns(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for column in [
        string_null(RequestLogs::RequestId),
        string_null(RequestLogs::SessionId),
        ColumnDef::from(string(RequestLogs::Endpoint).default("/v1/chat/completions")),
        double_null(RequestLogs::UpstreamTtfbMs),
        string_null(RequestLogs::GuardrailAction),
        string_null(RequestLogs::CacheType),
        string_null(RequestLogs::TagsJson),
    ] {
        add_column(
            manager,
            Table::alter()
                .table(RequestLogs::Table)
                .add_column(column)
                .to_owned(),
        )
        .await?;
    }
    Ok(())
}

async fn create_provider_keys(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(ProviderKeys::Table)
                .if_not_exists()
                .col(string(ProviderKeys::Id).primary_key())
                .col(string(ProviderKeys::ProviderId))
                .col(string(ProviderKeys::Name))
                .col(string(ProviderKeys::ApiKey))
                .col(integer(ProviderKeys::Weight).default(100))
                .col(string(ProviderKeys::ModelsJson).default("[]"))
                .col(integer_null(ProviderKeys::RpmLimit))
                .col(string_null(ProviderKeys::CooldownUntil))
                .col(string_null(ProviderKeys::LastError))
                .col(integer(ProviderKeys::IsActive).default(1))
                .col(string(ProviderKeys::CreatedAt))
                .col(string(ProviderKeys::UpdatedAt))
                .foreign_key(
                    ForeignKey::create()
                        .name("fk-provider_keys-provider_id")
                        .from(ProviderKeys::Table, ProviderKeys::ProviderId)
                        .to(Providers::Table, Providers::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_provider_keys_provider")
                .table(ProviderKeys::Table)
                .col(ProviderKeys::ProviderId)
                .col(ProviderKeys::IsActive)
                .if_not_exists()
                .to_owned(),
        )
        .await
}

async fn create_guardrail_rules(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(GuardrailRules::Table)
                .if_not_exists()
                .col(string(GuardrailRules::Id).primary_key())
                .col(string(GuardrailRules::Name))
                .col(string(GuardrailRules::Phase))
                .col(string(GuardrailRules::RuleType))
                .col(string_null(GuardrailRules::Pattern))
                .col(string(GuardrailRules::Action))
                .col(integer(GuardrailRules::Priority).default(0))
                .col(integer(GuardrailRules::IsActive).default(1))
                .col(string(GuardrailRules::CreatedAt))
                .col(string(GuardrailRules::UpdatedAt))
                .to_owned(),
        )
        .await
}

async fn create_webhooks(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Webhooks::Table)
                .if_not_exists()
                .col(string(Webhooks::Id).primary_key())
                .col(string(Webhooks::Name))
                .col(string(Webhooks::Url))
                .col(string_null(Webhooks::Secret))
                .col(string(Webhooks::EventsJson).default("[]"))
                .col(integer(Webhooks::IsActive).default(1))
                .col(integer_null(Webhooks::LastStatus))
                .col(string_null(Webhooks::LastError))
                .col(string(Webhooks::CreatedAt))
                .col(string(Webhooks::UpdatedAt))
                .to_owned(),
        )
        .await
}

async fn create_prompt_templates(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(PromptTemplates::Table)
                .if_not_exists()
                .col(string(PromptTemplates::Id).primary_key())
                .col(string(PromptTemplates::Slug))
                .col(integer(PromptTemplates::Version).default(1))
                .col(string(PromptTemplates::Name))
                .col(text(PromptTemplates::SystemPrompt).default(""))
                .col(text(PromptTemplates::UserTemplate).default(""))
                .col(string_null(PromptTemplates::Model))
                .col(double_null(PromptTemplates::Temperature))
                .col(integer_null(PromptTemplates::MaxTokens))
                .col(string(PromptTemplates::VariablesJson).default("[]"))
                .col(integer(PromptTemplates::IsActive).default(1))
                .col(string(PromptTemplates::CreatedAt))
                .col(string(PromptTemplates::UpdatedAt))
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_prompt_slug_version")
                .table(PromptTemplates::Table)
                .col(PromptTemplates::Slug)
                .col(PromptTemplates::Version)
                .unique()
                .if_not_exists()
                .to_owned(),
        )
        .await
}

async fn create_mcp_servers(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(McpServers::Table)
                .if_not_exists()
                .col(string(McpServers::Id).primary_key())
                .col(string_uniq(McpServers::Name))
                .col(string(McpServers::Transport))
                .col(string(McpServers::Url))
                .col(string(McpServers::HeadersJson).default("{}"))
                .col(string(McpServers::AllowedToolsJson).default("[]"))
                .col(text_null(McpServers::ToolsCacheJson))
                .col(string_null(McpServers::LastStatus))
                .col(integer(McpServers::IsActive).default(1))
                .col(string(McpServers::CreatedAt))
                .col(string(McpServers::UpdatedAt))
                .to_owned(),
        )
        .await
}

async fn create_semantic_cache(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(SemanticCacheEntries::Table)
                .if_not_exists()
                .col(string(SemanticCacheEntries::Id).primary_key())
                .col(string_uniq(SemanticCacheEntries::CacheKey))
                .col(string(SemanticCacheEntries::Model))
                .col(string_null(SemanticCacheEntries::ProviderName))
                .col(text(SemanticCacheEntries::PromptText))
                .col(text(SemanticCacheEntries::EmbeddingJson))
                .col(text(SemanticCacheEntries::ResponseJson))
                .col(integer(SemanticCacheEntries::Hits).default(0))
                .col(string(SemanticCacheEntries::CreatedAt))
                .col(string(SemanticCacheEntries::ExpiresAt))
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_semantic_cache_model")
                .table(SemanticCacheEntries::Table)
                .col(SemanticCacheEntries::Model)
                .col(SemanticCacheEntries::ExpiresAt)
                .if_not_exists()
                .to_owned(),
        )
        .await
}

fn pricing_rows() -> Vec<model_pricing::ActiveModel> {
    [
        ("jev-latest", "typesafe", 0.25, 0.0),
        ("jev-1.14.0", "typesafe", 0.25, 0.0),
        ("gpt-6-luna", "openai", 0.10, 0.0),
        ("gemini-decision-1", "gemini", 0.10, 0.0),
        ("text-embedding-3-small", "openai", 0.02, 0.0),
        ("text-embedding-3-large", "openai", 0.13, 0.0),
        ("gemini-embedding-001", "gemini", 0.15, 0.0),
    ]
    .into_iter()
    .map(
        |(model, provider_type, input_cost_per_1m, output_cost_per_1m)| {
            model_pricing::ActiveModel {
                model: Set(model.to_owned()),
                provider_type: Set(provider_type.to_owned()),
                input_cost_per_1m: Set(input_cost_per_1m),
                output_cost_per_1m: Set(output_cost_per_1m),
                updated_at: Set(SEEDED_AT.to_owned()),
            }
        },
    )
    .collect()
}

fn provider_template_row() -> provider::ActiveModel {
    provider::ActiveModel {
        id: Set("prov-typesafe".to_owned()),
        name: Set("TypeSafe (Jev)".to_owned()),
        provider_type: Set("typesafe".to_owned()),
        base_url: Set("https://api.typesafe.ai".to_owned()),
        api_key: Set(String::new()),
        models_json: Set(r#"["jev-latest","jev-1.14.0","jev-1.13.0","jev-preview"]"#.to_owned()),
        is_active: Set(0),
        timeout_ms: Set(30_000),
        created_at: Set(SEEDED_AT.to_owned()),
        updated_at: Set(SEEDED_AT.to_owned()),
        ..Default::default()
    }
}

fn preset_guardrail_rows() -> Vec<guardrail_rule::ActiveModel> {
    [
        (
            "guardrail-pii-masking",
            "PII Masking (Email / SSN / Credit Card / Phone)",
            "both",
            "pii",
            "mask",
            0,
        ),
        (
            "guardrail-secret-leak",
            "API Secret & Key Leak Guard",
            "both",
            "secret",
            "block",
            10,
        ),
        (
            "guardrail-injection",
            "Prompt Injection Heuristic Filter",
            "input",
            "injection",
            "block",
            20,
        ),
    ]
    .into_iter()
    .map(
        |(id, name, phase, rule_type, action, priority)| guardrail_rule::ActiveModel {
            id: Set(id.to_owned()),
            name: Set(name.to_owned()),
            phase: Set(phase.to_owned()),
            rule_type: Set(rule_type.to_owned()),
            pattern: Set(None),
            action: Set(action.to_owned()),
            priority: Set(priority),
            is_active: Set(0),
            created_at: Set(SEEDED_AT.to_owned()),
            updated_at: Set(SEEDED_AT.to_owned()),
        },
    )
    .collect()
}

#[derive(DeriveIden)]
enum Providers {
    Table,
    Id,
    Region,
    Zdr,
    ExtraHeadersJson,
}

#[derive(DeriveIden)]
enum VirtualKeys {
    Table,
    ExpiresAt,
    TeamName,
    AllowedMcpJson,
    TagsJson,
}

#[derive(DeriveIden)]
enum RequestLogs {
    Table,
    RequestId,
    SessionId,
    Endpoint,
    UpstreamTtfbMs,
    GuardrailAction,
    CacheType,
    TagsJson,
}

#[derive(DeriveIden)]
enum ProviderKeys {
    Table,
    Id,
    ProviderId,
    Name,
    ApiKey,
    Weight,
    ModelsJson,
    RpmLimit,
    CooldownUntil,
    LastError,
    IsActive,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum GuardrailRules {
    Table,
    Id,
    Name,
    Phase,
    RuleType,
    Pattern,
    Action,
    Priority,
    IsActive,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Webhooks {
    Table,
    Id,
    Name,
    Url,
    Secret,
    EventsJson,
    IsActive,
    LastStatus,
    LastError,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum PromptTemplates {
    Table,
    Id,
    Slug,
    Version,
    Name,
    SystemPrompt,
    UserTemplate,
    Model,
    Temperature,
    MaxTokens,
    VariablesJson,
    IsActive,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum McpServers {
    Table,
    Id,
    Name,
    Transport,
    Url,
    HeadersJson,
    AllowedToolsJson,
    ToolsCacheJson,
    LastStatus,
    IsActive,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum SemanticCacheEntries {
    Table,
    Id,
    CacheKey,
    Model,
    ProviderName,
    PromptText,
    EmbeddingJson,
    ResponseJson,
    Hits,
    CreatedAt,
    ExpiresAt,
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectOptions, ConnectionTrait, Database};
    use sea_orm_migration::MigratorTrait;

    fn ensure_test_key() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            std::env::set_var(
                "FERROGATE_ENCRYPTION_KEY",
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            );
        });
    }

    #[tokio::test]
    async fn resume_after_partial_add_column() {
        ensure_test_key();
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.expect("memory sqlite");
        crate::migration::Migrator::up(&db, Some(2))
            .await
            .expect("base migrations");
        db.execute_unprepared("ALTER TABLE providers ADD COLUMN region TEXT")
            .await
            .expect("simulate partial 000003");
        crate::migration::Migrator::up(&db, None)
            .await
            .expect("resume must tolerate the existing column");
    }
}
