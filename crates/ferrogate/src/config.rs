use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use ipnet::IpNet;

pub const DEFAULT_PORT: u16 = 8080;
pub const DEFAULT_DATABASE_URL: &str = "sqlite://ferrogate.db?mode=rwc";

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub host: IpAddr,
    pub port: u16,
    pub database_url: String,
    pub log_filter: String,
    pub redis_url: Option<String>,
    pub qdrant_url: Option<String>,
    pub semantic_embedding_model: Option<String>,
    pub otlp_endpoint: Option<String>,
    pub cors_origins: Vec<String>,
    pub log_retention_days: Option<u64>,
    pub trusted_proxies: Vec<IpNet>,
    pub allow_private_upstreams: bool,
    pub cookie_secure: Option<bool>,
    pub health_detailed: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: DEFAULT_PORT,
            database_url: DEFAULT_DATABASE_URL.to_string(),
            log_filter: "ferrogate=debug,tower_http=debug,info".to_string(),
            redis_url: None,
            qdrant_url: None,
            semantic_embedding_model: None,
            otlp_endpoint: None,
            cors_origins: Vec::new(),
            log_retention_days: None,
            trusted_proxies: Vec::new(),
            allow_private_upstreams: false,
            cookie_secure: None,
            health_detailed: false,
        }
    }
}

impl AppConfig {
    pub fn from_env() -> Self {
        let defaults = Self::default();

        let host = std::env::var("HOST")
            .ok()
            .and_then(|raw| raw.parse::<IpAddr>().ok())
            .unwrap_or(defaults.host);

        let port = std::env::var("PORT")
            .ok()
            .and_then(|raw| raw.parse::<u16>().ok())
            .unwrap_or(defaults.port);

        let database_url = non_empty("DATABASE_URL").unwrap_or(defaults.database_url);

        let log_filter = non_empty("RUST_LOG").unwrap_or(defaults.log_filter);

        Self {
            host,
            port,
            database_url,
            log_filter,
            redis_url: non_empty("REDIS_URL"),
            qdrant_url: non_empty("QDRANT_URL"),
            semantic_embedding_model: non_empty("SEMANTIC_EMBEDDING_MODEL"),
            otlp_endpoint: non_empty("OTEL_EXPORTER_OTLP_ENDPOINT"),
            cors_origins: parse_origins("CORS_ALLOWED_ORIGINS"),
            log_retention_days: match non_empty("LOG_RETENTION_DAYS") {
                Some(raw) => raw.parse::<u64>().ok(),
                None => Some(90),
            },
            trusted_proxies: parse_trusted_proxies("TRUSTED_PROXIES"),
            allow_private_upstreams: flag("ALLOW_PRIVATE_UPSTREAMS"),
            cookie_secure: opt_flag("COOKIE_SECURE"),
            health_detailed: flag("HEALTH_DETAILED"),
        }
    }

    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

fn non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn flag(name: &str) -> bool {
    matches!(
        non_empty(name)
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

fn opt_flag(name: &str) -> Option<bool> {
    non_empty(name).map(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}
fn parse_origins(name: &str) -> Vec<String> {
    non_empty(name)
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|origin| !origin.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_trusted_proxies(name: &str) -> Vec<IpNet> {
    non_empty(name)
        .map(|raw| parse_net_list(&raw))
        .unwrap_or_default()
}

pub(crate) fn parse_net_list(raw: &str) -> Vec<IpNet> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| match parse_net(entry) {
            Some(net) => Some(net),
            None => {
                tracing::warn!(entry, "ignoring unparsable trusted proxy");
                None
            }
        })
        .collect()
}

fn parse_net(entry: &str) -> Option<IpNet> {
    match entry.parse::<IpNet>() {
        Ok(net) => Some(net),
        Err(_) => entry.parse::<IpAddr>().ok().map(IpNet::from),
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{parse_net, parse_net_list, parse_trusted_proxies};

    fn contains(nets: &[ipnet::IpNet], addr: &str) -> bool {
        let ip: IpAddr = addr.parse().unwrap();
        nets.iter().any(|net| net.contains(&ip))
    }

    #[test]
    fn parses_mixed_proxy_lists() {
        let nets = parse_net_list("10.0.0.0/8, 127.0.0.1 , ::1,bogus");
        assert_eq!(nets.len(), 3, "malformed entry dropped: {nets:?}");
        assert!(contains(&nets, "10.1.2.3"));
        assert!(contains(&nets, "127.0.0.1"));
        assert!(contains(&nets, "::1"));
    }

    #[test]
    fn bare_ip_becomes_a_host_route() {
        let net = parse_net("192.0.2.7").unwrap();
        assert_eq!(net.prefix_len(), 32);
        assert!(parse_net("not-an-address").is_none());
    }

    #[test]
    fn unset_means_trust_nothing() {
        assert!(parse_trusted_proxies("FG_TEST_PROXIES_UNSET").is_empty());
    }
}
