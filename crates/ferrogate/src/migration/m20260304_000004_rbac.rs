use argon2::{
    password_hash::{rand_core::OsRng, SaltString},
    Argon2, PasswordHasher,
};
use sea_orm::{sea_query::OnConflict, DbErr, EntityTrait, Set};
use sea_orm_migration::{prelude::*, schema::*};

use crate::entities::user;

const SEEDED_AT: &str = "2026-03-04T00:00:00Z";

const BOOTSTRAP_EMAIL: &str = "admin@ferrogate.local";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_users(manager).await?;
        create_audit_log(manager).await?;
        seed_bootstrap_owner(manager.get_connection()).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [AuditLogs::Table.into_iden(), Users::Table.into_iden()] {
            manager
                .drop_table(Table::drop().table(table).if_exists().to_owned())
                .await?;
        }
        Ok(())
    }
}

async fn create_users(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Users::Table)
                .if_not_exists()
                .col(string(Users::Id).primary_key())
                .col(string(Users::Email))
                .col(string(Users::Name))
                .col(string(Users::PasswordHash))
                .col(string(Users::Role))
                .col(integer(Users::IsActive).default(1))
                .col(integer(Users::MustChangePassword).default(0))
                .col(string(Users::CreatedAt))
                .col(string_null(Users::LastLoginAt))
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_users_email")
                .table(Users::Table)
                .col(Users::Email)
                .unique()
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_users_role")
                .table(Users::Table)
                .col(Users::Role)
                .if_not_exists()
                .to_owned(),
        )
        .await
}

async fn create_audit_log(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(AuditLogs::Table)
                .if_not_exists()
                .col(string(AuditLogs::Id).primary_key())
                .col(string(AuditLogs::CreatedAt))
                .col(string_null(AuditLogs::ActorId))
                .col(string(AuditLogs::ActorLabel))
                .col(string(AuditLogs::Action))
                .col(string_null(AuditLogs::Target))
                .col(string(AuditLogs::Outcome))
                .col(string_null(AuditLogs::Detail))
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_audit_log_created_at")
                .table(AuditLogs::Table)
                .col((AuditLogs::CreatedAt, IndexOrder::Desc))
                .if_not_exists()
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_audit_log_action")
                .table(AuditLogs::Table)
                .col(AuditLogs::Action)
                .if_not_exists()
                .to_owned(),
        )
        .await
}

async fn seed_bootstrap_owner(db: &SchemaManagerConnection<'_>) -> Result<(), DbErr> {
    let already_populated = user::Entity::find().one(db).await?.is_some();
    if already_populated {
        return Ok(());
    }

    let raw: [u8; 16] = rand::Rng::random(&mut rand::rng());
    let password = format!("fg-{}", hex::encode(raw));
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .map_err(|err| DbErr::Custom(format!("bootstrap password hash: {err}")))?
        .to_string();
    eprintln!(
        "bootstrap owner {BOOTSTRAP_EMAIL} initial password: {password} (change it immediately)"
    );

    user::Entity::insert(user::ActiveModel {
        id: Set(format!("user-{}", uuid::Uuid::new_v4())),
        email: Set(BOOTSTRAP_EMAIL.to_owned()),
        name: Set("Admin".to_owned()),
        password_hash: Set(password_hash),
        role: Set("owner".to_owned()),
        is_active: Set(1),
        must_change_password: Set(1),
        created_at: Set(SEEDED_AT.to_owned()),
        last_login_at: Set(None),
    })
    .on_conflict(
        OnConflict::column(user::Column::Email)
            .do_nothing()
            .to_owned(),
    )
    .exec(db)
    .await?;

    Ok(())
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    Name,
    PasswordHash,
    Role,
    IsActive,
    MustChangePassword,
    CreatedAt,
    LastLoginAt,
}

#[derive(DeriveIden)]
enum AuditLogs {
    Table,
    Id,
    CreatedAt,
    ActorId,
    ActorLabel,
    Action,
    Target,
    Outcome,
    Detail,
}
