mod dual;
pub mod limiter;
mod middleware;
pub mod password;
pub mod role;
pub mod session;
pub mod throttle;
mod window;

pub use dual::RATE_LIMIT_WINDOW_SECS;
pub use middleware::{credential, require_virtual_key};

use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use moka::future::Cache;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use sha2::{Digest, Sha256};
use tracing::debug;

use crate::{
    entities::virtual_key,
    error::{AppError, AppResult},
    infra::redis::RedisStore,
    proxy::AuthenticatedKey,
    routing::ModelSelector,
    telemetry::{live_spend_micros, set_or_reset_live_spend},
};

use self::{dual::SharedLimiter, limiter::KeyLimiterRegistry};

const KEY_CACHE_TTL: Duration = Duration::from_secs(5);
const KEY_CACHE_CAPACITY: u64 = 4_096;

pub fn sha256_hex(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

#[derive(Clone, Debug)]
pub struct CachedKey {
    pub key: AuthenticatedKey,
    pub spent_usd: f64,
    pub expires_at: Option<String>,
}

impl CachedKey {
    fn is_expired(&self) -> bool {
        let Some(raw) = self.expires_at.as_deref() else {
            return false;
        };

        DateTime::parse_from_rfc3339(raw)
            .map(|expires_at| expires_at.to_utc() <= Utc::now())
            .unwrap_or(false)
    }
}

#[derive(Clone)]
pub struct AuthService {
    db: DatabaseConnection,
    cache: Cache<String, Option<CachedKey>>,
    live_spend: Arc<DashMap<String, Arc<AtomicI64>>>,
    pub limiters: Arc<KeyLimiterRegistry>,
    shared: Arc<SharedLimiter>,
}

impl AuthService {
    pub fn new(db: DatabaseConnection, live_spend: Arc<DashMap<String, Arc<AtomicI64>>>) -> Self {
        Self {
            db,
            cache: Cache::builder()
                .time_to_live(KEY_CACHE_TTL)
                .max_capacity(KEY_CACHE_CAPACITY)
                .build(),
            live_spend,
            limiters: Arc::new(KeyLimiterRegistry::new()),
            shared: Arc::new(SharedLimiter::new(None)),
        }
    }

    pub fn with_redis(mut self, redis: Option<Arc<RedisStore>>) -> Self {
        self.shared = Arc::new(SharedLimiter::new(redis));
        self
    }

    pub async fn reconcile_tpm(&self, key: &AuthenticatedKey, output_tokens: u32) {
        self.limiters
            .reconcile_tpm(&key.id, key.tpm_limit, output_tokens);
        self.shared.reconcile_tpm(key, output_tokens).await;
    }

    pub async fn admit_rpm(&self, key: &AuthenticatedKey) -> AppResult<()> {
        self.limiters.check_rpm(&key.id, key.rpm_limit)?;
        self.shared.admit_rpm(key).await
    }

    pub async fn check_tpm(&self, key: &AuthenticatedKey, estimated_tokens: u32) -> AppResult<()> {
        self.limiters
            .check_tpm(&key.id, key.tpm_limit, estimated_tokens)?;
        self.shared.consume_tpm(key, estimated_tokens).await
    }

    pub async fn authenticate(&self, raw_bearer: &str) -> AppResult<AuthenticatedKey> {
        let trimmed = raw_bearer.trim();
        if trimmed.is_empty() {
            return Err(AppError::Unauthorized("missing API key".to_string()));
        }

        let hash = sha256_hex(trimmed);
        if let Some(cached) = self.cache.get(&hash).await {
            return match cached {
                Some(entry) if entry.key.is_active => {
                    ensure_not_expired(&entry)?;
                    Ok(entry.key)
                }
                _ => Err(invalid_key()),
            };
        }

        let row = virtual_key::Entity::find()
            .filter(virtual_key::Column::KeyHash.eq(hash.clone()))
            .one(&self.db)
            .await?;

        let Some(row) = row else {
            self.cache.insert(hash, None).await;
            return Err(invalid_key());
        };

        set_or_reset_live_spend(&self.live_spend, &row.id, row.spent_usd);

        let cached = CachedKey {
            key: AuthenticatedKey {
                id: row.id.clone(),
                name: row.name.clone(),
                rpm_limit: row.rpm_limit,
                tpm_limit: row.tpm_limit,
                budget_usd: row.budget_usd,
                allowed_models: parse_models(&row.allowed_models_json),
                is_active: row.is_active == 1,
            },
            spent_usd: row.spent_usd,
            expires_at: row.expires_at.clone(),
        };

        if !cached.key.is_active {
            self.cache.insert(hash, Some(cached)).await;
            debug!(key_id = %row.id, "rejected inactive virtual key");
            return Err(invalid_key());
        }

        self.cache.insert(hash, Some(cached.clone())).await;
        ensure_not_expired(&cached)?;
        Ok(cached.key)
    }

    pub fn check_budget(&self, key: &AuthenticatedKey) -> AppResult<()> {
        let Some(budget) = key.budget_usd else {
            return Ok(());
        };

        let spent = live_spend_micros(&self.live_spend, &key.id) as f64 / 1_000_000.0;
        if spent >= budget {
            return Err(AppError::TooManyRequests(
                "Virtual key USD budget exceeded".to_string(),
            ));
        }

        Ok(())
    }

    pub fn check_model_allowed(
        &self,
        key: &AuthenticatedKey,
        requested_model: &str,
    ) -> AppResult<()> {
        if key.allowed_models.is_empty() || model_is_allowed(&key.allowed_models, requested_model) {
            return Ok(());
        }

        Err(AppError::Forbidden(format!(
            "model '{}' is not allowed for this virtual key",
            requested_model
        )))
    }

    pub async fn invalidate_key_hash(&self, key_hash: &str) {
        self.cache.invalidate(key_hash).await;

        if let Ok(Some(row)) = virtual_key::Entity::find()
            .filter(virtual_key::Column::KeyHash.eq(key_hash))
            .one(&self.db)
            .await
        {
            self.limiters.remove(&row.id);
        }
    }

    pub async fn invalidate_all(&self) {
        self.cache.invalidate_all();
        self.cache.run_pending_tasks().await;
        self.limiters.clear();
    }

    pub fn reset_key_state(&self, key_id: &str, reset_spend: bool) {
        self.limiters.remove(key_id);

        if reset_spend {
            self.live_spend
                .entry(key_id.to_string())
                .or_insert_with(|| Arc::new(AtomicI64::new(0)))
                .store(0, Ordering::Relaxed);
        }
    }

    pub fn effective_spend_usd(&self, key_id: &str, db_spent_usd: f64) -> f64 {
        let live = live_spend_micros(&self.live_spend, key_id) as f64 / 1_000_000.0;
        live.max(db_spent_usd)
    }
}

fn invalid_key() -> AppError {
    AppError::Unauthorized("invalid or inactive API key".to_string())
}

fn ensure_not_expired(entry: &CachedKey) -> AppResult<()> {
    if entry.is_expired() {
        return Err(AppError::Unauthorized("virtual key expired".to_string()));
    }
    Ok(())
}

fn parse_models(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

pub(crate) fn model_is_allowed(allowed: &[String], requested: &str) -> bool {
    let selector = ModelSelector::parse(requested);
    let bare = selector.base_model.as_str();

    allowed.iter().any(|entry| {
        if entry == requested || entry == bare {
            return true;
        }
        match entry.split_once('/') {
            Some((provider, model)) if !provider.is_empty() && !model.is_empty() => {
                model == bare && selector.provider_hint.as_deref() == Some(provider)
            }
            _ => false,
        }
    })
}
