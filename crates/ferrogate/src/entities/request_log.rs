use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "request_logs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub created_at: String,
    pub virtual_key_id: Option<String>,
    pub virtual_key_name: Option<String>,
    pub requested_model: String,
    pub routed_provider_id: Option<String>,
    pub routed_provider_name: Option<String>,
    pub routed_model: Option<String>,
    pub status_code: i64,
    pub is_streaming: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    pub ttft_ms: Option<f64>,
    pub latency_ms: f64,
    pub fallback_count: i64,
    pub cache_hit: i64,
    pub routing_trace_json: String,
    pub request_messages_json: Option<String>,
    pub response_text: Option<String>,
    pub error_message: Option<String>,
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub endpoint: String,
    pub upstream_ttfb_ms: Option<f64>,
    pub guardrail_action: Option<String>,
    pub cache_type: Option<String>,
    pub tags_json: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
