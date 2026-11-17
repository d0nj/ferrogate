use super::RateLimitSnapshot;

#[test]
fn uncapped_snapshots_report_no_limits() {
    let snapshot = RateLimitSnapshot::default();
    assert_eq!(snapshot.limit_requests, None);
    assert_eq!(snapshot.remaining_requests, None);
}

#[test]
fn remaining_saturates_at_zero() {
    let limiter = super::KeyLimiterRegistry::new();
    for _ in 0..3 {
        limiter.check_rpm("key-a", Some(5)).unwrap();
    }
    let snapshot = limiter.snapshot("key-a", Some(5), Some(10));
    assert_eq!(snapshot.limit_requests, Some(5));
    assert_eq!(snapshot.remaining_requests, Some(2));
    assert_eq!(snapshot.limit_tokens, Some(10));
    assert_eq!(snapshot.remaining_tokens, Some(10));

    limiter.check_tpm("key-a", Some(10), 4).unwrap();
    assert_eq!(
        limiter
            .snapshot("key-a", Some(5), Some(10))
            .remaining_tokens,
        Some(6)
    );

    limiter.check_tpm("key-a", Some(10), 6).unwrap();
    assert_eq!(
        limiter
            .snapshot("key-a", Some(5), Some(10))
            .remaining_tokens,
        Some(0)
    );
    assert!(limiter.check_tpm("key-a", Some(10), 1).is_err());
}

#[test]
fn uncapped_keys_never_build_buckets() {
    let limiter = super::KeyLimiterRegistry::new();
    limiter.check_rpm("key-b", None).unwrap();
    limiter.check_tpm("key-b", None, 100).unwrap();

    assert!(limiter.is_empty());
    assert_eq!(
        limiter.snapshot("key-b", None, None).remaining_requests,
        None
    );
}

#[test]
fn reconciled_tokens_reduce_the_next_request_budget() {
    let limiter = super::KeyLimiterRegistry::new();
    limiter.check_tpm("key-c", Some(100), 10).unwrap();
    limiter.reconcile_tpm("key-c", Some(100), 40);

    assert_eq!(
        limiter.snapshot("key-c", None, Some(100)).remaining_tokens,
        Some(50)
    );

    limiter.check_tpm("key-c", Some(100), 50).unwrap();
    assert!(limiter.check_tpm("key-c", Some(100), 1).is_err());
}

#[test]
fn reconciling_beyond_capacity_saturates_instead_of_failing() {
    let limiter = super::KeyLimiterRegistry::new();
    limiter.check_tpm("key-d", Some(20), 5).unwrap();
    limiter.reconcile_tpm("key-d", Some(20), 500);

    assert_eq!(
        limiter.snapshot("key-d", None, Some(20)).remaining_tokens,
        Some(0)
    );
    assert!(limiter.check_tpm("key-d", Some(20), 1).is_err());
}

#[test]
fn reconciling_an_uncapped_key_only_moves_counters() {
    let limiter = super::KeyLimiterRegistry::new();
    limiter.reconcile_tpm("key-e", None, 12);

    assert!(limiter.is_empty());
    assert_eq!(
        limiter.snapshot("key-e", None, Some(50)).remaining_tokens,
        Some(38)
    );
}
