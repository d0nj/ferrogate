use std::collections::{BTreeMap, BTreeSet};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    entities::{provider, routing_rule},
    error::{AppError, AppResult},
    routing::{AliasStrategy, RoutingPolicy, TierStrategy},
    AppState,
};

use super::now_rfc3339;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteTargetDto {
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
    pub upstream_model: String,
    #[serde(default = "default_weight")]
    pub weight: i64,
}

impl RouteTargetDto {
    fn build(rule: routing_rule::Model, provider: Option<provider::Model>) -> Self {
        Self {
            provider_id: rule.provider_id,
            provider_name: provider.map(|row| row.name),
            upstream_model: rule.upstream_model,
            weight: rule.weight,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouteDto {
    #[serde(default)]
    pub alias: String,
    #[serde(default)]
    pub strategy: Option<TierStrategy>,
    #[serde(default)]
    pub fallbacks: Vec<String>,
    pub targets: Vec<RouteTargetDto>,
}

fn default_weight() -> i64 {
    100
}

fn require_text(value: &str, field: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(format!("{field} must not be empty")));
    }
    Ok(trimmed.to_string())
}

fn route_not_found(alias: &str) -> AppError {
    AppError::NotFound(format!("route '{alias}' not found"))
}

async fn load_targets(state: &AppState) -> AppResult<BTreeMap<String, Vec<RouteTargetDto>>> {
    let rows = routing_rule::Entity::find()
        .find_also_related(provider::Entity)
        .order_by_asc(routing_rule::Column::Alias)
        .order_by_asc(routing_rule::Column::Position)
        .all(&state.db)
        .await?;

    let mut grouped: BTreeMap<String, Vec<RouteTargetDto>> = BTreeMap::new();
    for (rule, provider) in rows {
        grouped
            .entry(rule.alias.clone())
            .or_default()
            .push(RouteTargetDto::build(rule, provider));
    }
    Ok(grouped)
}

async fn load_alias_targets(state: &AppState, alias: &str) -> AppResult<Vec<RouteTargetDto>> {
    Ok(routing_rule::Entity::find()
        .find_also_related(provider::Entity)
        .filter(routing_rule::Column::Alias.eq(alias))
        .order_by_asc(routing_rule::Column::Position)
        .all(&state.db)
        .await?
        .into_iter()
        .map(|(rule, provider)| RouteTargetDto::build(rule, provider))
        .collect())
}

async fn load_fallbacks(state: &AppState) -> AppResult<BTreeMap<String, Vec<String>>> {
    crate::routing::read_route_fallbacks(&state.db).await
}

async fn load_strategies(state: &AppState) -> AppResult<BTreeMap<String, TierStrategy>> {
    let policy: RoutingPolicy = crate::routing::read_policy(&state.db).await?;
    Ok(policy
        .alias_strategies
        .iter()
        .map(|entry| (entry.alias.clone(), entry.strategy))
        .collect())
}

fn build_dto(
    alias: String,
    targets: Vec<RouteTargetDto>,
    fallbacks: &BTreeMap<String, Vec<String>>,
    strategies: &BTreeMap<String, TierStrategy>,
) -> RouteDto {
    let strategy = strategies.get(&alias).copied();
    let fallbacks = fallbacks.get(&alias).cloned().unwrap_or_default();
    RouteDto {
        alias,
        strategy,
        fallbacks,
        targets,
    }
}

async fn load_dto(state: &AppState, alias: &str) -> AppResult<RouteDto> {
    let targets = load_alias_targets(state, alias).await?;
    if targets.is_empty() {
        return Err(route_not_found(alias));
    }

    Ok(build_dto(
        alias.to_string(),
        targets,
        &load_fallbacks(state).await?,
        &load_strategies(state).await?,
    ))
}

async fn store_strategy(
    state: &AppState,
    alias: &str,
    strategy: Option<TierStrategy>,
) -> AppResult<()> {
    let mut policy: RoutingPolicy = crate::routing::read_policy(&state.db).await?;
    let previous = policy.alias_strategies.clone();
    policy.alias_strategies.retain(|entry| entry.alias != alias);
    if let Some(strategy) = strategy {
        policy.alias_strategies.push(AliasStrategy {
            alias: alias.to_string(),
            strategy,
        });
    }
    if policy.alias_strategies == previous {
        return Ok(());
    }

    let stored = crate::routing::save_policy(&state.db, policy).await?;
    state.routing_policy.set(stored.clone());
    state.routing.apply_policy(&stored).await;
    Ok(())
}

async fn validate_targets(state: &AppState, targets: &[RouteTargetDto]) -> AppResult<()> {
    if targets.is_empty() {
        return Err(AppError::BadRequest(
            "a route needs at least one target".to_string(),
        ));
    }
    let mut seen: BTreeSet<(&str, &str)> = BTreeSet::new();
    for (index, target) in targets.iter().enumerate() {
        if target.provider_id.trim().is_empty() {
            return Err(AppError::BadRequest(format!(
                "target {index}: provider_id must not be empty"
            )));
        }
        if target.upstream_model.trim().is_empty() {
            return Err(AppError::BadRequest(format!(
                "target {index}: upstream_model must not be empty"
            )));
        }
        if target.weight < 1 {
            return Err(AppError::BadRequest(format!(
                "target {index}: weight must be at least 1"
            )));
        }
        if !seen.insert((&target.provider_id, &target.upstream_model)) {
            return Err(AppError::BadRequest(format!(
                "target {index}: duplicate of {} / {}",
                target.provider_id, target.upstream_model
            )));
        }
    }

    let wanted: BTreeSet<String> = targets.iter().map(|t| t.provider_id.clone()).collect();
    let known: BTreeSet<String> = provider::Entity::find()
        .filter(provider::Column::Id.is_in(wanted.iter().cloned()))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect();

    match wanted.difference(&known).next() {
        Some(provider_id) => Err(AppError::BadRequest(format!(
            "provider '{provider_id}' does not exist"
        ))),
        None => Ok(()),
    }
}

fn validate_fallbacks(alias: &str, fallbacks: &[String]) -> AppResult<()> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for name in fallbacks {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::BadRequest(
                "fallback alias must not be empty".to_string(),
            ));
        }
        if name == alias {
            return Err(AppError::BadRequest(format!(
                "route '{alias}' cannot fall back to itself"
            )));
        }
        if !seen.insert(name) {
            return Err(AppError::BadRequest(format!(
                "fallback alias '{name}' is listed twice"
            )));
        }
    }
    Ok(())
}

pub async fn list(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let targets = load_targets(&state).await?;
    let fallbacks = load_fallbacks(&state).await?;
    let strategies = load_strategies(&state).await?;

    let mut aliases: BTreeSet<String> = targets.keys().cloned().collect();
    aliases.extend(fallbacks.keys().cloned());

    let routes: Vec<RouteDto> = aliases
        .into_iter()
        .map(|alias| {
            let targets = targets.get(&alias).cloned().unwrap_or_default();
            build_dto(alias, targets, &fallbacks, &strategies)
        })
        .collect();

    Ok(Json(json!({ "routes": routes })))
}

pub async fn get(
    State(state): State<AppState>,
    Path(alias): Path<String>,
) -> AppResult<Json<RouteDto>> {
    Ok(Json(load_dto(&state, &alias).await?))
}

pub async fn put(
    State(state): State<AppState>,
    Path(alias): Path<String>,
    Json(body): Json<RouteDto>,
) -> AppResult<Json<RouteDto>> {
    let alias = require_text(&alias, "alias")?;
    validate_targets(&state, &body.targets).await?;
    validate_fallbacks(&alias, &body.fallbacks)?;

    let now = now_rfc3339();
    let tx = state.db.begin().await?;

    routing_rule::Entity::delete_many()
        .filter(routing_rule::Column::Alias.eq(&alias))
        .exec(&tx)
        .await?;

    for (position, target) in body.targets.iter().enumerate() {
        routing_rule::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            alias: Set(alias.clone()),
            provider_id: Set(target.provider_id.clone()),
            upstream_model: Set(target.upstream_model.clone()),
            weight: Set(target.weight),
            position: Set(position as i64),
            is_active: Set(1),
            created_at: Set(now.clone()),
            updated_at: Set(now.clone()),
        }
        .insert(&tx)
        .await?;
    }

    crate::routing::save_route_fallback(&tx, &alias, &body.fallbacks).await?;

    tx.commit().await?;

    store_strategy(&state, &alias, body.strategy).await?;

    state.routing.invalidate().await;
    Ok(Json(load_dto(&state, &alias).await?))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(alias): Path<String>,
) -> AppResult<StatusCode> {
    let tx = state.db.begin().await?;

    routing_rule::Entity::delete_many()
        .filter(routing_rule::Column::Alias.eq(&alias))
        .exec(&tx)
        .await?;

    let fallbacks = crate::routing::read_route_fallbacks(&tx).await?;
    crate::routing::remove_route_fallback(&tx, &alias).await?;
    for (name, chain) in &fallbacks {
        if name == &alias || !chain.iter().any(|step| step == &alias) {
            continue;
        }
        let pruned: Vec<String> = chain
            .iter()
            .filter(|step| *step != &alias)
            .cloned()
            .collect();
        crate::routing::save_route_fallback(&tx, name, &pruned).await?;
    }

    tx.commit().await?;

    store_strategy(&state, &alias, None).await?;

    state.routing.invalidate().await;
    Ok(StatusCode::NO_CONTENT)
}
