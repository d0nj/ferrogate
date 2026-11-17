use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "providers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(unique)]
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key: String,
    pub models_json: String,
    pub is_active: i64,
    pub timeout_ms: i64,
    pub created_at: String,
    pub updated_at: String,
    pub region: Option<String>,
    pub zdr: i64,
    pub extra_headers_json: String,
    pub api_dialect: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::routing_rule::Entity")]
    RoutingRules,
    #[sea_orm(has_many = "super::provider_key::Entity")]
    ProviderKeys,
}

impl Related<super::routing_rule::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::RoutingRules.def()
    }
}

impl Related<super::provider_key::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ProviderKeys.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
