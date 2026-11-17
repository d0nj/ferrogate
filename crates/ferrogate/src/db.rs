use std::{str::FromStr, time::Duration};

use sea_orm::{DatabaseConnection, SqlxSqliteConnector};
use sea_orm_migration::MigratorTrait;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use crate::error::AppResult;

pub async fn init_db(database_url: &str) -> AppResult<DatabaseConnection> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(5_000));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                for pragma in [
                    "PRAGMA journal_mode = WAL;",
                    "PRAGMA synchronous = NORMAL;",
                    "PRAGMA foreign_keys = ON;",
                    "PRAGMA busy_timeout = 5000;",
                ] {
                    sqlx::query(pragma).execute(&mut *conn).await?;
                }
                Ok(())
            })
        })
        .connect_with(options)
        .await?;

    let db = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
    crate::migration::Migrator::up(&db, None).await?;

    Ok(db)
}
