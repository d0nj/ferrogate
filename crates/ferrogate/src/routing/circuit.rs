use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::entities::provider;

pub const CIRCUIT_FAILURE_THRESHOLD: u32 = 3;

pub const CIRCUIT_OPEN_DURATION: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealthSnapshot {
    pub provider_id: String,
    pub name: String,
    pub provider_type: String,
    pub is_active: bool,
    pub state: String,
    pub consecutive_failures: u32,
    pub total_requests: u64,
    pub total_failures: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CircuitState {
    pub consecutive_failures: u32,
    pub total_requests: u64,
    pub total_failures: u64,
    pub last_error: Option<String>,
    pub opened_at: Option<Instant>,
}

impl CircuitState {
    fn is_open_for(&self, cooldown: Duration) -> bool {
        self.opened_at
            .is_some_and(|opened_at| opened_at.elapsed() < cooldown)
    }

    fn label(&self, cooldown: Duration) -> &'static str {
        match self.opened_at {
            Some(opened_at) if opened_at.elapsed() < cooldown => "open",
            Some(_) => "half_open",
            None => "closed",
        }
    }
}

#[derive(Default)]
pub struct CircuitBreakerRegistry {
    states: DashMap<String, CircuitState>,
    generation: AtomicU64,
    tuning: std::sync::RwLock<(u32, Duration)>,
}

impl CircuitBreakerRegistry {
    pub fn new() -> Self {
        Self {
            states: DashMap::new(),
            generation: AtomicU64::new(0),
            tuning: std::sync::RwLock::new((CIRCUIT_FAILURE_THRESHOLD, CIRCUIT_OPEN_DURATION)),
        }
    }

    pub fn retune(&self, policy: &super::RoutingPolicy) {
        let mut tuning = self
            .tuning
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *tuning = (
            policy.failure_threshold,
            Duration::from_secs(policy.cooldown_seconds),
        );
    }

    fn threshold(&self) -> u32 {
        self.tuning
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0
    }

    fn cooldown(&self) -> Duration {
        self.tuning
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .1
    }

    pub fn record_success(&self, provider_id: &str) {
        let mut entry = self.states.entry(provider_id.to_string()).or_default();
        entry.consecutive_failures = 0;
        entry.opened_at = None;
        entry.total_requests += 1;
    }

    pub fn record_failure(&self, provider_id: &str, error: impl Into<String>) {
        let error = error.into();
        let mut entry = self.states.entry(provider_id.to_string()).or_default();
        entry.consecutive_failures += 1;
        entry.total_requests += 1;
        entry.total_failures += 1;
        entry.last_error = Some(error);
        if entry.consecutive_failures >= self.threshold() {
            entry.opened_at = Some(Instant::now());
        }
    }

    pub fn is_open(&self, provider_id: &str) -> bool {
        let cooldown = self.cooldown();
        self.states
            .get(provider_id)
            .is_some_and(|state| state.is_open_for(cooldown))
    }

    pub fn is_closed(&self, provider_id: &str) -> bool {
        !self.is_open(provider_id)
    }

    pub fn snapshots(&self, providers: &[provider::Model]) -> Vec<ProviderHealthSnapshot> {
        let cooldown = self.cooldown();
        providers
            .iter()
            .map(|provider| {
                let state = self
                    .states
                    .get(&provider.id)
                    .map(|entry| entry.clone())
                    .unwrap_or_default();
                ProviderHealthSnapshot {
                    provider_id: provider.id.clone(),
                    name: provider.name.clone(),
                    provider_type: provider.provider_type.clone(),
                    is_active: provider.is_active != 0,
                    state: state.label(cooldown).to_string(),
                    consecutive_failures: state.consecutive_failures,
                    total_requests: state.total_requests,
                    total_failures: state.total_failures,
                    last_error: state.last_error.clone(),
                }
            })
            .collect()
    }

    pub fn bump_generation(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }
}

pub type SharedCircuit = Arc<CircuitBreakerRegistry>;

#[cfg(test)]
mod retune_tests {
    use super::*;
    use crate::routing::RoutingPolicy;

    #[test]
    fn threshold_comes_from_the_policy() {
        let registry = CircuitBreakerRegistry::new();
        registry.retune(&RoutingPolicy {
            failure_threshold: 1,
            ..RoutingPolicy::default()
        });

        registry.record_failure("prov-a", "boom");
        assert!(
            registry.is_open("prov-a"),
            "one failure should trip at threshold 1"
        );
    }

    #[test]
    fn a_higher_threshold_keeps_the_circuit_closed() {
        let registry = CircuitBreakerRegistry::new();
        registry.retune(&RoutingPolicy {
            failure_threshold: 5,
            ..RoutingPolicy::default()
        });

        for _ in 0..4 {
            registry.record_failure("prov-b", "boom");
        }
        assert!(!registry.is_open("prov-b"));
        registry.record_failure("prov-b", "boom");
        assert!(registry.is_open("prov-b"));
    }

    #[test]
    fn success_closes_the_circuit() {
        let registry = CircuitBreakerRegistry::new();
        registry.retune(&RoutingPolicy {
            failure_threshold: 1,
            ..RoutingPolicy::default()
        });
        registry.record_failure("prov-c", "boom");
        assert!(registry.is_open("prov-c"));
        registry.record_success("prov-c");
        assert!(!registry.is_open("prov-c"));
    }

    #[test]
    fn zero_cooldown_never_reports_open() {
        let registry = CircuitBreakerRegistry::new();
        registry.retune(&RoutingPolicy {
            failure_threshold: 1,
            cooldown_seconds: 0,
            ..RoutingPolicy::default()
        });
        registry.record_failure("prov-d", "boom");
        assert!(!registry.is_open("prov-d"));
    }
}
