#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoutePreference {
    #[default]
    Default,
    Nitro,
    Floor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSelector {
    pub raw: String,
    pub base_model: String,
    pub provider_hint: Option<String>,
    pub mode: RoutePreference,
    pub require_zdr: bool,
}

impl ModelSelector {
    pub fn mode_strategy(&self) -> Option<super::TierStrategy> {
        match self.mode {
            RoutePreference::Nitro => Some(super::TierStrategy::Latency),
            RoutePreference::Floor => Some(super::TierStrategy::Cost),
            RoutePreference::Default => None,
        }
    }

    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        let mut rest = raw;
        let mut mode = RoutePreference::Default;
        let mut require_zdr = false;

        while let Some((head, modifier)) = rest.rsplit_once(':') {
            let modifier = modifier.trim();
            match modifier.to_ascii_lowercase().as_str() {
                "nitro" => {
                    if mode == RoutePreference::Default {
                        mode = RoutePreference::Nitro;
                    }
                }
                "floor" => {
                    if mode == RoutePreference::Default {
                        mode = RoutePreference::Floor;
                    }
                }
                "zdr" => require_zdr = true,
                _ => break,
            }
            rest = head.trim_end();
        }

        let (provider_hint, base_model) = match rest.split_once('/') {
            Some((provider, model)) if !provider.is_empty() && !model.is_empty() => {
                (Some(provider.to_string()), model.to_string())
            }
            _ => (None, rest.to_string()),
        };

        Self {
            raw: raw.to_string(),
            base_model,
            provider_hint,
            mode,
            require_zdr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_model_has_no_modifiers() {
        let selector = ModelSelector::parse("gpt-4o");
        assert_eq!(selector.base_model, "gpt-4o");
        assert_eq!(selector.provider_hint, None);
        assert_eq!(selector.mode, RoutePreference::Default);
        assert!(!selector.require_zdr);
    }

    #[test]
    fn prefix_and_suffixes_combine_in_any_order() {
        let selector = ModelSelector::parse("openai/gpt-4o:zdr:floor");
        assert_eq!(selector.provider_hint.as_deref(), Some("openai"));
        assert_eq!(selector.base_model, "gpt-4o");
        assert_eq!(selector.mode, RoutePreference::Floor);
        assert!(selector.require_zdr);

        let selector = ModelSelector::parse("anthropic/claude-3-5:floor:nitro");
        assert_eq!(selector.provider_hint.as_deref(), Some("anthropic"));
        assert_eq!(selector.base_model, "claude-3-5");
        assert_eq!(selector.mode, RoutePreference::Nitro);
        assert!(!selector.require_zdr);
    }

    #[test]
    fn nitro_needs_no_provider_prefix() {
        let selector = ModelSelector::parse("gpt-4o:nitro");
        assert_eq!(selector.provider_hint, None);
        assert_eq!(selector.base_model, "gpt-4o");
        assert_eq!(selector.mode, RoutePreference::Nitro);
    }

    #[test]
    fn unknown_colon_segments_stay_in_the_model() {
        let selector = ModelSelector::parse("vendor:model-7b:zdr");
        assert_eq!(selector.base_model, "vendor:model-7b");
        assert!(selector.require_zdr);
    }

    #[test]
    fn prefix_requires_both_sides() {
        assert_eq!(ModelSelector::parse("openai/").base_model, "openai/");
        assert_eq!(ModelSelector::parse("openai/").provider_hint, None);
        assert_eq!(ModelSelector::parse("/gpt-4o").provider_hint, None);
        assert_eq!(ModelSelector::parse("/gpt-4o").base_model, "/gpt-4o");
        assert_eq!(
            ModelSelector::parse("meta-llama/llama-3-70b")
                .provider_hint
                .as_deref(),
            Some("meta-llama")
        );
    }

    #[test]
    fn raw_is_preserved_verbatim() {
        assert_eq!(
            ModelSelector::parse("  openai/gpt-4o:zdr ").raw,
            "openai/gpt-4o:zdr"
        );
    }
}
