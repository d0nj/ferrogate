use axum::http::Method;

use crate::auth::role::Capability;

use super::auth::LOGIN_PATH;

const ADMIN_PREFIX: &str = "/admin/api";

type RouteMatcher = fn(&Method, &str, &[&str]) -> bool;

type RouteRule = (RouteMatcher, Capability);

pub(super) fn required_capability(method: &Method, path: &str) -> Option<Capability> {
    use Capability::*;

    if path == LOGIN_PATH {
        return None;
    }

    let segments: Vec<&str> = path
        .strip_prefix(ADMIN_PREFIX)?
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let (head, tail) = segments.split_first()?;

    let rules: &[RouteRule] = &[
        (|m, h, _| h == "overview" && m == Method::GET, ReadOverview),
        (|m, h, _| h == "logs" && m == Method::GET, ReadLogs),
        (|m, h, _| h == "logs" && m == Method::DELETE, ClearLogs),
        (|_, h, _| h == "audit", ReadLogs),
        (
            |m, h, t| h == "session" && t.is_empty() && (m == Method::GET || m == Method::DELETE),
            ReadConfig,
        ),
        (
            |m, h, t| h == "response-cache" && m == Method::GET && t == ["stats"],
            ReadConfig,
        ),
        (|m, h, _| h == "settings" && m == Method::GET, ReadConfig),
        (
            |m, h, t| h == "pricing" && t.is_empty() && m == Method::GET,
            ReadConfig,
        ),
        (|_, h, t| h == "pricing" && !t.is_empty(), ManagePricing),
        (
            |m, h, _| h == "settings" && m == Method::PUT,
            ChangeSettings,
        ),
        (
            |m, h, t| h == "response-cache" && m == Method::DELETE && t.is_empty(),
            ClearCache,
        ),
        (|_, h, _| matches!(h, "keys" | "teams"), ManageKeys),
        (
            |_, h, _| matches!(h, "semantic" | "semantic-cache"),
            ClearCache,
        ),
        (
            |_, h, _| matches!(h, "providers" | "provider_ops"),
            ManageProviders,
        ),
        (|_, h, _| h == "routing", ManageRouting),
        (|_, h, _| h == "routes", ManageRouting),
        (|_, h, _| h == "guardrails", ManageGuardrails),
        (|_, h, _| h == "webhooks", ManageWebhooks),
        (|_, h, _| h == "prompts", ManagePrompts),
        (|_, h, _| h == "mcp", ManageMcp),
        (
            |m, h, t| h == "users" && m == Method::GET && t == ["me"],
            ReadConfig,
        ),
        (
            |m, h, t| h == "users" && m == Method::POST && t == ["me", "password"],
            ReadConfig,
        ),
        (|_, h, _| h == "users", ManageUsers),
    ];

    rules
        .iter()
        .find(|(matches, _)| matches(method, head, tail))
        .map(|(_, capability)| *capability)
        .or(Some(Capability::ManageUsers))
}

pub(super) fn is_mutating(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

pub(super) fn is_session_route(path: &str) -> bool {
    path == LOGIN_PATH || path == "/admin/api/session/"
}

pub(super) fn audit_action(method: &Method, path: &str) -> String {
    format!("{} {path}", method.as_str().to_lowercase())
}
