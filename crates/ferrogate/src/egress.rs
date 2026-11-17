use std::net::IpAddr;

use crate::error::{AppError, AppResult};

const MAX_URL_LEN: usize = 2048;

pub async fn validate_url(raw: &str, allow_private: bool) -> AppResult<String> {
    let trimmed = validate_strict_no_dns(raw, allow_private)?;
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("url must not be empty".into()));
    }
    if trimmed.len() > MAX_URL_LEN {
        return Err(AppError::BadRequest("url is too long".into()));
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(AppError::BadRequest(
            "url must start with http:// or https://".into(),
        ));
    }

    let parsed: reqwest::Url = trimmed
        .parse()
        .map_err(|_| AppError::BadRequest("url is not a valid http(s) URL".into()))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::BadRequest(
            "url must not embed credentials".into(),
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::BadRequest("url must contain a host".into()))?;

    reject_special_hostname(host)?;
    if host.trim_matches(['[', ']']).parse::<IpAddr>().is_ok() {
        return Ok(trimmed);
    }

    let port = parsed.port_or_known_default().unwrap_or(80);
    match tokio::net::lookup_host((host, port)).await {
        Ok(addrs) => {
            let mut any = false;
            for addr in addrs {
                any = true;
                if reject_blocked_ip(addr.ip(), allow_private).is_err() {
                    return Err(AppError::BadRequest(format!(
                        "url resolves to a blocked address ({})",
                        addr.ip()
                    )));
                }
            }
            if !any {
                return Err(AppError::BadRequest("url resolves to no address".into()));
            }
        }
        Err(err) => {
            tracing::warn!(url = %redacted_host(&parsed), error = %err, "egress DNS lookup failed; accepting without re-check")
        }
    }

    Ok(trimmed)
}

pub fn validate_strict_no_dns(raw: &str, allow_private: bool) -> AppResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("url must not be empty".into()));
    }
    if trimmed.len() > MAX_URL_LEN {
        return Err(AppError::BadRequest("url is too long".into()));
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(AppError::BadRequest(
            "url must start with http:// or https://".into(),
        ));
    }

    let parsed: reqwest::Url = trimmed
        .parse()
        .map_err(|_| AppError::BadRequest("url is not a valid http(s) URL".into()))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::BadRequest(
            "url must not embed credentials".into(),
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::BadRequest("url must contain a host".into()))?;

    reject_special_hostname(host)?;
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        reject_blocked_ip(ip, allow_private)
            .map_err(|_| AppError::BadRequest(format!("url target '{host}' is not permitted")))?;
    }
    Ok(trimmed.to_string())
}

fn reject_special_hostname(host: &str) -> AppResult<()> {
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") || lower == "metadata.google.internal"
    {
        return Err(AppError::BadRequest(format!(
            "url host '{host}' is not a valid fetch target"
        )));
    }
    Ok(())
}

fn reject_blocked_ip(ip: IpAddr, allow_private: bool) -> Result<(), ()> {
    if is_always_blocked(ip) {
        return Err(());
    }
    if !allow_private && is_private(ip) {
        return Err(());
    }
    Ok(())
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || ip_in(ip, 100, 64, 0, 0, 10)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unicast_link_local()
                || (u128::from_be_bytes(v6.octets()) & !((1 << 121) - 1)
                    == 0xfc00_0000_0000_0000_0000_0000_0000_0000)
        }
    }
}

fn is_always_blocked(ip: IpAddr) -> bool {
    if ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            v4.is_link_local()
                || v4.is_broadcast()
                || is_v4_documentation(&v4)
                || ip_in(ip, 100, 64, 0, 0, 10)
        }
        IpAddr::V6(v6) => {
            v6.is_unicast_link_local() || ip6_in(ip, 0x2001_0db8_0000_0000_0000_0000_0000_0000, 32)
        }
    }
}

fn is_v4_documentation(v4: &std::net::Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    (a == 192 && b == 0 && v4.octets()[2] == 2)
        || (a == 198 && b == 51 && v4.octets()[2] == 100)
        || (a == 203 && b == 0 && v4.octets()[2] == 113)
}

fn ip_in(ip: IpAddr, a: u8, b: u8, c: u8, d: u8, bits: u32) -> bool {
    let IpAddr::V4(v4) = ip else {
        return false;
    };
    let net = u32::from_be_bytes([a, b, c, d]);
    let mask = match bits {
        0 => 0,
        _ => u32::MAX << (32 - bits),
    };
    u32::from_be_bytes(v4.octets()) & mask == net & mask
}

fn ip6_in(ip: IpAddr, prefix: u128, bits: u32) -> bool {
    let IpAddr::V6(v6) = ip else {
        return false;
    };
    let mask = match bits {
        0 => 0,
        _ => u128::MAX << (128 - bits),
    };
    u128::from_be_bytes(v6.octets()) & mask == prefix & mask
}

fn redacted_host(url: &reqwest::Url) -> String {
    url.host_str().unwrap_or("?").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn ok(url: &str, allow_private: bool) {
        assert!(
            validate_url(url, allow_private).await.is_ok(),
            "expected accept: {url}"
        );
    }

    async fn bad(url: &str, allow_private: bool) {
        assert!(
            validate_url(url, allow_private).await.is_err(),
            "expected reject: {url}"
        );
    }

    #[tokio::test]
    async fn rejects_non_http_and_userinfo() {
        bad("", false).await;
        bad("ftp://example.com/x", false).await;
        bad("https://user:pass@example.com/", false).await;
        bad(&"https://example.com/".to_string().repeat(200), false).await;
    }

    #[tokio::test]
    async fn rejects_loopback_metadata_and_special_names() {
        bad("http://127.0.0.1:11434/", false).await;
        bad("http://[::1]:11434/", false).await;
        bad("http://169.254.169.254/latest/meta-data/", false).await;
        bad("http://localhost:11434/", false).await;
        bad("http://foo.localhost/", false).await;
        bad("http://metadata.google.internal/", false).await;
        ok("http://127.0.0.1:11434/", true).await;
        bad("http://169.254.169.254/latest/meta-data/", true).await;
    }

    #[tokio::test]
    async fn rejects_private_unless_opted_in() {
        bad("http://192.168.1.10:11434/", false).await;
        bad("http://10.0.0.5/", false).await;
        ok("http://192.168.1.10:11434/", true).await;
        ok("https://api.openai.com/v1", false).await;
    }
}
