use std::num::NonZeroU32;

use governor::Quota;

use super::{DirectLimiter, KeyLimiterRegistry};

impl KeyLimiterRegistry {
    pub fn reconcile_tpm(&self, key_id: &str, tpm_limit: Option<i64>, additional_tokens: u32) {
        if additional_tokens == 0 {
            return;
        }

        if let Some(cap) = tpm_limit.filter(|cap| *cap > 0) {
            let bucket = self.entry(key_id, None, Some(cap));
            if let Some(bucket) = bucket.tpm.as_ref() {
                saturate(&bucket.limiter, cap, additional_tokens);
            }
        }

        self.usage.record_tokens_extra(key_id, additional_tokens);
    }
}

pub(super) fn quota_limiter(cap: i64) -> DirectLimiter {
    let replenish = std::time::Duration::from_secs_f64(60.0 / cap.max(1) as f64);
    let quota = Quota::with_period(replenish)
        .expect("replenish period is non-zero")
        .allow_burst(NonZeroU32::new(cap.min(i64::from(u32::MAX)) as u32).expect("positive cap"));
    governor::RateLimiter::direct(quota)
}

fn saturate(limiter: &DirectLimiter, cap: i64, tokens: u32) {
    let cells = NonZeroU32::new(tokens).expect("caller checked for non-zero tokens");
    if limiter.check_n(cells).is_ok() {
        return;
    }

    let one = NonZeroU32::new(1).expect("at least one cell");
    let mut drained = 0;
    while drained < cap.max(1) && limiter.check_n(one).is_ok() {
        drained += 1;
    }
}
