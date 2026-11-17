use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{auth::session::Principal, entities::audit_log, error::AppResult, AppState};

use super::now_rfc3339;

const MAX_ENTRIES: u64 = 200;

pub const OUTCOME_ALLOW: &str = "allow";
pub const OUTCOME_DENY: &str = "deny";

pub enum Actor<'a> {
    Principal(&'a Principal),
    Anonymous(&'a str),
}

pub async fn record<C: ConnectionTrait>(
    db: &C,
    actor: Actor<'_>,
    action: &str,
    target: Option<&str>,
    outcome: &str,
    detail: Option<&str>,
) -> AppResult<()> {
    let (actor_id, actor_label) = match actor {
        Actor::Principal(principal) => (
            Some(principal.user_id.clone()),
            principal.label().to_string(),
        ),
        Actor::Anonymous(label) => (None, label.to_string()),
    };

    let row = audit_log::ActiveModel {
        id: sea_orm::Set(format!("audit-{}", uuid::Uuid::new_v4())),
        created_at: sea_orm::Set(now_rfc3339()),
        actor_id: sea_orm::Set(actor_id),
        actor_label: sea_orm::Set(actor_label),
        action: sea_orm::Set(action.to_string()),
        target: sea_orm::Set(target.map(str::to_string)),
        outcome: sea_orm::Set(outcome.to_string()),
        detail: sea_orm::Set(detail.map(str::to_string)),
    };

    audit_log::Entity::insert(row).exec(db).await?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct AuditEntryDto {
    pub id: String,
    pub created_at: String,
    pub actor_id: Option<String>,
    pub actor_label: String,
    pub action: String,
    pub target: Option<String>,
    pub outcome: String,
    pub detail: Option<String>,
}

impl From<audit_log::Model> for AuditEntryDto {
    fn from(row: audit_log::Model) -> Self {
        Self {
            id: row.id,
            created_at: row.created_at,
            actor_id: row.actor_id,
            actor_label: row.actor_label,
            action: row.action,
            target: row.target,
            outcome: row.outcome,
            detail: row.detail,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct AuditQuery {
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<AuditQuery>,
) -> AppResult<Json<Value>> {
    let mut select = audit_log::Entity::find();
    if let Some(actor) = non_blank(query.actor.as_deref()) {
        select = select.filter(
            sea_orm::Condition::any()
                .add(audit_log::Column::ActorId.eq(actor))
                .add(audit_log::Column::ActorLabel.eq(actor)),
        );
    }
    if let Some(action) = non_blank(query.action.as_deref()) {
        select = select.filter(audit_log::Column::Action.eq(action));
    }
    if let Some(outcome) = non_blank(query.outcome.as_deref()) {
        select = select.filter(audit_log::Column::Outcome.eq(outcome));
    }

    let rows = select
        .order_by_desc(audit_log::Column::CreatedAt)
        .limit(MAX_ENTRIES)
        .all(&state.db)
        .await?;

    let total = rows.len();
    let entries: Vec<AuditEntryDto> = rows.into_iter().map(AuditEntryDto::from).collect();

    Ok(Json(json!({ "entries": entries, "total": total })))
}

fn non_blank(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
}

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/api/audit", get(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_filters_are_ignored() {
        assert_eq!(non_blank(Some("  ")), None);
        assert_eq!(non_blank(None), None);
        assert_eq!(non_blank(Some(" deny ")), Some("deny"));
    }

    #[test]
    fn dto_carries_every_column() {
        let row = audit_log::Model {
            id: "audit-1".into(),
            created_at: "2026-01-01T00:00:00.000Z".into(),
            actor_id: Some("user-1".into()),
            actor_label: "ops@example.com".into(),
            action: "post /admin/api/keys".into(),
            target: Some("/admin/api/keys".into()),
            outcome: OUTCOME_ALLOW.into(),
            detail: None,
        };

        let dto = AuditEntryDto::from(row);
        assert_eq!(dto.actor_id.as_deref(), Some("user-1"));
        assert_eq!(dto.outcome, "allow");
        assert_eq!(dto.target.as_deref(), Some("/admin/api/keys"));
    }
}
