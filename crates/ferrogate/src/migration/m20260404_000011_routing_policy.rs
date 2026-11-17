use sea_orm::{EntityTrait, Set};
use sea_orm_migration::prelude::*;

use crate::entities::app_setting;
use crate::routing::{RoutingPolicy, POLICY_KEY};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let value = serde_json::to_string(&RoutingPolicy::default())
            .map_err(|err| DbErr::Custom(format!("encode routing policy: {err}")))?;
        let now = chrono::Utc::now().to_rfc3339();

        app_setting::Entity::insert(app_setting::ActiveModel {
            key: Set(POLICY_KEY.to_string()),
            value: Set(value),
            updated_at: Set(now),
        })
        .on_conflict(
            sea_orm::sea_query::OnConflict::column(app_setting::Column::Key)
                .do_nothing()
                .to_owned(),
        )
        .exec(db)
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        app_setting::Entity::delete_by_id(POLICY_KEY)
            .exec(db)
            .await?;
        Ok(())
    }
}
