use std::{
    net::{IpAddr, SocketAddr},
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use governor::{
    clock::DefaultClock,
    state::{direct::NotKeyed, InMemoryState},
    Quota, RateLimiter,
};
use ipnet::IpNet;

const MAX_FAILURES: u32 = 5;

const WINDOW: Duration = Duration::from_secs(60);

type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

pub struct LoginThrottle {
    buckets: DashMap<String, (Arc<Limiter>, Instant)>,
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::new()
    }
}

impl LoginThrottle {
    pub fn new() -> Self {
        Self {
            buckets: DashMap::new(),
        }
    }

    fn limiter() -> Limiter {
        let replenish = WINDOW / MAX_FAILURES;
        let quota = Quota::with_period(replenish)
            .expect("replenish period is non-zero")
            .allow_burst(NonZeroU32::new(MAX_FAILURES).expect("positive burst"));
        RateLimiter::direct(quota)
    }

    fn sweep(&self) {
        let now = Instant::now();
        self.buckets
            .retain(|_, (_, seen)| now.duration_since(*seen) < WINDOW);
    }

    pub fn is_allowed(&self, client: &str) -> bool {
        if self.buckets.len() > 1_024 {
            self.sweep();
        }

        let (limiter, _) = self
            .buckets
            .entry(client.to_string())
            .or_insert_with(|| (Arc::new(Self::limiter()), Instant::now()))
            .clone();

        let allowed = limiter.check().is_ok();
        if let Some(mut entry) = self.buckets.get_mut(client) {
            entry.1 = Instant::now();
        }
        allowed
    }

    pub fn clear(&self, client: &str) {
        self.buckets.remove(client);
    }
}

pub fn client_key(
    headers: &axum::http::HeaderMap,
    peer: Option<SocketAddr>,
    trusted: &[IpNet],
) -> String {
    resolve_ip(headers, peer, trusted)
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn resolve_ip(
    headers: &axum::http::HeaderMap,
    peer: Option<SocketAddr>,
    trusted: &[IpNet],
) -> Option<IpAddr> {
    let Some(peer) = peer else {
        return untrusted_forwarded(headers);
    };

    if !is_trusted(peer.ip(), trusted) {
        return Some(peer.ip());
    }

    forwarded_chain(headers, peer.ip(), trusted)
        .or_else(|| real_ip(headers))
        .or(Some(peer.ip()))
}

fn forwarded_chain(
    headers: &axum::http::HeaderMap,
    peer: IpAddr,
    trusted: &[IpNet],
) -> Option<IpAddr> {
    let raw = headers.get("x-forwarded-for")?.to_str().ok()?;
    raw.rsplit(',')
        .filter_map(|entry| entry.trim().parse::<IpAddr>().ok())
        .map(canonical)
        .find(|ip| *ip != canonical(peer) && !is_trusted(*ip, trusted))
}

fn real_ip(headers: &axum::http::HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-real-ip")?
        .to_str()
        .ok()?
        .trim()
        .parse::<IpAddr>()
        .ok()
}

fn untrusted_forwarded(headers: &axum::http::HeaderMap) -> Option<IpAddr> {
    let first = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<IpAddr>().ok())
    };

    first("x-forwarded-for").or_else(|| first("x-real-ip"))
}

fn is_trusted(ip: IpAddr, trusted: &[IpNet]) -> bool {
    let ip = canonical(ip);
    trusted.iter().any(|net| net.contains(&ip))
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        ip => ip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    fn nets(raw: &str) -> Vec<IpNet> {
        crate::config::parse_net_list(raw)
    }

    fn headers(xff: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static(xff));
        headers
    }

    fn peer(addr: &str) -> Option<SocketAddr> {
        Some(SocketAddr::new(addr.parse().unwrap(), 40_000))
    }

    fn peer_at(socket: &str) -> Option<SocketAddr> {
        Some(socket.parse().unwrap())
    }

    #[test]
    fn blocks_after_the_budget_is_spent() {
        let throttle = LoginThrottle::new();
        for _ in 0..MAX_FAILURES {
            assert!(throttle.is_allowed("1.2.3.4"));
        }
        assert!(!throttle.is_allowed("1.2.3.4"));
        assert!(throttle.is_allowed("5.6.7.8"), "other clients unaffected");
    }

    #[test]
    fn clearing_restores_the_budget() {
        let throttle = LoginThrottle::new();
        for _ in 0..MAX_FAILURES {
            throttle.is_allowed("1.2.3.4");
        }
        assert!(!throttle.is_allowed("1.2.3.4"));
        throttle.clear("1.2.3.4");
        assert!(throttle.is_allowed("1.2.3.4"));
    }

    #[test]
    fn falls_back_to_the_first_forwarded_address() {
        assert_eq!(
            client_key(&headers("9.9.9.9, 10.0.0.1"), None, &[]),
            "9.9.9.9"
        );
    }

    #[test]
    fn falls_back_to_a_shared_bucket() {
        assert_eq!(client_key(&HeaderMap::new(), None, &[]), "unknown");
    }

    #[test]
    fn untrusted_peer_overrides_spoofed_headers() {
        let trusted = nets("10.0.0.0/8");
        assert_eq!(
            client_key(&headers("1.1.1.1"), peer("203.0.113.9"), &trusted),
            "203.0.113.9"
        );
    }

    #[test]
    fn untrusted_peer_ignores_headers_with_no_proxies_configured() {
        assert_eq!(
            client_key(&headers("1.1.1.1"), peer("203.0.113.9"), &[]),
            "203.0.113.9"
        );
    }

    #[test]
    fn trusted_peer_supplies_the_forwarded_client() {
        let trusted = nets("10.0.0.0/8");
        assert_eq!(
            client_key(&headers("198.51.100.7"), peer("10.0.0.5"), &trusted),
            "198.51.100.7"
        );
    }

    #[test]
    fn multi_hop_chain_skips_proxies_right_to_left() {
        let trusted = nets("10.0.0.0/8, 127.0.0.1");
        assert_eq!(
            client_key(
                &headers("1.1.1.1, 198.51.100.7, 10.0.0.9"),
                peer("10.0.0.5"),
                &trusted
            ),
            "198.51.100.7",
            "leftmost entries are attacker-controlled"
        );
    }

    #[test]
    fn trusted_peer_falls_back_when_chain_is_all_proxies() {
        let trusted = nets("10.0.0.0/8");
        assert_eq!(
            client_key(&headers("10.0.0.9, 10.0.0.10"), peer("10.0.0.5"), &trusted),
            "10.0.0.5"
        );
    }

    #[test]
    fn real_ip_honoured_only_from_a_trusted_peer() {
        let mut spoofed = HeaderMap::new();
        spoofed.insert("x-real-ip", HeaderValue::from_static("1.1.1.1"));
        let trusted = nets("10.0.0.0/8");

        assert_eq!(
            client_key(&spoofed, peer("203.0.113.9"), &trusted),
            "203.0.113.9"
        );
        assert_eq!(client_key(&spoofed, peer("10.0.0.5"), &trusted), "1.1.1.1");
    }

    #[test]
    fn ipv4_mapped_peer_matches_a_v4_proxy_range() {
        let trusted = nets("127.0.0.1");
        assert_eq!(
            client_key(
                &headers("198.51.100.7"),
                peer_at("[::ffff:127.0.0.1]:41234"),
                &trusted
            ),
            "198.51.100.7"
        );
    }
}
