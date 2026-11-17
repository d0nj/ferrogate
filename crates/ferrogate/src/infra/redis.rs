use std::sync::atomic::{AtomicBool, Ordering};

use redis::aio::ConnectionManager;
use redis::{AsyncCommands, RedisResult};
use serde::{de::DeserializeOwned, Serialize};
use tracing::warn;

pub const KEY_PREFIX: &str = "fg:";

static INCR_WINDOW_WITH_TTL_REPAIR: std::sync::LazyLock<redis::Script> =
    std::sync::LazyLock::new(|| {
        redis::Script::new(
            "local total = redis.call('INCR', KEYS[1]) \
         if tonumber(total) == 1 or redis.call('TTL', KEYS[1]) == -1 then \
         redis.call('EXPIRE', KEYS[1], ARGV[1]) end \
         return total",
        )
    });

pub type RedisValue<T> = Option<T>;

#[derive(Clone)]
pub struct RedisStore {
    manager: ConnectionManager,
    healthy: std::sync::Arc<AtomicBool>,
}

impl std::fmt::Debug for RedisStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisStore")
            .field("healthy", &self.healthy.load(Ordering::Relaxed))
            .finish()
    }
}

impl RedisStore {
    pub async fn connect(url: &str) -> Result<Self, String> {
        let client = redis::Client::open(url).map_err(|err| err.to_string())?;
        let manager = client
            .get_connection_manager()
            .await
            .map_err(|err| err.to_string())?;
        Ok(Self {
            manager,
            healthy: std::sync::Arc::new(AtomicBool::new(true)),
        })
    }

    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }

    pub async fn ping(&self) -> bool {
        let mut conn = self.manager.clone();
        let ok: RedisResult<String> = redis::cmd("PING").query_async(&mut conn).await;
        let up = ok.is_ok();
        self.healthy.store(up, Ordering::Relaxed);
        up
    }

    pub async fn get_json<T: DeserializeOwned>(&self, key: &str) -> RedisValue<T> {
        let raw = self.get_str(key).await?;
        serde_json::from_str(&raw).ok()
    }

    pub async fn set_json<T: Serialize>(&self, key: &str, value: &T, ttl_secs: u64) {
        let Ok(raw) = serde_json::to_string(value) else {
            return;
        };
        self.set_str(key, &raw, ttl_secs).await;
    }

    pub async fn get_str(&self, key: &str) -> RedisValue<String> {
        let mut conn = self.manager.clone();
        let result: RedisResult<Option<String>> = conn.get(key).await;
        self.record(result, "get").flatten()
    }

    pub async fn set_str(&self, key: &str, value: &str, ttl_secs: u64) {
        if ttl_secs == 0 {
            return;
        }
        let mut conn = self.manager.clone();
        let result: RedisResult<String> = conn.set_ex(key, value, ttl_secs).await;
        self.record(result, "set_ex");
    }

    pub async fn delete_prefix(&self, prefix: &str) {
        let mut conn = self.manager.clone();
        let pattern = format!("{prefix}*");
        let mut cursor = 0u64;
        loop {
            let page: RedisResult<(u64, Vec<String>)> = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(256)
                .query_async(&mut conn)
                .await;
            match self.record(page, "scan") {
                Some((next, keys)) => {
                    if !keys.is_empty() {
                        let deleted: RedisResult<i64> = conn.del(&keys).await;
                        self.record(deleted, "del");
                    }
                    cursor = next;
                }
                None => return,
            }
            if cursor == 0 {
                return;
            }
        }
    }

    pub async fn incr_window(&self, key: &str, delta: i64, window_secs: u64) -> RedisValue<i64> {
        if delta <= 0 || window_secs == 0 {
            let mut conn = self.manager.clone();
            let value: RedisResult<i64> = conn.incr(key, delta).await;
            return self.record(value, "incr");
        }
        let mut conn = self.manager.clone();
        let value: RedisResult<i64> = INCR_WINDOW_WITH_TTL_REPAIR
            .key(key)
            .arg(window_secs as i64)
            .invoke_async(&mut conn)
            .await;
        self.record(value, "incr_window")
    }

    fn record<T>(&self, result: RedisResult<T>, op: &str) -> Option<T> {
        match result {
            Ok(value) => {
                self.healthy.store(true, Ordering::Relaxed);
                Some(value)
            }
            Err(err) => {
                self.healthy.store(false, Ordering::Relaxed);
                warn!(op, error = %err, "redis command failed; continuing without it");
                None
            }
        }
    }
}
