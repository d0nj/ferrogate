use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use dashmap::DashMap;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};

use crate::entities::provider_key;

use super::parse_models_json;

const KEY_CACHE_TTL: Duration = Duration::from_secs(5);
const KEY_CACHE_CAPACITY: u64 = 256;
const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(30);
const AUTH_FAILURE_COOLDOWN: Duration = Duration::from_secs(300);
const SERVER_ERROR_COOLDOWN: Duration = Duration::from_secs(15);
const SERVER_ERROR_THRESHOLD: u32 = 2;
const MAX_ERROR_CHARS: usize = 512;

#[derive(Debug, Clone)]
pub struct ProviderKeyEntry {
    pub id: String,
    pub name: String,
    pub api_key: String,
    pub weight: i64,
    pub models: Vec<String>,
    pub rpm_limit: Option<i64>,
    pub cooldown_until: Option<String>,
    pub last_error: Option<String>,
}

impl ProviderKeyEntry {
    fn from_model(row: provider_key::Model, secrets: &crate::crypto::SecretBox) -> Self {
        let api_key = secrets.open(&row.api_key).unwrap_or_else(|err| {
            tracing::error!(key = %row.id, error = %err, "stored credential could not be decrypted");
            String::new()
        });

        Self {
            models: parse_models_json(&row.models_json),
            weight: row.weight,
            rpm_limit: row.rpm_limit,
            cooldown_until: row.cooldown_until,
            last_error: row.last_error,
            id: row.id,
            name: row.name,
            api_key,
        }
    }

    pub fn serves(&self, model: &str) -> bool {
        self.models.is_empty() || self.models.iter().any(|known| known == model)
    }

    fn pick(&self) -> f64 {
        use rand::Rng;

        let weight = self.weight.max(1) as f64;
        rand::rng()
            .random_range(f64::MIN_POSITIVE..1.0)
            .powf(1.0 / weight)
    }
}

#[derive(Debug, Clone)]
struct KeyCooldown {
    until: Instant,
    last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedKey {
    pub id: Option<String>,
    pub name: Option<String>,
    pub api_key: String,
}

impl SelectedKey {
    pub fn fallback(api_key: &str) -> Self {
        Self {
            id: None,
            name: None,
            api_key: api_key.to_string(),
        }
    }
}

impl From<&ProviderKeyEntry> for SelectedKey {
    fn from(entry: &ProviderKeyEntry) -> Self {
        Self {
            id: Some(entry.id.clone()),
            name: Some(entry.name.clone()),
            api_key: entry.api_key.clone(),
        }
    }
}

fn current_minute() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() / 60)
        .unwrap_or(0)
}

fn stable_hash(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn cooldown_window(status: Option<u16>, failures: u32) -> Option<Duration> {
    let server_error =
        matches!(status, Some(500..=599) | None) && failures >= SERVER_ERROR_THRESHOLD;
    if server_error {
        return Some(SERVER_ERROR_COOLDOWN);
    }
    match status {
        Some(429) => Some(RATE_LIMIT_COOLDOWN),
        Some(401 | 403) => Some(AUTH_FAILURE_COOLDOWN),
        _ => None,
    }
}

fn truncate(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((cut, _)) => text[..cut].to_string(),
        None => text.to_string(),
    }
}

fn cooldown_timestamp(remaining: Duration) -> String {
    let offset = chrono::Duration::from_std(remaining).unwrap_or_else(|_| chrono::Duration::zero());
    (chrono::Utc::now() + offset).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub struct ProviderKeyPool {
    db: DatabaseConnection,
    secrets: Arc<crate::crypto::SecretBox>,
    cache: moka::future::Cache<String, Arc<Vec<ProviderKeyEntry>>>,
    cooldowns: DashMap<String, KeyCooldown>,
    streaks: DashMap<String, u32>,
    rpm: DashMap<String, (u64, u32)>,
}

impl ProviderKeyPool {
    pub fn new(db: DatabaseConnection, secrets: Arc<crate::crypto::SecretBox>) -> Self {
        let cache = moka::future::Cache::builder()
            .time_to_live(KEY_CACHE_TTL)
            .max_capacity(KEY_CACHE_CAPACITY)
            .build();
        Self {
            db,
            secrets,
            cache,
            cooldowns: DashMap::new(),
            streaks: DashMap::new(),
            rpm: DashMap::new(),
        }
    }

    pub async fn entries(&self, provider_id: &str) -> Arc<Vec<ProviderKeyEntry>> {
        if let Some(cached) = self.cache.get(provider_id).await {
            return cached;
        }

        let loaded = match provider_key::Entity::find()
            .filter(provider_key::Column::ProviderId.eq(provider_id))
            .filter(provider_key::Column::IsActive.eq(1))
            .order_by_asc(provider_key::Column::Name)
            .all(&self.db)
            .await
        {
            Ok(rows) => Arc::new(
                rows.into_iter()
                    .map(|row| ProviderKeyEntry::from_model(row, &self.secrets))
                    .collect(),
            ),
            Err(error) => {
                tracing::warn!(
                    provider_id = %provider_id,
                    %error,
                    "failed to load provider keys; falling back to the provider credential"
                );
                Arc::new(Vec::new())
            }
        };

        self.cache
            .insert(provider_id.to_string(), loaded.clone())
            .await;
        loaded
    }

    pub fn in_cooldown(&self, key_id: &str) -> bool {
        let cooling = self
            .cooldowns
            .get(key_id)
            .is_some_and(|state| state.until > Instant::now());
        if !cooling {
            self.cooldowns.remove(key_id);
        }
        cooling
    }

    fn rpm_allowed(&self, entry: &ProviderKeyEntry) -> bool {
        let Some(cap) = entry.rpm_limit.filter(|cap| *cap > 0) else {
            return true;
        };
        let minute = current_minute();
        match self.rpm.get(&entry.id) {
            Some(used) if used.0 == minute => used.1 < cap as u32,
            _ => true,
        }
    }

    fn note_use(&self, key_id: &str) {
        let minute = current_minute();
        let mut used = self.rpm.entry(key_id.to_string()).or_insert((minute, 0));
        if used.0 != minute {
            *used = (minute, 0);
        }
        used.1 = used.1.saturating_add(1);
    }

    pub async fn select_key(
        &self,
        provider_id: &str,
        fallback_api_key: &str,
        model: &str,
        preferred_key_id: Option<&str>,
    ) -> SelectedKey {
        let entries = self.entries(provider_id).await;
        let eligible: Vec<&ProviderKeyEntry> = entries
            .iter()
            .filter(|entry| !entry.api_key.trim().is_empty() && entry.serves(model))
            .collect();
        if eligible.is_empty() {
            return SelectedKey::fallback(fallback_api_key);
        }

        if let Some(preferred) = preferred_key_id {
            if let Some(entry) = eligible.iter().find(|entry| {
                entry.id == preferred && !self.in_cooldown(&entry.id) && self.rpm_allowed(entry)
            }) {
                self.note_use(&entry.id);
                return SelectedKey::from(*entry);
            }
        }

        let ready: Vec<&ProviderKeyEntry> = eligible
            .iter()
            .copied()
            .filter(|entry| !self.in_cooldown(&entry.id) && self.rpm_allowed(entry))
            .collect();
        match ready.is_empty() {
            true => eligible
                .iter()
                .min_by_key(|entry| self.cooldowns.get(&entry.id).map(|state| state.until))
                .map(|entry| {
                    self.note_use(&entry.id);
                    SelectedKey::from(*entry)
                })
                .unwrap_or_else(|| SelectedKey::fallback(fallback_api_key)),
            false => {
                let chosen = ready
                    .iter()
                    .copied()
                    .max_by(|left, right| left.pick().total_cmp(&right.pick()))
                    .unwrap_or(ready[0]);
                self.note_use(&chosen.id);
                SelectedKey::from(chosen)
            }
        }
    }

    pub async fn select_key_for(
        &self,
        sticky_salt: &str,
        provider_id: &str,
        fallback_api_key: &str,
        model: &str,
        preferred_key_id: Option<&str>,
    ) -> SelectedKey {
        if sticky_salt.is_empty() {
            return self
                .select_key(provider_id, fallback_api_key, model, preferred_key_id)
                .await;
        }

        let entries = self.entries(provider_id).await;
        let ready: Vec<&ProviderKeyEntry> = entries
            .iter()
            .filter(|entry| {
                !entry.api_key.trim().is_empty()
                    && entry.serves(model)
                    && !self.in_cooldown(&entry.id)
                    && self.rpm_allowed(entry)
            })
            .collect();
        if ready.is_empty() {
            return self
                .select_key(provider_id, fallback_api_key, model, preferred_key_id)
                .await;
        }

        let index = (stable_hash(sticky_salt) as usize) % ready.len();
        self.note_use(&ready[index].id);
        SelectedKey::from(ready[index])
    }

    pub fn record_success(&self, key_id: &str) {
        self.cooldowns.remove(key_id);
        self.streaks.remove(key_id);
    }

    pub fn record_failure(&self, key_id: &str, status: Option<u16>, error: &str) {
        let now = Instant::now();
        let failures = self
            .streaks
            .get(key_id)
            .map(|seen| seen.saturating_add(1))
            .unwrap_or(1);
        self.streaks.insert(key_id.to_string(), failures);

        self.cooldowns.insert(
            key_id.to_string(),
            KeyCooldown {
                until: now + cooldown_window(status, failures).unwrap_or_default(),
                last_error: Some(truncate(error, MAX_ERROR_CHARS)),
            },
        );
    }

    pub fn cooldown_snapshot(&self, key_id: &str) -> (bool, Option<String>, Option<String>) {
        let now = Instant::now();
        match self.cooldowns.get(key_id) {
            Some(state) if state.until > now => (
                true,
                Some(cooldown_timestamp(state.until - now)),
                state.last_error.clone(),
            ),
            Some(state) => (false, None, state.last_error.clone()),
            None => (false, None, None),
        }
    }

    pub fn invalidate(&self) {
        self.cache.invalidate_all();
    }

    pub async fn invalidate_provider(&self, provider_id: &str) {
        self.cache.invalidate(provider_id).await;
        self.cache.run_pending_tasks().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, weight: i64, models: &[&str]) -> ProviderKeyEntry {
        ProviderKeyEntry {
            id: id.to_string(),
            name: format!("key {id}"),
            api_key: format!("sk-{id}"),
            weight,
            models: models.iter().map(|model| model.to_string()).collect(),
            rpm_limit: None,
            cooldown_until: None,
            last_error: None,
        }
    }

    fn pool() -> ProviderKeyPool {
        ProviderKeyPool {
            db: DatabaseConnection::default(),
            secrets: Arc::new(crate::crypto::SecretBox::test_key()),
            cache: moka::future::Cache::builder().build(),
            cooldowns: DashMap::new(),
            streaks: DashMap::new(),
            rpm: DashMap::new(),
        }
    }

    #[test]
    fn model_filter_accepts_any_when_unrestricted() {
        assert!(entry("a", 1, &[]).serves("gpt-4o"));
        assert!(entry("a", 1, &["gpt-4o"]).serves("gpt-4o"));
        assert!(!entry("a", 1, &["gpt-4o"]).serves("claude-3"));
    }

    #[test]
    fn heavier_weights_win_more_often() {
        let heavy = entry("heavy", 10, &[]);
        let light = entry("light", 1, &[]);
        let heavy_wins = (0..500).filter(|_| heavy.pick() > light.pick()).count();
        assert!(heavy_wins > 300, "heavy won only {heavy_wins}/500");
    }

    #[test]
    fn auth_failure_cools_down_and_success_clears() {
        let pool = pool();
        pool.record_failure("k1", Some(401), "unauthorized");

        let (cooling, until, error) = pool.cooldown_snapshot("k1");
        assert!(cooling);
        assert!(until.is_some());
        assert_eq!(error.as_deref(), Some("unauthorized"));

        pool.record_success("k1");
        assert!(!pool.cooldown_snapshot("k1").0);
    }

    #[test]
    fn server_errors_cool_only_after_threshold() {
        let pool = pool();
        pool.record_failure("k2", Some(502), "bad gateway");
        assert!(!pool.in_cooldown("k2"));
        pool.record_failure("k2", Some(502), "bad gateway");
        assert!(pool.in_cooldown("k2"));
    }

    #[test]
    fn cooldown_timestamp_is_rfc3339() {
        let stamp = cooldown_timestamp(Duration::from_secs(30));
        assert!(stamp.ends_with('Z'));
        assert!(chrono::DateTime::parse_from_rfc3339(&stamp).is_ok());
    }
}
