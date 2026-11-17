use std::{collections::BTreeMap, future::Future, pin::Pin};

use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type ProviderEventStream =
    Pin<Box<dyn Stream<Item = Result<StreamEvent, UpstreamCallError>> + Send>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiDialect {
    #[default]
    Chat,
    Responses,
}

impl ApiDialect {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "responses" => Self::Responses,
            _ => Self::Chat,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Responses => "responses",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Gemini,
    Typesafe,
}

impl ProviderKind {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "openai" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            "gemini" => Some(Self::Gemini),
            "typesafe" | "systemone" => Some(Self::Typesafe),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Typesafe => "typesafe",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderEndpoint {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub dialect: ApiDialect,
    pub base_url: String,
    pub api_key: String,
    pub upstream_model: String,
    pub timeout_ms: u64,
    pub extra_headers: BTreeMap<String, String>,
    pub zdr: bool,
    pub region: Option<String>,
    pub selected_key_id: Option<String>,
    pub selected_key_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Value>,
}

impl ChatMessage {
    pub fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Value::String(content.into()),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_version: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_variables: Option<serde_json::Map<String, Value>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageInfo {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChoiceMessage {
    pub role: String,
    pub content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChoice {
    pub index: u32,
    pub message: ChatChoiceMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageInfo>,
}

impl ChatCompletionResponse {
    pub fn content_mut(&mut self) -> Option<&mut serde_json::Value> {
        self.choices
            .first_mut()
            .map(|choice| &mut choice.message.content)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingData {
    pub object: String,
    pub embedding: Value,
    pub index: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmbeddingUsage {
    pub prompt_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    pub object: String,
    pub data: Vec<EmbeddingData>,
    pub model: String,
    pub usage: EmbeddingUsage,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Chunk {
        data: Value,
        delta_text: Option<String>,
        usage: Option<UsageInfo>,
        finish_reason: Option<String>,
    },
    Done,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum UpstreamCallError {
    #[error("upstream HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("upstream transport error: {0}")]
    Transport(String),
    #[error("upstream protocol error: {0}")]
    Protocol(String),
}

impl UpstreamCallError {
    pub fn class(&self) -> crate::routing::FailureClass {
        match self {
            Self::Status { status, .. } => match *status {
                429 => crate::routing::FailureClass::RateLimit,
                500..=599 => crate::routing::FailureClass::ServerError,
                _ => crate::routing::FailureClass::ClientError,
            },
            Self::Transport(_) => crate::routing::FailureClass::Transport,
            Self::Protocol(_) => crate::routing::FailureClass::Protocol,
        }
    }

    pub fn is_retryable_under(&self, policy: &crate::routing::RoutingPolicy) -> bool {
        policy.retry_on.allows(self.class())
    }

    pub fn is_retryable(&self) -> bool {
        matches!(
            self.class(),
            crate::routing::FailureClass::RateLimit
                | crate::routing::FailureClass::ServerError
                | crate::routing::FailureClass::Transport
        )
    }

    pub fn status_code(&self) -> u16 {
        match self {
            Self::Status { status, .. } => *status,
            Self::Transport(_) => 502,
            Self::Protocol(_) => 504,
        }
    }
}

#[cfg(test)]
mod retry_policy_tests {
    use super::*;
    use crate::routing::{RetryClasses, RoutingPolicy};

    fn status(code: u16) -> UpstreamCallError {
        UpstreamCallError::Status {
            status: code,
            body: "upstream said no".into(),
        }
    }

    #[test]
    fn statuses_map_to_the_right_class() {
        assert_eq!(status(429).class(), crate::routing::FailureClass::RateLimit);
        assert_eq!(
            status(503).class(),
            crate::routing::FailureClass::ServerError
        );
        assert_eq!(
            UpstreamCallError::Transport("connection reset".into()).class(),
            crate::routing::FailureClass::Transport
        );
        assert_eq!(
            UpstreamCallError::Protocol("bad json".into()).class(),
            crate::routing::FailureClass::Protocol
        );
    }

    #[test]
    fn client_errors_are_never_retried() {
        let policy = RoutingPolicy {
            retry_on: RetryClasses {
                rate_limit: true,
                server_error: true,
                transport: true,
                protocol: true,
            },
            ..RoutingPolicy::default()
        };
        assert_eq!(
            status(400).class(),
            crate::routing::FailureClass::ClientError
        );
        assert!(!status(400).is_retryable_under(&policy));
        assert!(!status(404).is_retryable_under(&policy));
    }

    #[test]
    fn disabling_a_class_stops_failover_for_it() {
        let policy = RoutingPolicy {
            retry_on: RetryClasses {
                rate_limit: false,
                server_error: true,
                transport: true,
                protocol: false,
            },
            ..RoutingPolicy::default()
        };
        assert!(!status(429).is_retryable_under(&policy));
        assert!(status(503).is_retryable_under(&policy));
        assert!(UpstreamCallError::Transport("timeout".into()).is_retryable_under(&policy));
    }

    #[test]
    fn default_policy_matches_the_previous_hardcoded_behaviour() {
        let policy = RoutingPolicy::default();
        assert!(status(429).is_retryable_under(&policy));
        assert!(status(500).is_retryable_under(&policy));
        assert!(status(599).is_retryable_under(&policy));
        assert!(UpstreamCallError::Transport("dns".into()).is_retryable_under(&policy));
        assert!(!UpstreamCallError::Protocol("bad".into()).is_retryable_under(&policy));
    }
}
