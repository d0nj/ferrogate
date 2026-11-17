use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "virtual_keys")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    #[sea_orm(unique)]
    pub key_hash: String,
    pub budget_usd: Option<f64>,
    pub spent_usd: f64,
    pub rpm_limit: Option<i64>,
    pub tpm_limit: Option<i64>,
    pub allowed_models_json: String,
    pub is_active: i64,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub team_name: Option<String>,
    pub allowed_mcp_json: String,
    pub tags_json: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
