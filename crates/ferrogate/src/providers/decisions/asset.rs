use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::common::media_type_of;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DecisionAsset {
    pub kind: String,
    #[serde(default)]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_data: Option<Value>,
}

impl DecisionAsset {
    pub fn new(kind: impl Into<String>, source: impl Into<String>) -> Self {
        let source = source.into();
        let media_type = media_type_of(&source);
        Self {
            kind: kind.into(),
            source,
            media_type,
            ..Self::default()
        }
    }

    pub fn json(payload: Value) -> Self {
        Self {
            json_data: Some(payload),
            ..Self::new("json", "")
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_resolution(mut self, resolution: impl Into<String>) -> Self {
        self.resolution = Some(resolution.into());
        self
    }

    pub fn resolved_media_type(&self) -> &str {
        self.media_type
            .as_deref()
            .unwrap_or(match self.kind.as_str() {
                "audio" => "audio/mpeg",
                "video" => "video/mp4",
                "document" => "application/pdf",
                "text" => "text/plain",
                "json" => "application/json",
                _ => "image/jpeg",
            })
    }

    pub fn data_url_payload(&self) -> Option<(&str, &str)> {
        super::super::support::parse_data_url(&self.source).map(|parsed| (parsed.mime, parsed.data))
    }

    pub fn is_binary_data(&self) -> bool {
        self.data_url_payload().is_some()
    }

    pub(crate) fn key(&self, index: usize) -> String {
        self.id
            .clone()
            .unwrap_or_else(|| format!("{}-{index}", self.kind))
    }
}
