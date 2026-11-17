use axum::{extract::State, Json};
use serde::Serialize;

use crate::AppState;

#[derive(Debug, Serialize)]
pub struct CacheStatsDto {
    pub entries: u64,
    pub hits: u64,
    pub misses: u64,
    pub redis_connected: bool,
    pub redis_configured: bool,
}

pub async fn stats(State(state): State<AppState>) -> Json<CacheStatsDto> {
    let (hits, misses) = state.response_cache.stats();

    Json(CacheStatsDto {
        entries: state.response_cache.entry_count(),
        hits,
        misses,
        redis_connected: state.redis.is_some(),
        redis_configured: state.config.redis_url.is_some(),
    })
}

#[derive(Debug, Serialize)]
pub struct CacheClearDto {
    pub cleared: bool,
}

pub async fn clear(State(state): State<AppState>) -> Json<CacheClearDto> {
    state.response_cache.clear().await;
    Json(CacheClearDto { cleared: true })
}
