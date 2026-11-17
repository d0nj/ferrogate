use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "semantic_cache_entries")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(unique)]
    pub cache_key: String,
    pub model: String,
    pub provider_name: Option<String>,
    pub virtual_key_id: Option<String>,
    pub prompt_text: String,
    pub embedding_json: String,
    pub response_json: String,
    pub hits: i64,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
