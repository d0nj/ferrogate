use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::{error::AppResult, AppState};

pub async fn get_policy(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let policy = crate::routing::read_policy(&state.db).await?;
    Ok(Json(json!({ "policy": policy })))
}

pub async fn put_policy(
    State(state): State<AppState>,
    Json(body): Json<crate::routing::RoutingPolicy>,
) -> AppResult<Json<Value>> {
    let stored = crate::routing::save_policy(&state.db, body).await?;
    state.routing_policy.set(stored.clone());
    state.routing.apply_policy(&stored).await;
    Ok(Json(json!({ "policy": stored })))
}
