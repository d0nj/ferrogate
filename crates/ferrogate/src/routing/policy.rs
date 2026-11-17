use std::sync::{Arc, RwLock};

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};

use crate::{
    entities::app_setting,
    error::{AppError, AppResult},
};

pub const POLICY_KEY: &str = "routing.policy";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TierStrategy {
    #[default]
    Weighted,
    Latency,
    Cost,
    Priority,
}

impl TierStrategy {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "latency" => Self::Latency,
            "cost" => Self::Cost,
            "priority" => Self::Priority,
            _ => Self::Weighted,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Weighted => "weighted",
            Self::Latency => "latency",
            Self::Cost => "cost",
            Self::Priority => "priority",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    RateLimit,
    ServerError,
    Transport,
    Protocol,
    ClientError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetryClasses {
    pub rate_limit: bool,
    pub server_error: bool,
    pub transport: bool,
    pub protocol: bool,
}

impl Default for RetryClasses {
    fn default() -> Self {
        Self {
            rate_limit: true,
            server_error: true,
            transport: true,
            protocol: false,
        }
    }
}

impl RetryClasses {
    pub fn allows(&self, class: FailureClass) -> bool {
        match class {
            FailureClass::RateLimit => self.rate_limit,
            FailureClass::ServerError => self.server_error,
            FailureClass::Transport => self.transport,
            FailureClass::Protocol => self.protocol,
            FailureClass::ClientError => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierStrategyOverride {
    pub position: i64,
    pub strategy: TierStrategy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasStrategy {
    pub alias: String,
    pub strategy: TierStrategy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingPolicy {
    pub failure_threshold: u32,
    pub cooldown_seconds: u64,
    pub cache_ttl_seconds: u64,
    pub key_rotations: u32,
    pub strategy: TierStrategy,
    pub tier_strategies: Vec<TierStrategyOverride>,
    pub alias_strategies: Vec<AliasStrategy>,
    pub retry_on: RetryClasses,
    pub max_attempts: u32,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            cooldown_seconds: 15,
            cache_ttl_seconds: 5,
            key_rotations: 2,
            strategy: TierStrategy::Weighted,
            tier_strategies: Vec::new(),
            alias_strategies: Vec::new(),
            retry_on: RetryClasses::default(),
            max_attempts: 0,
        }
    }
}

impl RoutingPolicy {
    pub fn strategy_for(&self, position: i64) -> TierStrategy {
        self.tier_strategies
            .iter()
            .find(|entry| entry.position == position)
            .map_or(self.strategy, |entry| entry.strategy)
    }

    pub fn strategy_for_alias(&self, alias: &str) -> Option<TierStrategy> {
        self.alias_strategies
            .iter()
            .find(|entry| entry.alias == alias)
            .map(|entry| entry.strategy)
    }

    pub fn allows_attempt(&self, failed: u32) -> bool {
        self.max_attempts == 0 || failed < self.max_attempts
    }

    pub fn sanitised(mut self) -> Self {
        self.failure_threshold = self.failure_threshold.clamp(1, 100);
        self.cooldown_seconds = self.cooldown_seconds.clamp(1, 3_600);
        self.cache_ttl_seconds = self.cache_ttl_seconds.clamp(1, 300);
        self.key_rotations = self.key_rotations.min(10);
        self.max_attempts = self.max_attempts.min(50);
        let mut seen: Vec<i64> = Vec::new();
        self.tier_strategies.retain(|entry| {
            if seen.contains(&entry.position) {
                false
            } else {
                seen.push(entry.position);
                true
            }
        });
        let mut aliases: Vec<String> = Vec::new();
        self.alias_strategies.retain(|entry| {
            if aliases.contains(&entry.alias) {
                false
            } else {
                aliases.push(entry.alias.clone());
                true
            }
        });
        self
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.failure_threshold == 0 {
            return Err(AppError::BadRequest(
                "failure_threshold must be at least 1".into(),
            ));
        }
        if self.cache_ttl_seconds == 0 {
            return Err(AppError::BadRequest(
                "cache_ttl_seconds must be at least 1".into(),
            ));
        }
        if self.cooldown_seconds == 0 {
            return Err(AppError::BadRequest(
                "cooldown_seconds must be at least 1".into(),
            ));
        }
        if self
            .tier_strategies
            .iter()
            .enumerate()
            .any(|(index, entry)| {
                self.tier_strategies[..index]
                    .iter()
                    .any(|other| other.position == entry.position)
            })
        {
            return Err(AppError::BadRequest(
                "tier_strategies must not repeat a position".into(),
            ));
        }
        if self
            .alias_strategies
            .iter()
            .enumerate()
            .any(|(index, entry)| {
                self.alias_strategies[..index]
                    .iter()
                    .any(|other| other.alias == entry.alias)
            })
        {
            return Err(AppError::BadRequest(
                "alias_strategies must not repeat an alias".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct PolicyStore {
    current: RwLock<Arc<RoutingPolicy>>,
}

impl PolicyStore {
    pub fn new(policy: RoutingPolicy) -> Self {
        Self {
            current: RwLock::new(Arc::new(policy.sanitised())),
        }
    }

    pub fn get(&self) -> Arc<RoutingPolicy> {
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn set(&self, policy: RoutingPolicy) {
        let mut slot = self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = Arc::new(policy.sanitised());
    }

    pub async fn load(&self, db: &sea_orm::DatabaseConnection) {
        let row = app_setting::Entity::find_by_id(POLICY_KEY)
            .one(db)
            .await
            .ok()
            .flatten();
        let Some(row) = row else {
            return;
        };
        match serde_json::from_str::<RoutingPolicy>(&row.value) {
            Ok(policy) => self.set(policy),
            Err(err) => tracing::error!(
                error = %err,
                "stored routing policy is malformed; keeping the active policy"
            ),
        }
    }
}

pub async fn read_policy(db: &sea_orm::DatabaseConnection) -> AppResult<RoutingPolicy> {
    let row = app_setting::Entity::find()
        .filter(app_setting::Column::Key.eq(POLICY_KEY))
        .one(db)
        .await?;
    Ok(row
        .and_then(|row| serde_json::from_str::<RoutingPolicy>(&row.value).ok())
        .unwrap_or_default())
}

pub async fn save_policy(
    db: &sea_orm::DatabaseConnection,
    policy: RoutingPolicy,
) -> AppResult<RoutingPolicy> {
    policy.validate()?;
    let policy = policy.sanitised();
    let value = serde_json::to_string(&policy)
        .map_err(|err| AppError::Internal(format!("encode routing policy: {err}")))?;
    let now = crate::admin::now_rfc3339();

    match app_setting::Entity::find_by_id(POLICY_KEY).one(db).await? {
        Some(row) => {
            let mut active: app_setting::ActiveModel = row.into();
            active.value = Set(value);
            active.updated_at = Set(now);
            app_setting::Entity::update(active).exec(db).await?;
        }
        None => {
            app_setting::ActiveModel {
                key: Set(POLICY_KEY.to_string()),
                value: Set(value),
                updated_at: Set(now),
            }
            .insert(db)
            .await?;
        }
    }

    Ok(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_stable() {
        let policy = RoutingPolicy::default();
        assert_eq!(policy.failure_threshold, 3);
        assert_eq!(policy.cooldown_seconds, 15);
        assert_eq!(policy.cache_ttl_seconds, 5);
        assert_eq!(policy.strategy, TierStrategy::Weighted);
        assert!(policy.retry_on.rate_limit);
        assert!(!policy.retry_on.protocol);
    }

    #[test]
    fn tier_override_wins_over_the_default_strategy() {
        let mut policy = RoutingPolicy::default();
        policy.tier_strategies.push(TierStrategyOverride {
            position: 2,
            strategy: TierStrategy::Cost,
        });
        assert_eq!(policy.strategy_for(0), TierStrategy::Weighted);
        assert_eq!(policy.strategy_for(2), TierStrategy::Cost);
    }

    #[test]
    fn sanitise_clamps_hostile_values() {
        let policy = RoutingPolicy {
            failure_threshold: 0,
            cooldown_seconds: 999_999,
            cache_ttl_seconds: 0,
            key_rotations: 99,
            max_attempts: 9_999,
            ..RoutingPolicy::default()
        }
        .sanitised();
        assert_eq!(policy.failure_threshold, 1);
        assert_eq!(policy.cooldown_seconds, 3_600);
        assert_eq!(policy.cache_ttl_seconds, 1);
        assert_eq!(policy.key_rotations, 10);
        assert_eq!(policy.max_attempts, 50);
    }

    #[test]
    fn duplicate_tier_overrides_are_rejected_then_dropped() {
        let policy = RoutingPolicy {
            tier_strategies: vec![
                TierStrategyOverride {
                    position: 1,
                    strategy: TierStrategy::Cost,
                },
                TierStrategyOverride {
                    position: 1,
                    strategy: TierStrategy::Latency,
                },
            ],
            ..RoutingPolicy::default()
        };
        assert!(policy.validate().is_err());
        assert_eq!(policy.sanitised().tier_strategies.len(), 1);
    }

    #[test]
    fn zero_thresholds_fail_validation() {
        for policy in [
            RoutingPolicy {
                failure_threshold: 0,
                ..RoutingPolicy::default()
            },
            RoutingPolicy {
                cache_ttl_seconds: 0,
                ..RoutingPolicy::default()
            },
            RoutingPolicy {
                cooldown_seconds: 0,
                ..RoutingPolicy::default()
            },
        ] {
            assert!(policy.validate().is_err());
        }
    }

    #[test]
    fn unknown_strategy_names_fall_back_to_weighted() {
        assert_eq!(TierStrategy::parse("COST"), TierStrategy::Cost);
        assert_eq!(TierStrategy::parse("  latency "), TierStrategy::Latency);
        assert_eq!(TierStrategy::parse("nonsense"), TierStrategy::Weighted);
    }

    #[test]
    fn store_round_trips_a_policy() {
        let store = PolicyStore::new(RoutingPolicy::default());
        store.set(RoutingPolicy {
            failure_threshold: 7,
            ..RoutingPolicy::default()
        });
        assert_eq!(store.get().failure_threshold, 7);
    }
}
