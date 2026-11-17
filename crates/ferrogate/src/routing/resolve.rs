use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use super::targets::{direct_provider_matches, not_found, provider_matches, selector_candidates};
use super::{ModelSelector, Provenance, RoutingService, RoutingTarget};
use crate::{
    entities::{provider, routing_rule},
    error::AppResult,
};

impl RoutingService {
    pub(super) async fn resolve_targets_inner(
        &self,
        selector: &ModelSelector,
    ) -> AppResult<Vec<RoutingTarget>> {
        let found = self.lookup_targets(selector).await?;
        found.ok_or_else(|| not_found(&selector.raw))
    }

    pub(super) async fn lookup_targets(
        &self,
        selector: &ModelSelector,
    ) -> AppResult<Option<Vec<RoutingTarget>>> {
        for name in selector_candidates(selector) {
            let mut targets = self.load_targets(selector, &name).await?;
            if targets.is_empty() {
                targets = self.load_direct_targets(selector, &name).await?;
            }
            if !targets.is_empty() {
                return Ok(Some(targets));
            }
        }
        Ok(None)
    }

    pub(super) async fn load_targets(
        &self,
        selector: &ModelSelector,
        alias: &str,
    ) -> AppResult<Vec<RoutingTarget>> {
        let rows = routing_rule::Entity::find()
            .find_also_related(provider::Entity)
            .filter(routing_rule::Column::Alias.eq(alias))
            .filter(routing_rule::Column::IsActive.eq(1))
            .filter(provider::Column::IsActive.eq(1))
            .order_by_asc(routing_rule::Column::Position)
            .all(&self.db)
            .await?;

        let direct_request = alias != selector.raw;
        let hint = selector.provider_hint.as_deref();

        let candidates = rows
            .into_iter()
            .filter_map(|(rule, provider)| {
                let provider = provider?;
                let targets_hint = rule.provider_id == alias
                    || hint.is_some_and(|hint| provider_matches(&provider, hint));
                if direct_request && hint.is_some() && !targets_hint {
                    return None;
                }
                Some(Provenance {
                    rule_id: rule.id,
                    upstream_model: rule.upstream_model,
                    weight: rule.weight,
                    position: rule.position,
                    provider,
                })
            })
            .collect();

        Ok(self.build_targets(candidates))
    }

    pub(super) async fn load_direct_targets(
        &self,
        selector: &ModelSelector,
        model: &str,
    ) -> AppResult<Vec<RoutingTarget>> {
        let providers = provider::Entity::find()
            .filter(provider::Column::IsActive.eq(1))
            .order_by_asc(provider::Column::Name)
            .all(&self.db)
            .await?;

        let candidates = providers
            .into_iter()
            .filter(|provider| direct_provider_matches(provider, selector, model))
            .map(|provider| Provenance {
                rule_id: String::new(),
                upstream_model: model.to_string(),
                weight: 100,
                position: 0,
                provider,
            })
            .collect();

        Ok(self.build_targets(candidates))
    }
}
