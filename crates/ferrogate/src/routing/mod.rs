mod affinity;
mod circuit;
mod ewma;
mod keys;
mod policy;
mod selector;

pub use affinity::{extract_session_id, SessionAffinity, SessionBinding, AFFINITY_TTL};
pub use circuit::{
    CircuitBreakerRegistry, CircuitState, ProviderHealthSnapshot, SharedCircuit,
    CIRCUIT_FAILURE_THRESHOLD, CIRCUIT_OPEN_DURATION,
};
pub use ewma::{EwmaRegistry, EWMA_ALPHA};
pub use keys::{ProviderKeyEntry, ProviderKeyPool, SelectedKey};
pub use policy::{
    read_policy, save_policy, AliasStrategy, FailureClass, PolicyStore, RetryClasses,
    RoutingPolicy, TierStrategy, TierStrategyOverride, POLICY_KEY,
};
pub use selector::{ModelSelector, RoutePreference};

pub mod cid {
    pub use super::affinity::extract_session_id;
}

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

use sea_orm::DatabaseConnection;

use crate::{
    entities::{app_setting, provider},
    error::{AppError, AppResult},
    providers::{ProviderEndpoint, ProviderKind},
    tokenizer::PricingRegistry,
};

pub const ROUTING_CACHE_TTL: Duration = Duration::from_secs(5);

pub const ROUTING_CACHE_CAPACITY: u64 = 512;

pub const ROUTE_FALLBACKS_KEY: &str = "route.fallbacks";

#[derive(Debug, Clone)]
pub struct RoutingTarget {
    pub provider_id: String,
    pub provider_name: String,
    pub kind: ProviderKind,
    pub dialect: crate::providers::ApiDialect,
    pub base_url: String,
    pub api_key: String,
    pub upstream_model: String,
    pub timeout_ms: u64,
    pub rule_id: Option<String>,
    pub weight: i64,
    pub position: i64,
    pub zdr: bool,
    pub region: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
    pub selected_key_id: Option<String>,
    pub selected_key_name: Option<String>,
}

impl RoutingTarget {
    pub fn endpoint(&self) -> ProviderEndpoint {
        ProviderEndpoint {
            id: self.provider_id.clone(),
            name: self.provider_name.clone(),
            kind: self.kind,
            dialect: self.dialect,
            base_url: self.base_url.clone(),
            api_key: self.api_key.clone(),
            upstream_model: self.upstream_model.clone(),
            timeout_ms: self.timeout_ms,
            extra_headers: self.extra_headers.clone(),
            zdr: self.zdr,
            region: self.region.clone(),
            selected_key_id: self.selected_key_id.clone(),
            selected_key_name: self.selected_key_name.clone(),
        }
    }
}

struct Provenance {
    rule_id: String,
    upstream_model: String,
    weight: i64,
    position: i64,
    provider: provider::Model,
}

pub struct RoutingService {
    db: DatabaseConnection,
    cache: Arc<ArcSwapCache>,
    fallback_cache: moka::future::Cache<(), Arc<BTreeMap<String, Vec<String>>>>,
    circuit: Arc<CircuitBreakerRegistry>,
    secrets: Arc<crate::crypto::SecretBox>,
    policy: Arc<PolicyStore>,
}

struct ArcSwapCache {
    slot: std::sync::RwLock<moka::future::Cache<String, Arc<Vec<RoutingTarget>>>>,
    ttl: std::sync::atomic::AtomicU64,
}

impl ArcSwapCache {
    fn new(ttl: Duration) -> Self {
        Self {
            slot: std::sync::RwLock::new(Self::build(ttl)),
            ttl: std::sync::atomic::AtomicU64::new(ttl.as_secs()),
        }
    }

    fn build(ttl: Duration) -> moka::future::Cache<String, Arc<Vec<RoutingTarget>>> {
        moka::future::Cache::builder()
            .time_to_live(ttl)
            .max_capacity(ROUTING_CACHE_CAPACITY)
            .build()
    }

    fn handle(&self) -> moka::future::Cache<String, Arc<Vec<RoutingTarget>>> {
        self.slot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn retune(&self, ttl: Duration) -> bool {
        use std::sync::atomic::Ordering;
        let seconds = ttl.as_secs();
        if self.ttl.load(Ordering::Relaxed) == seconds {
            return false;
        }
        self.ttl.store(seconds, Ordering::Relaxed);
        let mut slot = self
            .slot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = Self::build(ttl);
        true
    }

    async fn clear(&self) {
        let cache = self.handle();
        cache.invalidate_all();
        cache.run_pending_tasks().await;
    }
}

impl RoutingService {
    pub fn new(
        db: DatabaseConnection,
        circuit: Arc<CircuitBreakerRegistry>,
        secrets: Arc<crate::crypto::SecretBox>,
        policy: Arc<PolicyStore>,
    ) -> Self {
        let ttl = Duration::from_secs(policy.get().cache_ttl_seconds);
        let cache = Arc::new(ArcSwapCache::new(ttl));

        Self {
            db,
            cache,
            fallback_cache: moka::future::Cache::builder()
                .time_to_live(ttl)
                .max_capacity(1)
                .build(),
            circuit,
            secrets,
            policy,
        }
    }

    pub fn circuit(&self) -> &Arc<CircuitBreakerRegistry> {
        &self.circuit
    }

    pub fn policy(&self) -> Arc<RoutingPolicy> {
        self.policy.get()
    }

    pub async fn apply_policy(&self, policy: &RoutingPolicy) {
        let ttl = Duration::from_secs(policy.cache_ttl_seconds);
        if self.cache.retune(ttl) {
            self.cache.clear().await;
        }
        self.fallback_cache.invalidate_all();
        self.fallback_cache.run_pending_tasks().await;
        self.circuit.retune(policy);
        self.circuit.bump_generation();
    }

    pub async fn invalidate(&self) {
        self.cache.clear().await;
        self.fallback_cache.invalidate_all();
        self.fallback_cache.run_pending_tasks().await;
        self.circuit.bump_generation();
    }

    pub async fn fallback_aliases(&self) -> Arc<BTreeMap<String, Vec<String>>> {
        if let Some(cached) = self.fallback_cache.get(&()).await {
            return cached;
        }

        let loaded = match read_route_fallbacks(&self.db).await {
            Ok(chains) => Arc::new(chains),
            Err(error) => {
                tracing::warn!(%error, "failed to load route fallbacks; treating every alias as terminal");
                Arc::new(BTreeMap::new())
            }
        };

        self.fallback_cache.insert((), loaded.clone()).await;
        loaded
    }

    pub async fn resolve_targets(&self, alias: &str) -> AppResult<Vec<RoutingTarget>> {
        let selector = ModelSelector::parse(alias);

        if let Some(cached) = self.cache.handle().get(alias).await {
            return self
                .apply_selector((*cached).clone(), &selector)
                .ok_or_else(|| not_found(alias));
        }

        let targets = self.resolve_targets_inner(&selector).await?;
        self.cache
            .handle()
            .insert(alias.to_string(), Arc::new(targets.clone()))
            .await;

        self.apply_selector(targets, &selector)
            .ok_or_else(|| not_found(alias))
    }

    pub async fn resolve_ordered(
        &self,
        alias: &str,
        ewma: &EwmaRegistry,
        pricing: &PricingRegistry,
    ) -> AppResult<Vec<RoutingTarget>> {
        let selector = ModelSelector::parse(alias);
        let fallbacks = self.fallback_aliases().await;
        let targets = self.resolve_with_fallbacks(alias, &fallbacks).await?;
        Ok(self.order_targets(targets, &selector, ewma, pricing).await)
    }

    async fn resolve_with_fallbacks(
        &self,
        alias: &str,
        fallbacks: &BTreeMap<String, Vec<String>>,
    ) -> AppResult<Vec<RoutingTarget>> {
        if let Some(targets) = self.try_alias(alias).await? {
            return Ok(targets);
        }

        for name in fallback_chain(fallbacks, alias) {
            if let Some(targets) = self.try_alias(&name).await? {
                return Ok(targets);
            }
        }

        tracing::info!(%alias, "no routing target found, fallbacks exhausted");
        Err(not_found(alias))
    }

    async fn try_alias(&self, alias: &str) -> AppResult<Option<Vec<RoutingTarget>>> {
        match self.resolve_targets(alias).await {
            Ok(targets) => Ok(Some(targets)),
            Err(AppError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn order_targets(
        &self,
        mut targets: Vec<RoutingTarget>,
        selector: &ModelSelector,
        ewma: &EwmaRegistry,
        pricing: &PricingRegistry,
    ) -> Vec<RoutingTarget> {
        let policy = self.policy();
        let mut tiers: Vec<(i64, usize, usize)> = Vec::new();
        for (index, target) in targets.iter().enumerate() {
            match tiers.last_mut() {
                Some((position, _, end)) if *position == target.position => *end = index + 1,
                _ => tiers.push((target.position, index, index + 1)),
            }
        }

        for (position, start, end) in tiers {
            let strategy = selector.mode_strategy().unwrap_or_else(|| {
                policy
                    .strategy_for_alias(&selector.base_model)
                    .unwrap_or_else(|| policy.strategy_for(position))
            });
            let tier = &mut targets[start..end];
            match strategy {
                TierStrategy::Weighted | TierStrategy::Priority => {}
                TierStrategy::Latency => ewma.sort_span_by_latency(tier),
                TierStrategy::Cost => cost_sort(tier, pricing).await,
            }
        }

        targets
    }

    pub async fn resolve_targets_with_pricing(
        &self,
        alias: &str,
        pricing: &PricingRegistry,
    ) -> AppResult<Vec<RoutingTarget>> {
        self.resolve_ordered(alias, &EwmaRegistry::new(), pricing)
            .await
    }

    fn apply_selector(
        &self,
        targets: Vec<RoutingTarget>,
        selector: &ModelSelector,
    ) -> Option<Vec<RoutingTarget>> {
        let targets: Vec<RoutingTarget> = if selector.require_zdr {
            targets.into_iter().filter(|target| target.zdr).collect()
        } else {
            targets
        };

        if targets.is_empty() {
            return None;
        }

        Some(self.apply_health_ordering(targets))
    }

    fn apply_health_ordering(&self, targets: Vec<RoutingTarget>) -> Vec<RoutingTarget> {
        let (mut healthy, mut degraded): (Vec<_>, Vec<_>) = targets
            .into_iter()
            .partition(|target| self.circuit.is_closed(&target.provider_id));

        healthy.append(&mut degraded);
        healthy
    }

    pub async fn resolve_targets_unfiltered(&self, alias: &str) -> AppResult<Vec<RoutingTarget>> {
        let selector = ModelSelector::parse(alias);
        Ok(self.lookup_targets(&selector).await?.unwrap_or_default())
    }

    fn build_targets(&self, candidates: Vec<Provenance>) -> Vec<RoutingTarget> {
        let mut targets: Vec<RoutingTarget> = candidates
            .iter()
            .map(|candidate| self.target(candidate))
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect();
        targets.sort_by_key(|target| target.position);
        targets
    }

    fn target(&self, candidate: &Provenance) -> Option<RoutingTarget> {
        let provider = &candidate.provider;
        let Some(kind) = ProviderKind::parse(&provider.provider_type) else {
            tracing::warn!(
                provider_id = %provider.id,
                provider_type = %provider.provider_type,
                "skipping provider with unsupported type"
            );
            return None;
        };

        Some(RoutingTarget {
            dialect: crate::providers::ApiDialect::parse(&provider.api_dialect),
            zdr: provider.zdr != 0,
            region: provider.region.clone(),
            extra_headers: parse_extra_headers(&provider.extra_headers_json),
            provider_id: provider.id.clone(),
            provider_name: provider.name.clone(),
            kind,
            base_url: provider.base_url.clone(),
            api_key: self.secrets.open(&provider.api_key).unwrap_or_else(|err| {
                tracing::error!(
                    provider = %provider.id,
                    error = %err,
                    "stored credential could not be decrypted"
                );
                String::new()
            }),
            upstream_model: candidate.upstream_model.clone(),
            timeout_ms: provider.timeout_ms.max(0) as u64,
            rule_id: (!candidate.rule_id.is_empty()).then(|| candidate.rule_id.clone()),
            weight: candidate.weight,
            position: candidate.position,
            selected_key_id: None,
            selected_key_name: None,
        })
    }
}

mod listing;
mod resolve;
mod targets;

pub const MAX_FALLBACK_HOPS: usize = 4;

use targets::not_found;
pub use targets::parse_extra_headers;

fn fallback_chain(fallbacks: &BTreeMap<String, Vec<String>>, alias: &str) -> Vec<String> {
    let mut chain = Vec::new();
    let mut visited = BTreeSet::from([alias.to_string()]);
    collect_fallbacks(fallbacks, alias, &mut visited, &mut chain);
    chain
}

fn collect_fallbacks(
    fallbacks: &BTreeMap<String, Vec<String>>,
    alias: &str,
    visited: &mut BTreeSet<String>,
    chain: &mut Vec<String>,
) {
    for name in fallbacks.get(alias).into_iter().flatten() {
        if chain.len() >= MAX_FALLBACK_HOPS {
            return;
        }
        if !visited.insert(name.clone()) {
            continue;
        }
        chain.push(name.clone());
        collect_fallbacks(fallbacks, name, visited, chain);
    }
}

async fn cost_sort(tier: &mut [RoutingTarget], pricing: &PricingRegistry) {
    if tier.len() < 2 {
        return;
    }
    let mut costs = Vec::with_capacity(tier.len());
    for target in tier.iter() {
        let (input, output) = pricing.unit_costs(&target.upstream_model).await;
        let total = input + output;
        costs.push(if total > 0.0 { Some(total) } else { None });
    }
    let mut order: Vec<usize> = (0..tier.len()).collect();
    order.sort_by(|&left, &right| match (costs[left], costs[right]) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    let original: Vec<RoutingTarget> = tier.to_vec();
    for (slot, index) in order.into_iter().enumerate() {
        tier[slot] = original[index].clone();
    }
}

pub fn parse_models_json(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

pub async fn read_route_fallbacks<C: sea_orm::ConnectionTrait>(
    db: &C,
) -> AppResult<BTreeMap<String, Vec<String>>> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let row = app_setting::Entity::find()
        .filter(app_setting::Column::Key.eq(ROUTE_FALLBACKS_KEY))
        .one(db)
        .await?;
    Ok(row
        .and_then(|row| serde_json::from_str::<BTreeMap<String, Vec<String>>>(&row.value).ok())
        .unwrap_or_default())
}

pub async fn write_route_fallbacks<C: sea_orm::ConnectionTrait>(
    db: &C,
    chains: BTreeMap<String, Vec<String>>,
) -> AppResult<()> {
    use sea_orm::{sea_query::OnConflict, EntityTrait, Set};

    if chains.is_empty() {
        app_setting::Entity::delete_by_id(ROUTE_FALLBACKS_KEY)
            .exec(db)
            .await?;
        return Ok(());
    }

    let value = serde_json::to_string(&chains)
        .map_err(|err| AppError::Internal(format!("encode route fallbacks: {err}")))?;
    let row = app_setting::ActiveModel {
        key: Set(ROUTE_FALLBACKS_KEY.to_string()),
        value: Set(value),
        updated_at: Set(crate::admin::now_rfc3339()),
    };
    app_setting::Entity::insert(row)
        .on_conflict(
            OnConflict::column(app_setting::Column::Key)
                .update_columns([app_setting::Column::Value, app_setting::Column::UpdatedAt])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

pub async fn save_route_fallback<C: sea_orm::ConnectionTrait>(
    db: &C,
    alias: &str,
    chain: &[String],
) -> AppResult<()> {
    let mut chains = read_route_fallbacks(db).await?;
    if chain.is_empty() {
        chains.remove(alias);
    } else {
        chains.insert(alias.to_string(), chain.to_vec());
    }
    write_route_fallbacks(db, chains).await
}

pub async fn remove_route_fallback<C: sea_orm::ConnectionTrait>(
    db: &C,
    alias: &str,
) -> AppResult<()> {
    save_route_fallback(db, alias, &[]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chains(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(alias, names)| {
                (
                    (*alias).to_string(),
                    names.iter().map(|name| (*name).to_string()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn direct_fallbacks_come_back_in_declared_order() {
        let map = chains(&[("smart", &["fast", "cheap"])]);
        assert_eq!(fallback_chain(&map, "smart"), vec!["fast", "cheap"]);
    }

    #[test]
    fn fallback_walk_is_depth_first() {
        let map = chains(&[("a", &["b", "c"]), ("b", &["d"])]);
        assert_eq!(fallback_chain(&map, "a"), vec!["b", "d", "c"]);
    }

    #[test]
    fn alias_without_fallbacks_yields_nothing() {
        let map = chains(&[("other", &["fast"])]);
        assert!(fallback_chain(&map, "smart").is_empty());
    }

    #[test]
    fn cycles_are_broken_instead_of_looping() {
        let map = chains(&[("a", &["b"]), ("b", &["a"]), ("c", &["c"])]);
        assert_eq!(fallback_chain(&map, "a"), vec!["b"]);
        assert!(fallback_chain(&map, "c").is_empty());
    }

    #[test]
    fn shared_fallback_is_attempted_once() {
        let map = chains(&[("a", &["b", "c"]), ("b", &["d"]), ("c", &["d"])]);
        assert_eq!(fallback_chain(&map, "a"), vec!["b", "d", "c"]);
    }

    #[test]
    fn walk_stops_at_the_hop_limit() {
        let map = chains(&[
            ("a", &["b"]),
            ("b", &["c"]),
            ("c", &["d"]),
            ("d", &["e"]),
            ("e", &["f"]),
        ]);
        let chain = fallback_chain(&map, "a");
        assert_eq!(chain, vec!["b", "c", "d", "e"]);
        assert_eq!(chain.len(), MAX_FALLBACK_HOPS);
    }
}
