use std::sync::Arc;

use crate::{
    error::{AppError, AppResult},
    infra::redis::RedisStore,
    proxy::AuthenticatedKey,
};

pub const RATE_LIMIT_WINDOW_SECS: u64 = 60;

fn rpm_key(key_id: &str) -> String {
    format!("fg:rl:rpm:{key_id}")
}

fn tpm_key(key_id: &str) -> String {
    format!("fg:rl:tpm:{key_id}")
}

pub struct SharedLimiter {
    redis: Option<Arc<RedisStore>>,
}

impl SharedLimiter {
    pub fn new(redis: Option<Arc<RedisStore>>) -> Self {
        Self { redis }
    }

    pub async fn admit_rpm(&self, key: &AuthenticatedKey) -> AppResult<()> {
        self.consume(&rpm_key(&key.id), key.rpm_limit, 1).await
    }

    pub async fn consume_tpm(&self, key: &AuthenticatedKey, tokens: u32) -> AppResult<()> {
        self.consume(&tpm_key(&key.id), key.tpm_limit, tokens).await
    }

    pub async fn reconcile_tpm(&self, key: &AuthenticatedKey, output_tokens: u32) {
        if output_tokens == 0 {
            return;
        }
        if let Err(err) = self
            .consume(&tpm_key(&key.id), key.tpm_limit, output_tokens)
            .await
        {
            tracing::debug!(key_id = %key.id, error = %err, "shared tpm reconcile skipped");
        }
    }

    async fn consume(&self, key: &str, limit: Option<i64>, delta: u32) -> AppResult<()> {
        let (Some(redis), Some(cap)) = (self.redis.as_ref(), limit.filter(|cap| *cap > 0)) else {
            return Ok(());
        };

        match redis
            .incr_window(key, i64::from(delta), RATE_LIMIT_WINDOW_SECS)
            .await
        {
            Some(used) if used > cap => Err(too_many(cap)),
            _ => Ok(()),
        }
    }
}

fn too_many(cap: i64) -> AppError {
    AppError::TooManyRequests(format!(
        "virtual key shared rate limit exceeded ({cap} per {RATE_LIMIT_WINDOW_SECS}s)"
    ))
}
