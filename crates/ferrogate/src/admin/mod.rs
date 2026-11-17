pub mod audit;
pub mod auth;
pub mod cache;
mod csrf;
pub mod guardrails;
pub mod keys;
pub mod logs;
pub mod mcp;
pub mod overview;
mod policy;
pub mod prompts;
pub mod provider_keys;
pub mod provider_ops;
pub mod providers;
pub mod routes;
pub mod routing;
pub mod semantic;
pub mod settings;
pub mod users;
pub mod webhooks;

use chrono::{SecondsFormat, Utc};

use crate::error::{AppError, AppResult};
use crate::AppState;

pub fn router() -> axum::Router<AppState> {
    use axum::routing::{delete, get, patch, post, put};

    axum::Router::new()
        .route("/admin/api/overview", get(overview::overview))
        .route(
            "/admin/api/providers",
            get(providers::list).post(providers::create),
        )
        .route(
            "/admin/api/providers/{id}",
            patch(providers::update).delete(providers::remove),
        )
        .route(
            "/admin/api/providers/{id}/keys",
            get(provider_keys::list).post(provider_keys::create),
        )
        .route(
            "/admin/api/providers/{provider_id}/keys/{key_id}",
            patch(provider_keys::update).delete(provider_keys::remove),
        )
        .route("/admin/api/providers/{id}/test", post(provider_ops::test))
        .route(
            "/admin/api/providers/{id}/sync-models",
            post(provider_ops::sync_models),
        )
        .route(
            "/admin/api/routing/policy",
            get(routing::get_policy).put(routing::put_policy),
        )
        .route("/admin/api/routes", get(routes::list))
        .route(
            "/admin/api/routes/{alias}",
            get(routes::get).put(routes::put).delete(routes::remove),
        )
        .route("/admin/api/response-cache/stats", get(cache::stats))
        .route("/admin/api/response-cache", delete(cache::clear))
        .route("/admin/api/keys", get(keys::list).post(keys::create))
        .route("/admin/api/teams", get(keys::teams))
        .route("/admin/api/users", get(users::list).post(users::create))
        .route("/admin/api/users/me", get(users::me))
        .route(
            "/admin/api/users/me/password",
            post(users::change_own_password),
        )
        .route(
            "/admin/api/users/{id}",
            patch(users::update).delete(users::remove),
        )
        .route(
            "/admin/api/users/{id}/password",
            post(users::reset_password),
        )
        .route("/admin/api/session/login", post(auth::login))
        .route(
            "/admin/api/session",
            get(auth::session).delete(auth::logout),
        )
        .route(
            "/admin/api/keys/{id}",
            patch(keys::update).delete(keys::remove),
        )
        .route("/admin/api/logs", get(logs::list))
        .route(
            "/admin/api/logs/{id}",
            get(logs::detail).delete(logs::remove),
        )
        .route("/admin/api/pricing", get(settings::get_pricing))
        .route("/admin/api/pricing/{model}", put(settings::put_pricing))
        .route(
            "/admin/api/settings",
            get(settings::get_settings).put(settings::put_settings),
        )
        .merge(guardrails::router())
        .merge(mcp::router())
        .merge(semantic::router())
        .merge(prompts::router())
        .merge(webhooks::router())
        .merge(audit::router())
}

pub(crate) fn seal(state: &AppState, secret: &str) -> AppResult<String> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    state
        .secrets
        .seal(trimmed)
        .map_err(|err| AppError::Internal(err.to_string()))
}

pub(crate) fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn mask_secret(secret: &str) -> String {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if crate::crypto::SecretBox::is_sealed(trimmed) {
        return "••••••••".to_string();
    }

    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 8 {
        return "•".repeat(8);
    }

    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}••••{tail}")
}

pub(crate) fn parse_string_list(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

pub(crate) fn encode_string_list(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_long_secrets() {
        assert_eq!(mask_secret("sk-abcdefghijklmnop"), "sk-a••••mnop");
    }

    #[test]
    fn masks_short_and_empty_secrets() {
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("short"), "••••••••");
    }

    #[test]
    fn round_trips_string_lists() {
        let values = vec!["gpt-4o".to_string(), "claude-3-5".to_string()];
        assert_eq!(parse_string_list(&encode_string_list(&values)), values);
        assert!(parse_string_list("not json").is_empty());
    }
}
