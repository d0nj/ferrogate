use std::{sync::Arc, time::Duration};

use dashmap::DashMap;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    entities::semantic_cache_entry::{self, Entity as Entry},
    error::AppResult,
    providers::ChatCompletionResponse,
};

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: String,
    pub cache_key: String,
    pub model: String,
    pub embedding: Vec<f32>,
    pub response: ChatCompletionResponse,
}

pub const MAX_ENTRIES: u64 = 5_000;

const LOOKUP_LIMIT: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Sqlite,
    Qdrant,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Qdrant => "qdrant",
        }
    }
}

pub struct NewEntry<'a> {
    pub cache_key: &'a str,
    pub model: &'a str,
    pub virtual_key_id: Option<&'a str>,
    pub provider_name: Option<&'a str>,
    pub prompt_text: &'a str,
    pub embedding: &'a [f32],
    pub response: &'a ChatCompletionResponse,
    pub expires_at: &'a str,
}

pub struct EntryStore {
    db: DatabaseConnection,
    qdrant_url: Option<String>,
    qdrant: DashMap<String, Arc<Qdrant>>,
}

impl EntryStore {
    pub fn new(db: DatabaseConnection, qdrant_url: Option<String>) -> Self {
        Self {
            db,
            qdrant_url,
            qdrant: DashMap::new(),
        }
    }

    pub fn collection_name(model: Option<&str>, dims: Option<usize>) -> String {
        let base = match model {
            None => "semantic_cache_local".to_string(),
            Some(model) => {
                let slug: String = model
                    .chars()
                    .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
                    .collect();
                format!("semantic_cache_{slug}")
            }
        };
        match dims {
            Some(dims) => format!("{base}_{dims}"),
            None => base,
        }
    }

    fn client(&self, name: &str) -> Option<Arc<Qdrant>> {
        let url = self.qdrant_url.as_deref()?;
        if let Some(existing) = self.qdrant.get(name) {
            return Some(existing.clone());
        }
        let created = Arc::new(Qdrant::new(url.to_string(), name));
        Some(
            self.qdrant
                .entry(name.to_string())
                .or_insert(created)
                .clone(),
        )
    }

    pub fn backend(&self) -> Backend {
        match self.qdrant_url {
            Some(_) => Backend::Qdrant,
            None => Backend::Sqlite,
        }
    }

    pub async fn ensure_collection(&self, name: &str, dims: usize) {
        let Some(qdrant) = self.client(name) else {
            return;
        };
        match qdrant.ensure_collection(dims).await {
            Ok(true) => tracing::info!(collection = name, "qdrant collection ready"),
            Ok(false) => {}
            Err(err) => qdrant.warn_once(&err),
        }
    }

    pub fn database(&self) -> &DatabaseConnection {
        &self.db
    }

    pub async fn insert(&self, entry: NewEntry<'_>, collection: &str) -> AppResult<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let embedding_json = serde_json::to_string(entry.embedding)?;
        let response_json = serde_json::to_string(entry.response)?;
        let now = now_rfc3339();

        sqlx::query(
            "INSERT INTO semantic_cache_entries \
             (id, cache_key, model, provider_name, virtual_key_id, prompt_text, embedding_json, response_json, \
              hits, created_at, expires_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10) \
             ON CONFLICT(cache_key) DO UPDATE SET \
              model = excluded.model, provider_name = excluded.provider_name, \
              virtual_key_id = excluded.virtual_key_id, \
              prompt_text = excluded.prompt_text, embedding_json = excluded.embedding_json, \
              response_json = excluded.response_json, hits = 0, \
              created_at = excluded.created_at, expires_at = excluded.expires_at",
        )
        .bind(&id)
        .bind(entry.cache_key)
        .bind(entry.model)
        .bind(entry.provider_name)
        .bind(entry.virtual_key_id)
        .bind(entry.prompt_text)
        .bind(&embedding_json)
        .bind(&response_json)
        .bind(&now)
        .bind(entry.expires_at)
        .execute(self.db.get_sqlite_connection_pool())
        .await?;

        if let Some(qdrant) = self.client(collection) {
            let point = Point {
                id: id.clone(),
                vector: entry.embedding.to_vec(),
                payload: json!({
                    "cache_key": entry.cache_key,
                    "model": entry.model,
                    "virtual_key_id": entry.virtual_key_id,
                    "expires_at": entry.expires_at,
                }),
            };
            if let Err(err) = qdrant.upsert(&point).await {
                qdrant.warn_once(&err);
            }
        }

        self.trim().await;
        Ok(id)
    }

    pub async fn search(
        &self,
        model: &str,
        key_id: Option<&str>,
        embedding: &[f32],
        threshold: f32,
        now: &str,
        collection: &str,
    ) -> AppResult<Vec<Candidate>> {
        if let Some(qdrant) = self.client(collection) {
            match qdrant.search(model, key_id, embedding, threshold).await {
                Ok(hits) => {
                    if let Some(candidates) = self.load_by_ids(&hits, now).await? {
                        return Ok(candidates);
                    }
                }
                Err(err) => qdrant.warn_once(&err),
            }
        }

        let candidates = self.sqlite_candidates(model, key_id, now).await?;
        Ok(rank(candidates, embedding, threshold))
    }

    pub async fn clear(&self, collection: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM semantic_cache_entries")
            .execute(self.db.get_sqlite_connection_pool())
            .await?;

        if let Some(qdrant) = self.client(collection) {
            if let Err(err) = qdrant.clear().await {
                qdrant.warn_once(&err);
            }
        }
        Ok(())
    }

    pub async fn count(&self) -> AppResult<u64> {
        let rows: Vec<(i64,)> =
            sqlx::query_as("SELECT COUNT(*) FROM semantic_cache_entries WHERE expires_at > ?1")
                .bind(now_rfc3339())
                .fetch_all(self.db.get_sqlite_connection_pool())
                .await?;
        Ok(rows
            .first()
            .map(|(count,)| (*count).max(0) as u64)
            .unwrap_or(0))
    }

    pub async fn sweep(&self) -> AppResult<u64> {
        let expired = sqlx::query("DELETE FROM semantic_cache_entries WHERE expires_at <= ?1")
            .bind(now_rfc3339())
            .execute(self.db.get_sqlite_connection_pool())
            .await?
            .rows_affected();

        let overflow = sqlx::query(
            "DELETE FROM semantic_cache_entries WHERE id IN ( \
                 SELECT id FROM semantic_cache_entries \
                 ORDER BY created_at DESC LIMIT -1 OFFSET ?1)",
        )
        .bind(MAX_ENTRIES as i64)
        .execute(self.db.get_sqlite_connection_pool())
        .await?
        .rows_affected();

        Ok(expired + overflow)
    }

    async fn trim(&self) {
        if let Err(err) = self.sweep().await {
            tracing::warn!(error = %err, "semantic cache sweep failed");
        }
    }

    async fn sqlite_candidates(
        &self,
        model: &str,
        key_id: Option<&str>,
        now: &str,
    ) -> AppResult<Vec<Candidate>> {
        let mut query = Entry::find()
            .filter(semantic_cache_entry::Column::Model.eq(model))
            .filter(semantic_cache_entry::Column::ExpiresAt.gt(now));
        query = match key_id {
            Some(key_id) => query.filter(semantic_cache_entry::Column::VirtualKeyId.eq(key_id)),
            None => query.filter(semantic_cache_entry::Column::VirtualKeyId.is_null()),
        };
        let rows = query
            .order_by_desc(semantic_cache_entry::Column::CreatedAt)
            .limit(LOOKUP_LIMIT)
            .all(&self.db)
            .await?;

        Ok(rows.into_iter().filter_map(to_candidate).collect())
    }

    async fn load_by_ids(&self, ids: &[String], now: &str) -> AppResult<Option<Vec<Candidate>>> {
        if ids.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let rows = Entry::find()
            .filter(semantic_cache_entry::Column::Id.is_in(ids.to_vec()))
            .filter(semantic_cache_entry::Column::ExpiresAt.gt(now))
            .all(&self.db)
            .await?;
        Ok(Some(rows.into_iter().filter_map(to_candidate).collect()))
    }
}

fn to_candidate(row: semantic_cache_entry::Model) -> Option<Candidate> {
    let embedding: Vec<f32> = serde_json::from_str(&row.embedding_json).ok()?;
    let response: ChatCompletionResponse = serde_json::from_str(&row.response_json).ok()?;
    Some(Candidate {
        id: row.id,
        cache_key: row.cache_key,
        model: row.model,
        embedding,
        response,
    })
}

fn rank(candidates: Vec<Candidate>, embedding: &[f32], threshold: f32) -> Vec<Candidate> {
    let mut scored: Vec<(f32, Candidate)> = candidates
        .into_iter()
        .map(|candidate| {
            let score = super::embed::cosine(&candidate.embedding, embedding);
            (score, candidate)
        })
        .filter(|(score, _)| *score >= threshold)
        .collect();

    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().map(|(_, candidate)| candidate).collect()
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[derive(Serialize)]
struct Point {
    id: String,
    vector: Vec<f32>,
    payload: Value,
}

struct Qdrant {
    base: String,
    client: reqwest::Client,
    warned: std::sync::atomic::AtomicBool,
}

impl Qdrant {
    fn new(url: String, collection: &str) -> Self {
        Self {
            base: format!("{}/collections/{collection}", url.trim_end_matches('/')),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
            warned: std::sync::atomic::AtomicBool::new(false),
        }
    }

    async fn ensure_collection(&self, dims: usize) -> Result<bool, String> {
        let body = json!({ "vectors": { "size": dims, "distance": "Cosine" } });
        let response = self
            .client
            .put(&self.base)
            .json(&body)
            .send()
            .await
            .map_err(|err| err.to_string())?;

        if response.status().is_success() {
            return Ok(true);
        }
        if response.status().as_u16() == 409 {
            return Ok(false);
        }
        Err(format!(
            "create collection failed: HTTP {}",
            response.status()
        ))
    }

    async fn upsert(&self, point: &Point) -> Result<(), String> {
        self.client
            .put(format!("{}/points?wait=false", self.base))
            .json(&json!({ "points": [point] }))
            .send()
            .await
            .map_err(|err| err.to_string())?
            .error_for_status()
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn search(
        &self,
        model: &str,
        key_id: Option<&str>,
        embedding: &[f32],
        threshold: f32,
    ) -> Result<Vec<String>, String> {
        let mut filter = vec![json!({ "key": "model", "match": { "value": model } })];
        match key_id {
            Some(key_id) => {
                filter.push(json!({ "key": "virtual_key_id", "match": { "value": key_id } }))
            }
            None => filter.push(json!({ "key": "virtual_key_id", "match": { "any": [null] } })),
        }
        let body = json!({
            "vector": embedding,
            "limit": LOOKUP_LIMIT,
            "with_payload": false,
            "score_threshold": threshold,
            "filter": { "must": filter },
        });

        let payload: Value = self
            .client
            .post(format!("{}/points/search", self.base))
            .json(&body)
            .send()
            .await
            .map_err(|err| err.to_string())?
            .error_for_status()
            .map_err(|err| err.to_string())?
            .json()
            .await
            .map_err(|err| err.to_string())?;

        Ok(payload["result"]
            .as_array()
            .map(|hits| {
                hits.iter()
                    .filter_map(|hit| hit["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn clear(&self) -> Result<(), String> {
        self.client
            .post(format!("{}/points/delete?wait=false", self.base))
            .json(&json!({ "filter": {} }))
            .send()
            .await
            .map_err(|err| err.to_string())?
            .error_for_status()
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    fn warn_once(&self, err: &str) {
        use std::sync::atomic::Ordering;
        if !self.warned.swap(true, Ordering::Relaxed) {
            tracing::debug!(error = %err, "qdrant unavailable; semantic cache using sqlite");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_url_trims_trailing_slashes() {
        let qdrant = Qdrant::new(
            "http://127.0.0.1:6333/".to_string(),
            "semantic_cache_local_256",
        );
        assert_eq!(
            qdrant.base,
            "http://127.0.0.1:6333/collections/semantic_cache_local_256"
        );
    }

    #[test]
    fn collection_name_tracks_the_vector_space() {
        assert_eq!(
            EntryStore::collection_name(None, Some(256)),
            "semantic_cache_local_256"
        );
        assert_eq!(
            EntryStore::collection_name(Some("text-embedding-3-small"), None),
            "semantic_cache_text_embedding_3_small"
        );
        assert_ne!(
            EntryStore::collection_name(Some("text-embedding-3-small"), None),
            EntryStore::collection_name(Some("text-embedding-3-large"), None)
        );
        assert!(
            EntryStore::collection_name(Some("nvidia/nv-embedqa-e5-v5"), None)
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        );
    }

    #[test]
    fn cosine_ranking_orders_and_filters_by_threshold() {
        let embedding = vec![1.0f32, 0.0, 0.0];
        let candidate = |id: &str, vector: Vec<f32>| Candidate {
            id: id.to_string(),
            cache_key: id.to_string(),
            model: "m".to_string(),
            embedding: vector,
            response: ChatCompletionResponse {
                id: id.to_string(),
                object: "chat.completion".to_string(),
                created: 0,
                model: "m".to_string(),
                choices: Vec::new(),
                usage: None,
            },
        };

        let ranked = rank(
            vec![
                candidate("orthogonal", vec![0.0, 1.0, 0.0]),
                candidate("identical", vec![1.0, 0.0, 0.0]),
            ],
            &embedding,
            0.5,
        );

        assert_eq!(ranked.len(), 1, "below-threshold candidates are dropped");
        assert_eq!(ranked[0].id, "identical");
    }
}
