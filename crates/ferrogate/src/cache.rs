use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::http::HeaderMap;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    infra::redis::RedisStore,
    providers::{ChatCompletionRequest, ChatCompletionResponse},
};

pub const CACHE_TTL: Duration = Duration::from_secs(60);

pub const CACHE_CAPACITY: u64 = 2_000;

const REDIS_KEY_PREFIX: &str = "fg:cache:";

pub struct ResponseCache {
    inner: moka::future::Cache<String, ChatCompletionResponse>,
    redis: Option<Arc<RedisStore>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl Default for ResponseCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponseCache {
    pub fn new() -> Self {
        Self::with_redis(None)
    }

    pub fn with_redis(redis: Option<Arc<RedisStore>>) -> Self {
        let inner = moka::future::Cache::builder()
            .time_to_live(CACHE_TTL)
            .max_capacity(CACHE_CAPACITY)
            .build();
        Self {
            inner,
            redis,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    pub async fn get(&self, key: &str) -> Option<ChatCompletionResponse> {
        if let Some(hit) = self.inner.get(key).await {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return Some(hit);
        }

        let stored: Option<ChatCompletionResponse> = match self.redis.as_ref() {
            Some(redis) => redis.get_json(&redis_key(key)).await,
            None => None,
        };

        match stored {
            Some(response) => {
                self.inner.insert(key.to_string(), response.clone()).await;
                self.hits.fetch_add(1, Ordering::Relaxed);
                Some(response)
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    pub async fn insert(&self, key: String, response: ChatCompletionResponse) {
        if let Some(redis) = self.redis.as_ref() {
            let redis = redis.clone();
            let redis_key = redis_key(&key);
            let value = response.clone();
            tokio::spawn(async move {
                redis
                    .set_json(&redis_key, &value, CACHE_TTL.as_secs())
                    .await;
            });
        }
        self.inner.insert(key, response).await;
    }

    pub async fn clear(&self) {
        self.inner.invalidate_all();
        self.inner.run_pending_tasks().await;

        if let Some(redis) = self.redis.clone() {
            tokio::spawn(async move { redis.delete_prefix(REDIS_KEY_PREFIX).await });
        }
    }

    pub fn stats(&self) -> (u64, u64) {
        (
            self.hits.load(Ordering::Relaxed),
            self.misses.load(Ordering::Relaxed),
        )
    }

    pub fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }
}

fn redis_key(key: &str) -> String {
    format!("{REDIS_KEY_PREFIX}{key}")
}

pub fn cache_key(alias: &str, req: &ChatCompletionRequest, key_id: Option<&str>) -> String {
    let canonical = json!({
        "alias": alias,
        "key_id": key_id,
        "messages": req.messages,
        "temperature": req.temperature,
        "top_p": req.top_p,
        "max_tokens": req.max_tokens,
        "max_completion_tokens": req.max_completion_tokens,
        "stop": req.stop,
        "presence_penalty": req.presence_penalty,
        "frequency_penalty": req.frequency_penalty,
        "tools": req.tools,
        "tool_choice": req.tool_choice,
        "response_format": req.response_format,
        "seed": req.seed,
    });

    let serialized = serde_json::to_string(&canonical).unwrap_or_default();
    let digest = Sha256::digest(serialized.as_bytes());
    hex::encode(digest)
}

pub fn is_cache_eligible(req: &ChatCompletionRequest, headers: &HeaderMap) -> bool {
    if req.stream.unwrap_or(false) {
        return false;
    }

    if header_eq(headers, "cache-control", "no-cache") {
        return false;
    }
    if header_eq(headers, "x-ferrogate-cache", "false") {
        return false;
    }

    let explicit_opt_in = header_eq(headers, "x-ferrogate-cache", "true");
    let deterministic = req.temperature == Some(0.0);

    deterministic || explicit_opt_in
}

fn header_eq(headers: &HeaderMap, name: &str, expected: &str) -> bool {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

pub const CACHE_HIT_HEADER: &str = "X-FerroGate-Cache";

pub fn cache_hit_body(response: &ChatCompletionResponse) -> Value {
    serde_json::to_value(response).unwrap_or(Value::Null)
}
