use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "model_pricing")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub model: String,
    pub provider_type: String,
    pub input_cost_per_1m: f64,
    pub output_cost_per_1m: f64,
    pub updated_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
