pub mod detect;
pub mod webhooks;

use std::{collections::HashSet, sync::Arc, time::Duration};

use moka::future::Cache;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use serde_json::Value;

use crate::{
    entities::guardrail_rule,
    error::{AppError, AppResult},
    providers::{message_content_to_text, ChatCompletionRequest},
};

use self::{detect::Match, webhooks::WebhookSender};

const RULES_TTL: Duration = Duration::from_secs(30);
const RULES_CAPACITY: u64 = 512;

const PHASE_INPUT: &str = "input";
const PHASE_OUTPUT: &str = "output";

const ACTION_BLOCK: &str = "block";
const ACTION_REDACT: &str = "redact";
const ACTION_FLAG: &str = "flag";

#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub phase: Phase,
    pub rule_type: String,
    pub pattern: Option<String>,
    pub action: Action,
    pub priority: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Input,
    Output,
}

impl Phase {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            PHASE_INPUT => Some(Self::Input),
            PHASE_OUTPUT => Some(Self::Output),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Block,
    Redact,
    Flag,
}

impl Action {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            ACTION_BLOCK => Some(Self::Block),
            ACTION_REDACT => Some(Self::Redact),
            ACTION_FLAG => Some(Self::Flag),
            _ => None,
        }
    }
}

impl Rule {
    pub fn from_model(row: guardrail_rule::Model) -> Option<Self> {
        if row.is_active == 0 {
            return None;
        }

        let Some(phase) = Phase::parse(&row.phase) else {
            tracing::warn!(rule = %row.id, phase = %row.phase, "unknown guardrail phase; skipping");
            return None;
        };
        let Some(action) = Action::parse(&row.action) else {
            tracing::warn!(rule = %row.id, action = %row.action, "unknown guardrail action; skipping");
            return None;
        };

        Some(Self {
            id: row.id,
            name: row.name,
            phase,
            rule_type: row.rule_type,
            pattern: row.pattern,
            action,
            priority: row.priority,
        })
    }

    fn hits(&self, text: &str) -> Vec<Match> {
        detect::detect(&self.rule_type, self.pattern.as_deref(), text)
    }
}

#[derive(Debug, Clone, Default)]
pub struct InputVerdict {
    pub flags: Vec<String>,
    pub redactions: Vec<String>,
    pub triggered: Vec<String>,
}

impl InputVerdict {
    pub fn is_clean(&self) -> bool {
        self.triggered.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct OutputVerdict {
    pub text: String,
    pub redacted: bool,
    pub triggered: Vec<String>,
}

pub struct GuardrailEngine {
    db: DatabaseConnection,
    rules: Cache<&'static str, Arc<Vec<Rule>>>,
    webhooks: WebhookSender,
}

impl GuardrailEngine {
    pub fn new(db: DatabaseConnection, webhooks: WebhookSender) -> Self {
        let rules = Cache::builder()
            .max_capacity(RULES_CAPACITY)
            .time_to_live(RULES_TTL)
            .build();

        Self {
            db,
            rules,
            webhooks,
        }
    }

    pub async fn invalidate(&self) {
        self.rules.invalidate_all();
    }

    pub fn emit(&self, event: webhooks::WebhookEvent) {
        self.webhooks.emit(event);
    }

    pub async fn check_input(
        &self,
        request: &mut ChatCompletionRequest,
        virtual_key_id: Option<&str>,
        endpoint: &str,
    ) -> AppResult<InputVerdict> {
        let rules = self.active_rules().await;
        let mut verdict = InputVerdict::default();

        for rule in rules.iter().filter(|rule| rule.phase == Phase::Input) {
            let mut touched = false;
            for message in &mut request.messages {
                let text = message_content_to_text(&message.content);
                if text.is_empty() {
                    continue;
                }

                let hits = rule.hits(&text);
                if hits.is_empty() {
                    continue;
                }

                touched = true;
                match rule.action {
                    Action::Block => {
                        self.note_rejection(rule, virtual_key_id, endpoint);
                        return Err(AppError::Forbidden(format!(
                            "blocked by guardrail '{}'",
                            rule.name
                        )));
                    }
                    Action::Redact => {
                        message.content = Value::String(detect::redact(&text, &hits));
                    }
                    Action::Flag => {}
                }
            }

            if !touched {
                continue;
            }

            verdict.triggered.push(rule.name.clone());
            match rule.action {
                Action::Redact => verdict.redactions.push(rule.name.clone()),
                Action::Flag => verdict.flags.push(rule.name.clone()),
                Action::Block => {}
            }
        }

        if !verdict.triggered.is_empty() {
            self.emit(triggered_event(
                &verdict.triggered,
                "input",
                virtual_key_id,
                endpoint,
            ));
        }

        Ok(verdict)
    }

    pub async fn filter_output(
        &self,
        text: &str,
        virtual_key_id: Option<&str>,
        endpoint: &str,
    ) -> OutputVerdict {
        let rules = self.active_rules().await;
        let verdict = apply_output_rules(&rules, text);

        if !verdict.triggered.is_empty() {
            self.emit(triggered_event(
                &verdict.triggered,
                "output",
                virtual_key_id,
                endpoint,
            ));
        }

        verdict
    }

    pub async fn filter_output_cached(
        &self,
        text: &str,
        virtual_key_id: Option<&str>,
        endpoint: &str,
    ) -> OutputVerdict {
        let rules = self.rules.get("active").await.unwrap_or_default();
        let verdict = apply_output_rules(&rules, text);

        if !verdict.triggered.is_empty() {
            self.emit(triggered_event(
                &verdict.triggered,
                "output",
                virtual_key_id,
                endpoint,
            ));
        }

        verdict
    }

    pub async fn has_output_rules(&self) -> bool {
        self.active_rules()
            .await
            .iter()
            .any(|rule| rule.phase == Phase::Output)
    }

    async fn active_rules(&self) -> Arc<Vec<Rule>> {
        self.rules
            .get_with("active", async { Arc::new(self.load_rules().await) })
            .await
    }

    async fn load_rules(&self) -> Vec<Rule> {
        let rows = match guardrail_rule::Entity::find()
            .filter(guardrail_rule::Column::IsActive.eq(1))
            .order_by_asc(guardrail_rule::Column::Priority)
            .all(&self.db)
            .await
        {
            Ok(rows) => rows,
            Err(err) => {
                tracing::warn!(error = %err, "failed to load guardrail rules; running unrestricted");
                return Vec::new();
            }
        };

        rows.into_iter().filter_map(Rule::from_model).collect()
    }

    fn note_rejection(&self, rule: &Rule, virtual_key_id: Option<&str>, endpoint: &str) {
        self.emit(webhooks::WebhookEvent::new(
            webhooks::EVENT_REQUEST_BLOCKED,
            serde_json::json!({
                "guardrail": rule.name,
                "rule_id": rule.id,
                "rule_type": rule.rule_type,
                "virtual_key_id": virtual_key_id,
                "endpoint": endpoint,
            }),
        ));
        self.emit(triggered_event(
            std::slice::from_ref(&rule.name),
            "input",
            virtual_key_id,
            endpoint,
        ));
    }
}

fn apply_output_rules(rules: &[Rule], text: &str) -> OutputVerdict {
    let mut out = text.to_string();
    let mut triggered = Vec::new();
    let mut redacted = false;

    for rule in rules.iter().filter(|rule| rule.phase == Phase::Output) {
        let hits = rule.hits(&out);
        if hits.is_empty() {
            continue;
        }

        triggered.push(rule.name.clone());
        if rule.action != Action::Flag {
            out = detect::redact(&out, &hits);
            redacted = true;
        }
    }

    OutputVerdict {
        text: out,
        redacted,
        triggered,
    }
}

pub fn triggered_event(
    rules: &[String],
    phase: &str,
    virtual_key_id: Option<&str>,
    endpoint: &str,
) -> webhooks::WebhookEvent {
    webhooks::WebhookEvent::new(
        webhooks::EVENT_GUARDRAIL_TRIGGERED,
        serde_json::json!({
            "rules": rules,
            "phase": phase,
            "virtual_key_id": virtual_key_id,
            "endpoint": endpoint,
        }),
    )
}

pub fn action_label(flags: &[String], redactions: &[String]) -> Option<String> {
    let mut parts = Vec::new();
    if !redactions.is_empty() {
        parts.push(format!("redact:{}", redactions.join(",")));
    }
    if !flags.is_empty() {
        parts.push(format!("flag:{}", flags.join(",")));
    }
    match parts.is_empty() {
        true => None,
        false => Some(parts.join(";")),
    }
}

pub fn dedup(names: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    names
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_label_joins_both_actions() {
        let label = action_label(&["inject".into()], &["pii".into()]);
        assert_eq!(label.as_deref(), Some("redact:pii;flag:inject"));
        assert!(action_label(&[], &[]).is_none());
    }

    #[test]
    fn unknown_phase_is_dropped() {
        let row = guardrail_rule::Model {
            id: "r1".into(),
            name: "both".into(),
            phase: "both".into(),
            rule_type: "pii".into(),
            pattern: None,
            action: "block".into(),
            priority: 0,
            is_active: 1,
            created_at: String::new(),
            updated_at: String::new(),
        };
        assert!(Rule::from_model(row).is_none());
    }

    #[test]
    fn dedup_keeps_first_occurrence() {
        assert_eq!(dedup(vec!["a".into(), "a".into(), "b".into()]).len(), 2);
    }
}
