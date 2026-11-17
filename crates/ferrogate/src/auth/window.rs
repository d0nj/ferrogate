use std::{
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dashmap::DashMap;

const WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateLimitSnapshot {
    pub limit_requests: Option<i64>,
    pub remaining_requests: Option<u32>,
    pub limit_tokens: Option<i64>,
    pub remaining_tokens: Option<u32>,
}

#[derive(Debug, Default)]
pub(crate) struct WindowCounter {
    minute: AtomicU64,
    pub(crate) requests: AtomicU32,
    pub(crate) tokens: AtomicU32,
}

impl WindowCounter {
    fn current(&self, minute: u64) -> (u32, u32) {
        if self.minute.load(Ordering::Relaxed) != minute {
            self.minute.store(minute, Ordering::Relaxed);
            self.requests.store(0, Ordering::Relaxed);
            self.tokens.store(0, Ordering::Relaxed);
        }
        (
            self.requests.load(Ordering::Relaxed),
            self.tokens.load(Ordering::Relaxed),
        )
    }
}

#[derive(Default)]
pub(crate) struct UsageCounters {
    counters: DashMap<String, Arc<WindowCounter>>,
}

impl UsageCounters {
    pub(crate) fn counter(&self, key_id: &str) -> Arc<WindowCounter> {
        let minute = current_minute();
        let counter = self.counters.entry(key_id.to_string()).or_default().clone();
        counter.current(minute);
        counter
    }

    pub(crate) fn snapshot(
        &self,
        key_id: &str,
        rpm_limit: Option<i64>,
        tpm_limit: Option<i64>,
    ) -> RateLimitSnapshot {
        let (requests, tokens) = self
            .counters
            .get(key_id)
            .map(|counter| counter.current(current_minute()))
            .unwrap_or((0, 0));

        RateLimitSnapshot {
            limit_requests: rpm_limit.filter(|cap| *cap > 0),
            remaining_requests: remaining(rpm_limit, requests),
            limit_tokens: tpm_limit.filter(|cap| *cap > 0),
            remaining_tokens: remaining(tpm_limit, tokens),
        }
    }

    pub(crate) fn record_tokens_extra(&self, key_id: &str, tokens: u32) {
        if tokens == 0 {
            return;
        }
        self.counter(key_id)
            .tokens
            .fetch_add(tokens, Ordering::Relaxed);
    }

    pub(crate) fn remove(&self, key_id: &str) {
        self.counters.remove(key_id);
    }

    pub(crate) fn clear(&self) {
        self.counters.clear();
    }
}

fn current_minute() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / WINDOW.as_secs()
}

fn remaining(limit: Option<i64>, used: u32) -> Option<u32> {
    limit.filter(|cap| *cap > 0).map(|cap| {
        let left = (cap - i64::from(used)).max(0);
        left.min(i64::from(u32::MAX)) as u32
    })
}
