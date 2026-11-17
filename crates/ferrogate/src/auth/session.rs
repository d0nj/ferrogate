use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use moka::future::Cache;
use rand::Rng;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use sha2::{Digest, Sha256};

use super::role::{self, Capability, Role};
use crate::{
    entities::session::{self as row, ActiveModel as SessionRow, Column},
    infra::redis::RedisStore,
};

pub const TOKEN_PREFIX: &str = "fgs_";

pub const SESSION_TTL_SECS: u64 = 43_200;

const SESSION_CAPACITY: u64 = 4_096;

const REDIS_KEY_PREFIX: &str = "fg:session:";

#[derive(Debug, Clone)]
pub struct Principal {
    pub user_id: String,
    pub email: String,
    pub role: Role,
    pub must_change_password: bool,
}

impl Principal {
    pub fn label(&self) -> &str {
        &self.email
    }

    pub fn can(&self, cap: Capability) -> bool {
        role::grants(self.role, cap)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Session {
    pub token: String,
    pub user_id: String,
    pub email: String,
    pub role: Role,
    pub issued_at: u64,
    pub expires_at: u64,
}

impl Session {
    pub fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }

    fn remaining(&self, now: u64) -> u64 {
        self.expires_at.saturating_sub(now)
    }
}

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

pub struct SessionStore {
    db: DatabaseConnection,
    node_id: String,
    cache: Cache<String, Session>,
    redis: Option<Arc<RedisStore>>,
    now: Mutex<Clock>,
}

impl SessionStore {
    pub fn new(db: DatabaseConnection) -> Self {
        let node_id = format!("node-{}", uuid::Uuid::new_v4());
        Self {
            db,
            node_id,
            cache: Cache::builder()
                .time_to_live(Duration::from_secs(SESSION_TTL_SECS))
                .max_capacity(SESSION_CAPACITY)
                .build(),
            redis: None,
            now: Mutex::new(Arc::new(unix_now)),
        }
    }

    pub fn with_redis(mut self, redis: Option<Arc<RedisStore>>) -> Self {
        self.redis = redis;
        self
    }

    pub fn with_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = node_id.into();
        self
    }

    pub fn set_clock(&self, now: impl Fn() -> u64 + Send + Sync + 'static) {
        let mut clock = self
            .now
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *clock = Arc::new(now);
    }

    fn current_secs(&self) -> u64 {
        let clock = self.clock();
        clock()
    }

    fn clock(&self) -> Clock {
        self.now
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub async fn mint(&self, user_id: String, email: String, role: Role) -> Session {
        let token = generate_token();
        let digest = digest_token(&token);
        let issued_at = self.current_secs();
        let stored = Session {
            token: digest.clone(),
            user_id,
            email,
            role,
            issued_at,
            expires_at: issued_at.saturating_add(SESSION_TTL_SECS),
        };

        let row = SessionRow {
            token: Set(digest.clone()),
            user_id: Set(stored.user_id.clone()),
            email: Set(stored.email.clone()),
            role: Set(stored.role.as_str().to_owned()),
            issued_at: Set(stored.issued_at as i64),
            expires_at: Set(stored.expires_at as i64),
            node_id: Set(self.node_id.clone()),
        };

        if let Err(err) = row.insert(&self.db).await {
            tracing::error!(%err, "failed to persist session; it will not survive a restart");
        }

        self.publish(&stored, issued_at).await;
        Session {
            token,
            user_id: stored.user_id.clone(),
            email: stored.email.clone(),
            role: stored.role,
            issued_at: stored.issued_at,
            expires_at: stored.expires_at,
        }
    }

    pub async fn get(&self, token: &str) -> Option<Session> {
        let digest = digest_token(token);
        let now = self.current_secs();

        if let Some(session) = self.cache.get(&digest).await {
            if !session.is_expired(now) {
                return Some(session);
            }
            self.revoke(token).await;
            return None;
        }

        if let Some(session) = self.read_redis(&digest, now).await {
            return Some(session);
        }

        let session = self.read_db(&digest).await?;
        if session.is_expired(now) {
            self.revoke(token).await;
            return None;
        }

        self.publish(&session, now).await;
        Some(session)
    }

    pub async fn revoke(&self, token: &str) {
        let digest = digest_token(token);
        self.cache.invalidate(&digest).await;
        self.remove_redis(&digest).await;

        if let Err(err) = row::Entity::delete_by_id(digest).exec(&self.db).await {
            tracing::warn!(%err, "failed to delete session row");
        }
    }

    pub async fn revoke_user(&self, user_id: &str) {
        let tokens = match row::Entity::find()
            .filter(Column::UserId.eq(user_id))
            .all(&self.db)
            .await
        {
            Ok(rows) => rows.into_iter().map(|row| row.token).collect::<Vec<_>>(),
            Err(err) => {
                tracing::warn!(%err, "failed to load sessions for user; revoking by id only");
                Vec::new()
            }
        };

        if let Err(err) = row::Entity::delete_many()
            .filter(Column::UserId.eq(user_id))
            .exec(&self.db)
            .await
        {
            tracing::warn!(%err, user_id, "failed to delete sessions for user");
        }

        for token in tokens {
            self.cache.invalidate(&token).await;
            self.remove_redis(&token).await;
        }
    }

    pub async fn prune_expired(&self) -> u64 {
        let cutoff = self.current_secs() as i64;
        match row::Entity::delete_many()
            .filter(Column::ExpiresAt.lte(cutoff))
            .exec(&self.db)
            .await
        {
            Ok(outcome) => outcome.rows_affected,
            Err(err) => {
                tracing::warn!(%err, "failed to prune expired sessions");
                0
            }
        }
    }

    async fn read_redis(&self, token: &str, now: u64) -> Option<Session> {
        let redis = self.redis.as_ref()?;
        let session: Session = redis.get_json(&redis_key(token)).await?;

        if session.is_expired(now) {
            self.remove_redis(token).await;
            return None;
        }

        self.cache.insert(token.to_owned(), session.clone()).await;
        Some(session)
    }

    async fn read_db(&self, token: &str) -> Option<Session> {
        let stored = match row::Entity::find_by_id(token)
            .to_owned()
            .one(&self.db)
            .await
        {
            Ok(found) => found?,
            Err(err) => {
                tracing::warn!(%err, "failed to load session from database");
                return None;
            }
        };

        Some(Session {
            token: stored.token,
            user_id: stored.user_id,
            email: stored.email,
            role: Role::from_str_strict(&stored.role).unwrap_or(Role::Viewer),
            issued_at: stored.issued_at.max(0) as u64,
            expires_at: stored.expires_at.max(0) as u64,
        })
    }

    async fn publish(&self, session: &Session, now: u64) {
        self.cache
            .insert(session.token.clone(), session.clone())
            .await;

        let Some(redis) = self.redis.as_ref() else {
            return;
        };
        let ttl = session.remaining(now);
        if ttl == 0 {
            return;
        }
        redis
            .set_json(&redis_key(&session.token), session, ttl)
            .await;
    }

    async fn remove_redis(&self, token: &str) {
        if let Some(redis) = self.redis.as_ref() {
            redis.delete_prefix(&redis_key(token)).await;
        }
    }
}

fn redis_key(token: &str) -> String {
    format!("{REDIS_KEY_PREFIX}{token}")
}

fn digest_token(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

fn generate_token() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    format!("{TOKEN_PREFIX}{}", hex::encode(bytes))
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
