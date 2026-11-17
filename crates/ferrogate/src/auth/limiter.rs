use std::{
    num::NonZeroU32,
    sync::{atomic::Ordering, Arc},
};

use dashmap::DashMap;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    RateLimiter,
};

pub use super::window::RateLimitSnapshot;
use super::window::UsageCounters;
use crate::error::{AppError, AppResult};
use saturate::quota_limiter;

pub type DirectLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

pub struct Bucket {
    pub cap: i64,
    pub limiter: Arc<DirectLimiter>,
}

pub struct KeyRateLimiters {
    pub rpm: Option<Bucket>,
    pub tpm: Option<Bucket>,
}

impl KeyRateLimiters {
    fn new(rpm_cap: i64, tpm_cap: i64) -> Self {
        Self {
            rpm: build(rpm_cap),
            tpm: build(tpm_cap),
        }
    }

    fn clone_of(current: &Self) -> Self {
        Self {
            rpm: current.rpm.clone(),
            tpm: current.tpm.clone(),
        }
    }

    fn sync_rpm(&mut self, cap: i64) {
        if self.rpm.as_ref().map(|bucket| bucket.cap) != Some(cap) {
            self.rpm = build(cap);
        }
    }

    fn sync_tpm(&mut self, cap: i64) {
        if self.tpm.as_ref().map(|bucket| bucket.cap) != Some(cap) {
            self.tpm = build(cap);
        }
    }
}

impl Clone for Bucket {
    fn clone(&self) -> Self {
        Self {
            cap: self.cap,
            limiter: self.limiter.clone(),
        }
    }
}

fn build(cap: i64) -> Option<Bucket> {
    (cap > 0).then(|| Bucket {
        cap,
        limiter: Arc::new(quota_limiter(cap)),
    })
}

#[derive(Default)]
pub struct KeyLimiterRegistry {
    keys: DashMap<String, Arc<KeyRateLimiters>>,
    usage: UsageCounters,
}

impl KeyLimiterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn snapshot(
        &self,
        key_id: &str,
        rpm_limit: Option<i64>,
        tpm_limit: Option<i64>,
    ) -> RateLimitSnapshot {
        self.usage.snapshot(key_id, rpm_limit, tpm_limit)
    }

    pub fn check_rpm(&self, key_id: &str, rpm_limit: Option<i64>) -> AppResult<()> {
        let Some(cap) = rpm_limit.filter(|cap| *cap > 0) else {
            return Ok(());
        };
        let bucket = self.entry(key_id, Some(cap), None);
        let limiter = bucket
            .rpm
            .as_ref()
            .map(|bucket| bucket.limiter.clone())
            .ok_or_else(|| AppError::TooManyRequests("request rate limit unavailable".into()))?;

        limiter.check().map_err(|_| {
            AppError::TooManyRequests(format!(
                "virtual key request rate limit exceeded ({cap} requests/min)"
            ))
        })?;

        self.usage
            .counter(key_id)
            .requests
            .fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub fn check_tpm(&self, key_id: &str, tpm_limit: Option<i64>, tokens: u32) -> AppResult<()> {
        let Some(cap) = tpm_limit.filter(|cap| *cap > 0) else {
            return Ok(());
        };

        let cells = NonZeroU32::new(tokens.max(1)).expect("clamped to at least one cell");
        let bucket = self.entry(key_id, None, Some(cap));
        let limiter = bucket
            .tpm
            .as_ref()
            .map(|bucket| bucket.limiter.clone())
            .ok_or_else(|| AppError::TooManyRequests("token rate limit unavailable".into()))?;

        match limiter.check_n(cells) {
            Ok(Ok(_)) if i64::from(tokens) <= cap => {
                self.usage
                    .counter(key_id)
                    .tokens
                    .fetch_add(tokens, Ordering::Relaxed);
                Ok(())
            }
            Ok(Ok(_)) => Err(AppError::TooManyRequests(format!(
                "request of {tokens} tokens exceeds the {cap} tokens/min limit"
            ))),
            Ok(Err(_)) => Err(AppError::TooManyRequests(format!(
                "virtual key token rate limit exceeded ({cap} tokens/min)"
            ))),
            Err(_) => Err(AppError::TooManyRequests(format!(
                "{cells} tokens exceed the {cap} tokens/min burst"
            ))),
        }
    }

    pub fn remove(&self, key_id: &str) {
        self.keys.remove(key_id);
        self.usage.remove(key_id);
    }

    pub fn clear(&self) {
        self.keys.clear();
        self.usage.clear();
    }

    fn entry(
        &self,
        key_id: &str,
        rpm_cap: Option<i64>,
        tpm_cap: Option<i64>,
    ) -> Arc<KeyRateLimiters> {
        let existing = self.keys.get(key_id).map(|entry| entry.value().clone());

        if let Some(current) = existing.as_ref() {
            let rpm_ok = rpm_cap.is_none_or(|cap| current.rpm.as_ref().map(|b| b.cap) == Some(cap));
            let tpm_ok = tpm_cap.is_none_or(|cap| current.tpm.as_ref().map(|b| b.cap) == Some(cap));
            if rpm_ok && tpm_ok {
                return current.clone();
            }
        }

        let mut fresh = match existing.as_ref() {
            Some(current) => KeyRateLimiters::clone_of(current),
            None => KeyRateLimiters::new(rpm_cap.unwrap_or(0), tpm_cap.unwrap_or(0)),
        };
        if let Some(cap) = rpm_cap {
            fresh.sync_rpm(cap);
        }
        if let Some(cap) = tpm_cap {
            fresh.sync_tpm(cap);
        }

        let shared = Arc::new(fresh);
        self.keys.insert(key_id.to_string(), shared.clone());
        shared
    }
}

#[path = "limiter_saturate.rs"]
mod saturate;

#[cfg(test)]
#[path = "limiter_tests.rs"]
mod limiter_tests;
