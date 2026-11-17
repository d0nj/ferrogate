use std::collections::BTreeMap;

use super::{parse_models_json, ModelSelector};
use crate::{entities::provider, error::AppError};

pub(super) fn selector_candidates(selector: &ModelSelector) -> Vec<String> {
    let mut names = vec![selector.raw.clone()];
    if selector.base_model != selector.raw {
        names.push(selector.base_model.clone());
    }
    if let Some(hint) = &selector.provider_hint {
        let qualified = format!("{hint}/{}", selector.base_model);
        if !names.contains(&qualified) {
            names.push(qualified);
        }
    }
    names
}

pub(super) fn direct_provider_matches(
    provider: &provider::Model,
    selector: &ModelSelector,
    model: &str,
) -> bool {
    let declared = parse_models_json(&provider.models_json);
    match selector.provider_hint.as_deref() {
        Some(hint) => {
            provider_matches(provider, hint)
                && (declared.is_empty() || declared.iter().any(|candidate| candidate == model))
        }
        None => declared.iter().any(|candidate| candidate == model),
    }
}

pub(super) fn provider_matches(provider: &provider::Model, hint: &str) -> bool {
    provider.name.eq_ignore_ascii_case(hint) || provider.provider_type.eq_ignore_ascii_case(hint)
}

fn is_reserved_header(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "host"
            | "content-length"
            | "transfer-encoding"
            | "x-api-key"
            | "x-goog-api-key"
    )
}

pub fn parse_extra_headers(raw: &str) -> BTreeMap<String, String> {
    serde_json::from_str::<BTreeMap<String, String>>(raw)
        .map(|headers| {
            headers
                .into_iter()
                .filter(|(name, _)| !name.trim().is_empty())
                .filter(|(name, _)| {
                    if is_reserved_header(name) {
                        tracing::warn!(header = %name, "dropping reserved extra header");
                        return false;
                    }
                    true
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn not_found(alias: &str) -> AppError {
    AppError::NotFound(format!("no routing target found for model '{alias}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderKind;
    use crate::routing::RoutingTarget;

    fn row(name: &str, kind: &str, models: &[&str], zdr: bool) -> provider::Model {
        provider::Model {
            id: format!("prov-{name}"),
            name: name.to_string(),
            provider_type: kind.to_string(),
            base_url: "https://example.invalid".to_string(),
            api_dialect: "chat".to_string(),
            api_key: String::new(),
            models_json: serde_json::to_string(models).unwrap(),
            is_active: 1,
            timeout_ms: 1_000,
            created_at: String::new(),
            updated_at: String::new(),
            region: Some("eu".to_string()),
            zdr: i64::from(zdr),
            extra_headers_json: r#"{"x-team":"edge"}"#.to_string(),
        }
    }

    #[test]
    fn candidates_try_raw_then_bare_then_qualified() {
        let selector = ModelSelector::parse("openai/gpt-4o:zdr");
        assert_eq!(
            selector_candidates(&selector),
            vec!["openai/gpt-4o:zdr", "gpt-4o", "openai/gpt-4o"]
        );
        assert_eq!(
            selector_candidates(&ModelSelector::parse("gpt-4o")),
            vec!["gpt-4o"]
        );
    }

    #[test]
    fn direct_matching_honours_hint_and_empty_model_lists() {
        let named = row("fast-openai", "openai", &["gpt-4o"], false);
        let typed = row("other", "openai", &["gpt-4o"], false);
        let anywhere = row("gateway", "openai", &[], false);
        let anthropic = row("claude", "anthropic", &["gpt-4o"], false);

        let hinted = ModelSelector::parse("fast-openai/gpt-4o");
        assert!(direct_provider_matches(&named, &hinted, "gpt-4o"));
        assert!(!direct_provider_matches(&typed, &hinted, "gpt-4o"));
        let by_type = ModelSelector::parse("anthropic/gpt-4o");
        assert!(direct_provider_matches(&anthropic, &by_type, "gpt-4o"));
        assert!(!direct_provider_matches(&named, &by_type, "gpt-4o"));

        let open_ended = ModelSelector::parse("gateway/anything");
        assert!(direct_provider_matches(&anywhere, &open_ended, "anything"));
        assert!(direct_provider_matches(
            &named,
            &ModelSelector::parse("gpt-4o"),
            "gpt-4o"
        ));
    }

    #[test]
    fn provider_extras_survive_into_targets() {
        let provider = row("openai-eu", "openai", &["gpt-4o"], true);
        let target = RoutingTarget {
            provider_id: provider.id.clone(),
            provider_name: provider.name.clone(),
            kind: ProviderKind::OpenAi,
            dialect: crate::providers::ApiDialect::Chat,
            base_url: provider.base_url.clone(),
            api_key: String::new(),
            upstream_model: "gpt-4o".to_string(),
            timeout_ms: 1_000,
            rule_id: None,
            weight: 100,
            position: 0,
            zdr: provider.zdr != 0,
            region: provider.region.clone(),
            extra_headers: parse_extra_headers(&provider.extra_headers_json),
            selected_key_id: None,
            selected_key_name: None,
        };
        let endpoint = target.endpoint();

        assert!(endpoint.zdr);
        assert_eq!(endpoint.region.as_deref(), Some("eu"));
        assert_eq!(
            endpoint.extra_headers.get("x-team").map(String::as_str),
            Some("edge")
        );
        assert!(parse_extra_headers("not json").is_empty());
    }
}
