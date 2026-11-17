use std::collections::BTreeSet;

use sea_orm::{sea_query::Order, ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use super::{parse_models_json, ProviderHealthSnapshot, RoutingService};
use crate::{
    entities::{provider, routing_rule},
    error::AppResult,
};

impl RoutingService {
    pub async fn list_available_models(&self) -> AppResult<Vec<String>> {
        let aliases: Vec<String> = routing_rule::Entity::find()
            .find_also_related(provider::Entity)
            .filter(routing_rule::Column::IsActive.eq(1))
            .filter(provider::Column::IsActive.eq(1))
            .order_by(routing_rule::Column::Alias, Order::Asc)
            .all(&self.db)
            .await?
            .into_iter()
            .map(|(rule, _provider)| rule.alias)
            .collect();

        let providers = provider::Entity::find()
            .filter(provider::Column::IsActive.eq(1))
            .order_by_asc(provider::Column::Name)
            .all(&self.db)
            .await?;

        let mut seen: BTreeSet<String> = aliases.into_iter().collect();
        for provider in &providers {
            seen.extend(parse_models_json(&provider.models_json));
        }

        Ok(seen.into_iter().collect())
    }

    pub async fn health_snapshots(&self) -> AppResult<Vec<ProviderHealthSnapshot>> {
        let providers = provider::Entity::find()
            .order_by_asc(provider::Column::Name)
            .all(&self.db)
            .await?;
        Ok(self.circuit.snapshots(&providers))
    }
}
