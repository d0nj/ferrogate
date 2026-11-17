use std::sync::atomic::{AtomicBool, Ordering};

use dashmap::DashMap;

use super::RoutingTarget;

pub const EWMA_ALPHA: f64 = 0.3;

const WARMUP_SAMPLES: u64 = 10;

const MAX_PLAUSIBLE_MS: f64 = 600_000.0;

#[derive(Debug, Clone, Copy)]
struct EwmaEntry {
    ewma_ms: f64,
    samples: u64,
}

pub struct EwmaRegistry {
    entries: DashMap<(String, String), EwmaEntry>,
    warm: AtomicBool,
}

impl Default for EwmaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl EwmaRegistry {
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
            warm: AtomicBool::new(false),
        }
    }

    pub fn record(&self, provider_id: &str, model: &str, latency_ms: f64) {
        if !latency_ms.is_finite() || !(0.0..=MAX_PLAUSIBLE_MS).contains(&latency_ms) {
            return;
        }

        let key = (provider_id.to_string(), model.to_string());
        let mut entry = self.entries.entry(key).or_insert(EwmaEntry {
            ewma_ms: latency_ms,
            samples: 0,
        });

        entry.ewma_ms = match entry.samples {
            0 => latency_ms,
            _ => EWMA_ALPHA * latency_ms + (1.0 - EWMA_ALPHA) * entry.ewma_ms,
        };
        entry.samples = entry.samples.saturating_add(1);

        if entry.samples >= WARMUP_SAMPLES {
            self.warm.store(true, Ordering::Relaxed);
        }
    }

    pub fn score_ms(&self, provider_id: &str, model: &str) -> Option<f64> {
        self.entries
            .get(&(provider_id.to_string(), model.to_string()))
            .map(|entry| entry.ewma_ms)
    }

    pub fn is_warm(&self) -> bool {
        self.warm.load(Ordering::Relaxed)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&self) {
        self.entries.clear();
        self.warm.store(false, Ordering::Relaxed);
    }

    pub fn sort_by_latency(&self, targets: &mut [RoutingTarget]) {
        if targets.len() < 2 {
            return;
        }

        let head_position = targets[0].position;
        let reorderable = targets
            .iter()
            .take_while(|target| target.position == head_position)
            .count();
        if reorderable < 2 {
            return;
        }

        self.sort_span_by_latency(&mut targets[..reorderable]);
    }

    pub fn sort_span_by_latency(&self, tier: &mut [RoutingTarget]) {
        if tier.len() < 2 {
            return;
        }
        tier.sort_by(
            |left, right| match (self.score_for(left), self.score_for(right)) {
                (Some(a), Some(b)) => a.total_cmp(&b),
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
            },
        );
    }

    fn score_for(&self, target: &RoutingTarget) -> Option<f64> {
        self.score_ms(&target.provider_id, &target.upstream_model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderKind;

    fn target(provider_id: &str) -> RoutingTarget {
        RoutingTarget {
            provider_id: provider_id.to_string(),
            provider_name: provider_id.to_string(),
            kind: ProviderKind::OpenAi,
            dialect: crate::providers::ApiDialect::Chat,
            base_url: "https://example.test".to_string(),
            api_key: "sk-test".to_string(),
            upstream_model: "gpt-4o".to_string(),
            timeout_ms: 60_000,
            rule_id: None,
            weight: 1,
            position: 0,
            zdr: false,
            region: None,
            extra_headers: Default::default(),
            selected_key_id: None,
            selected_key_name: None,
        }
    }

    #[test]
    fn first_sample_initialises_and_later_samples_smooth() {
        let registry = EwmaRegistry::new();
        registry.record("p1", "gpt-4o", 100.0);
        assert_eq!(registry.score_ms("p1", "gpt-4o"), Some(100.0));

        registry.record("p1", "gpt-4o", 200.0);
        let expected = EWMA_ALPHA * 200.0 + (1.0 - EWMA_ALPHA) * 100.0;
        let actual = registry.score_ms("p1", "gpt-4o").unwrap();
        assert!((actual - expected).abs() < 1e-9);
    }

    #[test]
    fn implausible_samples_are_ignored() {
        let registry = EwmaRegistry::new();
        registry.record("p1", "gpt-4o", f64::NAN);
        registry.record("p1", "gpt-4o", -5.0);
        assert!(registry.is_empty());
    }

    #[test]
    fn warm_flag_flips_after_enough_samples() {
        let registry = EwmaRegistry::new();
        assert!(!registry.is_warm());
        for _ in 0..WARMUP_SAMPLES {
            registry.record("p1", "gpt-4o", 50.0);
        }
        assert!(registry.is_warm());
    }

    #[test]
    fn cold_candidates_stay_ahead_of_slow_providers() {
        let registry = EwmaRegistry::new();
        for _ in 0..WARMUP_SAMPLES {
            registry.record("fast", "gpt-4o", 40.0);
            registry.record("slow", "gpt-4o", 900.0);
        }

        let mut targets = vec![target("slow"), target("fast"), target("cold")];
        registry.sort_by_latency(&mut targets);
        assert_eq!(targets[2].provider_id, "slow");

        let fast_at = targets
            .iter()
            .position(|target| target.provider_id == "fast")
            .unwrap();
        let cold_at = targets
            .iter()
            .position(|target| target.provider_id == "cold")
            .unwrap();
        assert!(cold_at < fast_at || fast_at == 0);
    }

    #[test]
    fn later_positions_are_never_reordered() {
        let registry = EwmaRegistry::new();
        for _ in 0..WARMUP_SAMPLES {
            registry.record("a", "gpt-4o", 10.0);
            registry.record("b", "gpt-4o", 500.0);
        }

        let mut low = target("b");
        low.position = 1;
        let mut high = target("b");
        high.position = 0;
        let mut targets = vec![high, low];
        registry.sort_by_latency(&mut targets);

        assert_eq!(targets[1].position, 1);
        assert_eq!(targets[0].position, 0);
    }
}
