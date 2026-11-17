use axum::{
    extract::{Path, Query, State},
    Json,
};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, ModelTrait, PaginatorTrait, QueryFilter, QueryOrder,
    Select,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    entities::request_log,
    error::{AppError, AppResult},
    AppState,
};

const DEFAULT_PAGE_SIZE: u64 = 25;
const MAX_PAGE_SIZE: u64 = 50;
const MAX_SEARCH_LEN: usize = 128;

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    #[serde(default)]
    pub page: Option<u64>,
    #[serde(default)]
    pub page_size: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub key_id: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RequestLogDto {
    pub id: String,
    pub created_at: String,
    pub virtual_key_id: Option<String>,
    pub virtual_key_name: Option<String>,
    pub requested_model: String,
    pub routed_provider_id: Option<String>,
    pub routed_provider_name: Option<String>,
    pub routed_model: Option<String>,
    pub status_code: i64,
    pub is_streaming: bool,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    pub ttft_ms: Option<f64>,
    pub latency_ms: f64,
    pub fallback_count: i64,
    pub cache_hit: bool,
    pub routing_trace: Value,
    pub request_messages: Option<Value>,
    pub response_text: Option<String>,
    pub error_message: Option<String>,
}

fn parse_json(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(Value::Null)
}

fn parse_optional_json(raw: &Option<String>) -> Option<Value> {
    raw.as_deref().map(parse_json)
}

impl From<request_log::Model> for RequestLogDto {
    fn from(row: request_log::Model) -> Self {
        Self {
            routing_trace: parse_json(&row.routing_trace_json),
            request_messages: parse_optional_json(&row.request_messages_json),
            is_streaming: row.is_streaming != 0,
            cache_hit: row.cache_hit != 0,
            id: row.id,
            created_at: row.created_at,
            virtual_key_id: row.virtual_key_id,
            virtual_key_name: row.virtual_key_name,
            requested_model: row.requested_model,
            routed_provider_id: row.routed_provider_id,
            routed_provider_name: row.routed_provider_name,
            routed_model: row.routed_model,
            status_code: row.status_code,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            cost_usd: row.cost_usd,
            ttft_ms: row.ttft_ms,
            latency_ms: row.latency_ms,
            fallback_count: row.fallback_count,
            response_text: row.response_text,
            error_message: row.error_message,
        }
    }
}

fn non_empty(raw: &Option<String>) -> Option<&str> {
    raw.as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn search_text(raw: &Option<String>) -> Option<String> {
    non_empty(raw).map(|value| value.chars().take(MAX_SEARCH_LEN).collect())
}

fn build_filter(query: &LogsQuery) -> AppResult<Condition> {
    let mut filter = Condition::all();

    if let Some(status) = non_empty(&query.status) {
        let code: i64 = status
            .parse()
            .map_err(|_| AppError::BadRequest(format!("invalid status filter '{status}'")))?;
        filter = filter.add(request_log::Column::StatusCode.eq(code));
    }

    if let Some(model) = non_empty(&query.model) {
        filter = filter.add(
            Condition::any()
                .add(request_log::Column::RequestedModel.eq(model.to_string()))
                .add(request_log::Column::RoutedModel.eq(model.to_string())),
        );
    }

    if let Some(key_id) = non_empty(&query.key_id) {
        filter = filter.add(request_log::Column::VirtualKeyId.eq(key_id.to_string()));
    }

    if let Some(search) = search_text(&query.search) {
        filter = filter.add(
            Condition::any()
                .add(request_log::Column::RequestedModel.contains(search.to_string()))
                .add(request_log::Column::RoutedModel.contains(search.to_string()))
                .add(request_log::Column::VirtualKeyName.contains(search.to_string()))
                .add(request_log::Column::ErrorMessage.contains(search.to_string()))
                .add(request_log::Column::Id.contains(search.to_string())),
        );
    }

    Ok(filter)
}

pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<LogsQuery>,
) -> AppResult<Json<Value>> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query
        .page_size
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);

    let base: Select<request_log::Entity> =
        request_log::Entity::find().filter(build_filter(&query)?);

    let total = base.clone().count(&state.db).await?;

    let rows = base
        .order_by_desc(request_log::Column::CreatedAt)
        .paginate(&state.db, page_size)
        .fetch_page(page - 1)
        .await?;

    let items: Vec<RequestLogDto> = rows.into_iter().map(RequestLogDto::from).collect();

    Ok(Json(json!({
        "items": items,
        "total": total,
        "page": page,
        "page_size": page_size,
    })))
}

pub async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<RequestLogDto>> {
    let row = request_log::Entity::find_by_id(id.clone())
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("request log '{id}' not found")))?;

    Ok(Json(RequestLogDto::from(row)))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let row = request_log::Entity::find_by_id(id.clone())
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("request log '{id}' not found")))?;
    row.delete(&state.db).await?;
    Ok(Json(json!({ "deleted": true, "id": id })))
}
