use moka::future::Cache;

use axum::http::{HeaderMap, HeaderName};

use super::RoutingTarget;

const SESSION_HEADERS: [&str; 7] = [
    "session-id",
    "x-bf-session-id",
    "x-session-id",
    "x-ferrogate-session-id",
    "x-claude-code-session-id",
    "x-session-affinity",
    "thread-id",
];

const AFFINITY_SWITCH: &str = "x-ferrogate-session-affinity";

const MAX_SESSION_ID_LEN: usize = 255;

pub const AFFINITY_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

pub const AFFINITY_CAPACITY: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionBinding {
    pub provider_id: String,
    pub key_id: Option<String>,
}

pub fn extract_session_id(headers: &HeaderMap) -> Option<String> {
    for switch in [AFFINITY_SWITCH, "x-session-affinity-enabled"] {
        if let Some(value) = header_text(headers, switch) {
            if is_off(value) {
                return None;
            }
        }
    }

    for name in SESSION_HEADERS {
        if let Some(value) = header_text(headers, name) {
            if let Some(id) = usable_session_id(value) {
                return Some(id);
            }
        }
    }
    None
}

fn usable_session_id(value: &str) -> Option<String> {
    let value = value.trim();
    let too_long = value.len() > MAX_SESSION_ID_LEN;
    if too_long {
        return None;
    }
    Some(value.to_string())
}
fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let name = HeaderName::from_bytes(name.as_bytes()).ok()?;
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn is_off(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "off" | "false" | "0" | "none" | "disabled"
    )
}

fn sanitize_redis_key_segment(segment: &str) -> String {
    segment
        .trim()
        .chars()
        .map(|ch| match ch {
            ':' | ' ' | '\t' | '\n' | '\r' => '_',
            other => other,
        })
        .collect()
}

pub struct SessionAffinity {
    bindings: Cache<String, SessionBinding>,
}

impl Default for SessionAffinity {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionAffinity {
    pub fn new() -> Self {
        Self::with_capacity(AFFINITY_CAPACITY)
    }

    pub fn with_capacity(capacity: u64) -> Self {
        Self {
            bindings: Cache::builder()
                .time_to_live(AFFINITY_TTL)
                .max_capacity(capacity)
                .build(),
        }
    }

    fn scope_key(scope: &str, session_id: &str, model: &str) -> String {
        format!("{scope}\u{1f}{session_id}\u{1f}{model}")
    }

    pub fn tenant_scope(base: &str, key_id: Option<&str>) -> String {
        match key_id.filter(|id| !id.is_empty()) {
            Some(id) => format!("{base}\u{1f}key:{id}"),
            None => format!("{base}\u{1f}key:anon"),
        }
    }

    fn binding_key(scope: &str, session_id: &str, model: &str) -> String {
        let scope = sanitize_redis_key_segment(scope);
        let session_id = sanitize_redis_key_segment(session_id);
        let model = sanitize_redis_key_segment(model);
        format!("fg:aff:{scope}:{session_id}:{model}")
    }

    pub async fn get(&self, scope: &str, session_id: &str, model: &str) -> Option<SessionBinding> {
        self.bindings
            .get(&Self::scope_key(scope, session_id, model))
            .await
    }

    pub async fn set(&self, scope: &str, session_id: &str, model: &str, binding: SessionBinding) {
        self.bindings
            .insert(Self::scope_key(scope, session_id, model), binding)
            .await;
    }

    pub async fn invalidate(&self, scope: &str, session_id: &str, model: &str) {
        self.bindings
            .invalidate(&Self::scope_key(scope, session_id, model))
            .await;
    }

    pub fn len(&self) -> u64 {
        self.bindings.entry_count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn redis_key(scope: &str, session_id: &str, model: &str) -> String {
        Self::binding_key(scope, session_id, model)
    }

    pub async fn set_with_redis(
        &self,
        redis: Option<&crate::infra::redis::RedisStore>,
        scope: &str,
        session_id: &str,
        model: &str,
        binding: &SessionBinding,
    ) {
        self.set(scope, session_id, model, binding.clone()).await;
        if let Some(redis) = redis {
            redis
                .set_json(
                    &Self::binding_key(scope, session_id, model),
                    binding,
                    AFFINITY_TTL.as_secs(),
                )
                .await;
        }
    }

    pub async fn get_with_redis(
        &self,
        redis: Option<&crate::infra::redis::RedisStore>,
        scope: &str,
        session_id: &str,
        model: &str,
    ) -> Option<SessionBinding> {
        if let Some(binding) = self.get(scope, session_id, model).await {
            return Some(binding);
        }
        let binding: SessionBinding = redis?
            .get_json(&Self::binding_key(scope, session_id, model))
            .await?;
        self.set(scope, session_id, model, binding.clone()).await;
        Some(binding)
    }

    pub async fn invalidate_with_redis(
        &self,
        redis: Option<&crate::infra::redis::RedisStore>,
        scope: &str,
        session_id: &str,
        model: &str,
    ) {
        self.invalidate(scope, session_id, model).await;
        if let Some(redis) = redis {
            redis
                .delete_prefix(&Self::binding_key(scope, session_id, model))
                .await;
        }
    }

    pub fn promote_sticky(targets: &mut Vec<RoutingTarget>, binding: &SessionBinding) {
        let Some(head_model) = targets.first().map(|target| target.upstream_model.clone()) else {
            return;
        };
        let Some(index) = targets.iter().position(|target| {
            target.provider_id == binding.provider_id && target.upstream_model == head_model
        }) else {
            return;
        };
        if index == 0 {
            return;
        }
        let target = targets.remove(index);
        targets.insert(0, target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderKind;
    use axum::http::HeaderValue;

    fn target(provider_id: &str) -> RoutingTarget {
        RoutingTarget {
            provider_id: provider_id.to_string(),
            provider_name: provider_id.to_string(),
            kind: ProviderKind::Anthropic,
            dialect: crate::providers::ApiDialect::Chat,
            base_url: "https://example.test".to_string(),
            api_key: "sk-test".to_string(),
            upstream_model: "claude-sonnet-4".to_string(),
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

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn reads_known_session_headers() {
        let map = headers(&[("x-claude-code-session-id", "abc123")]);
        assert_eq!(extract_session_id(&map).as_deref(), Some("abc123"));
    }

    #[test]
    fn first_known_header_wins() {
        let map = headers(&[("thread-id", "thread"), ("x-bf-session-id", "bf")]);
        assert_eq!(extract_session_id(&map).as_deref(), Some("bf"));
    }

    #[test]
    fn off_switch_disables_affinity() {
        let map = headers(&[
            ("x-ferrogate-session-id", "abc"),
            ("x-ferrogate-session-affinity", "off"),
        ]);
        assert_eq!(extract_session_id(&map), None);
    }

    #[test]
    fn overlong_ids_are_rejected() {
        let map = headers(&[
            ("x-session-id", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ("session-id", "short"),
        ]);
        assert_eq!(extract_session_id(&map).as_deref(), Some("short"));
    }

    #[tokio::test]
    async fn bindings_are_scoped_per_model() {
        let affinity = SessionAffinity::new();
        let binding = SessionBinding {
            provider_id: "p1".to_string(),
            key_id: Some("k1".to_string()),
        };
        affinity
            .set("tenant", "s1", "claude", binding.clone())
            .await;

        assert_eq!(
            affinity.get("tenant", "s1", "claude").await,
            Some(binding.clone())
        );
        assert_eq!(affinity.get("tenant", "s1", "gpt-4o").await, None);
        assert_eq!(affinity.get("other", "s1", "claude").await, None);

        affinity.invalidate("tenant", "s1", "claude").await;
        assert_eq!(affinity.get("tenant", "s1", "claude").await, None);
    }

    #[test]
    fn promote_moves_bound_provider_to_front() {
        let binding = SessionBinding {
            provider_id: "p2".to_string(),
            key_id: None,
        };
        let mut targets = vec![target("p1"), target("p2"), target("p3")];
        SessionAffinity::promote_sticky(&mut targets, &binding);
        assert_eq!(targets[0].provider_id, "p2");
        assert_eq!(targets[1].provider_id, "p1");
    }

    #[test]
    fn promote_ignores_unknown_provider() {
        let binding = SessionBinding {
            provider_id: "missing".to_string(),
            key_id: None,
        };
        let mut targets = vec![target("p1"), target("p2")];
        SessionAffinity::promote_sticky(&mut targets, &binding);
        assert_eq!(targets[0].provider_id, "p1");
    }
}
