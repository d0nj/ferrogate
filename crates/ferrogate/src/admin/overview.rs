mod stats;

use axum::{
    extract::{Query, State},
    Json,
};
use sea_orm::{EntityTrait, QueryOrder};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{entities::provider, error::AppResult, AppState};

#[derive(Debug, Deserialize)]
pub struct OverviewQuery {
    #[serde(default)]
    pub window: Option<String>,
}

pub async fn overview(
    State(state): State<AppState>,
    Query(query): Query<OverviewQuery>,
) -> AppResult<Json<Value>> {
    let window = stats::Window::parse(query.window.as_deref());
    let rows = stats::fetch_aggregates(&state, window).await?;

    let providers = provider::Entity::find()
        .order_by_asc(provider::Column::Name)
        .all(&state.db)
        .await?;

    Ok(Json(json!({
        "window": window.label(),
        "kpis": stats::compute_kpis(&rows, window),
        "timeseries": stats::build_timeseries(&rows, window),
        "providers_health": stats::provider_health(&state, &providers),
    })))
}
