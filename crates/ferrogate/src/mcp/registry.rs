use std::{collections::HashMap, sync::Arc, time::Duration};

use moka::future::Cache;
use reqwest::Client;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

const TOOLS_TTL: Duration = Duration::from_secs(60);
const TOOLS_CAPACITY: u64 = 256;

pub const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);

pub const STATUS_READY: &str = "ready";
pub const STATUS_UNREACHABLE: &str = "unreachable";
pub const STATUS_NEVER_SYNCED: &str = "never_synced";

const TRANSPORT_HTTP: &str = "http";
const TRANSPORT_SSE: &str = "sse";

const NAME_SEPARATOR: &str = "__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ServerEntry {
    pub id: String,
    pub name: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub tools: Arc<Vec<ToolInfo>>,
}

impl ServerEntry {
    pub fn qualified(server: &str, tool: &str) -> String {
        format!("{server}{NAME_SEPARATOR}{tool}")
    }

    pub fn find_tool(&self, qualified_name: &str) -> Option<&ToolInfo> {
        let tool = qualified_name
            .strip_prefix(self.name.as_str())
            .and_then(|rest| rest.strip_prefix(NAME_SEPARATOR))?;
        self.tools.iter().find(|entry| entry.name == tool)
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct CachedTools {
    #[serde(default)]
    names: Vec<String>,
    #[serde(default)]
    tools: Vec<CachedTool>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CachedTool {
    name: String,
    #[serde(default)]
    description: String,
}

impl CachedTools {
    fn into_tools(self) -> Vec<ToolInfo> {
        match self.tools.is_empty() {
            true => self
                .names
                .into_iter()
                .map(|name| ToolInfo {
                    description: description_for(&name),
                    name,
                })
                .collect(),
            false => self
                .tools
                .into_iter()
                .map(|tool| ToolInfo {
                    name: tool.name,
                    description: tool.description,
                })
                .collect(),
        }
    }

    fn from_tools(tools: &[ToolInfo]) -> Self {
        Self {
            names: tools.iter().map(|tool| tool.name.clone()).collect(),
            tools: tools
                .iter()
                .map(|tool| CachedTool {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                })
                .collect(),
        }
    }

    fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

fn description_for(name: &str) -> String {
    format!("Tool `{name}` reported by this MCP server.")
}

pub struct McpRegistry {
    db: DatabaseConnection,
    http: Client,
    cache: Cache<String, Arc<Vec<ToolInfo>>>,
}

impl McpRegistry {
    pub fn new(db: DatabaseConnection, http: Client) -> Self {
        let cache = Cache::builder()
            .max_capacity(TOOLS_CAPACITY)
            .time_to_live(TOOLS_TTL)
            .build();

        Self { db, http, cache }
    }

    pub async fn active_servers(&self) -> AppResult<Vec<ServerEntry>> {
        let rows = crate::entities::mcp_server::Entity::find()
            .filter(crate::entities::mcp_server::Column::IsActive.eq(1))
            .order_by_asc(crate::entities::mcp_server::Column::Name)
            .all(&self.db)
            .await?;

        let mut servers = Vec::with_capacity(rows.len());
        for row in rows {
            let tools = self.tools_for(&row).await;
            servers.push(ServerEntry {
                id: row.id,
                name: row.name,
                url: row.url,
                headers: parse_headers(&row.headers_json),
                tools,
            });
        }
        Ok(servers)
    }

    pub async fn resolve(&self, qualified_name: &str) -> AppResult<(ServerEntry, ToolInfo)> {
        for server in self.active_servers().await? {
            let tool = server.find_tool(qualified_name).map(|tool| ToolInfo {
                name: tool.name.clone(),
                description: tool.description.clone(),
            });

            if let Some(tool) = tool {
                return Ok((server, tool));
            }
        }

        Err(AppError::NotFound(format!(
            "unknown tool '{qualified_name}'"
        )))
    }

    pub async fn call_tool(
        &self,
        server: &ServerEntry,
        tool: &ToolInfo,
        arguments: Value,
    ) -> AppResult<Value> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool.name, "arguments": arguments },
        });

        let response = self.post(server, &request).await.map_err(|err| {
            AppError::Upstream(format!("mcp server '{}' failed: {err}", server.name))
        })?;

        if let Some(error) = response.get("error") {
            return Err(AppError::Upstream(format!(
                "mcp server '{}' rejected '{}': {}",
                server.name,
                tool.name,
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
            )));
        }

        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn refresh(
        &self,
        row: &crate::entities::mcp_server::Model,
    ) -> AppResult<Vec<ToolInfo>> {
        self.cache.invalidate(&row.id).await;

        let (tools, status) = match self.discover(row).await {
            Ok(tools) => (Some(tools), STATUS_READY),
            Err(err) => {
                tracing::warn!(server = %row.name, error = %err, "mcp tool discovery failed");
                (None, STATUS_UNREACHABLE)
            }
        };

        self.persist(row, tools.as_deref(), status).await?;

        let tools = tools.unwrap_or_default();
        let tools = Arc::new(filter_allowed(&row.allowed_tools_json, tools));
        self.cache.insert(row.id.clone(), tools.clone()).await;
        Ok((*tools).clone())
    }

    pub async fn invalidate(&self, server_id: &str) {
        self.cache.invalidate(server_id).await;
    }

    async fn tools_for(&self, row: &crate::entities::mcp_server::Model) -> Arc<Vec<ToolInfo>> {
        if let Some(cached) = self.cache.get(&row.id).await {
            return cached;
        }

        if let Some(fresh) = cached_from_row(row) {
            let tools = Arc::new(fresh);
            self.cache.insert(row.id.clone(), tools.clone()).await;
            return tools;
        }

        match self.refresh(row).await {
            Ok(tools) => Arc::new(tools),
            Err(err) => {
                tracing::warn!(server = %row.name, error = %err, "mcp discovery failed");
                Arc::new(Vec::new())
            }
        }
    }

    async fn discover(&self, row: &crate::entities::mcp_server::Model) -> AppResult<Vec<ToolInfo>> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {},
        });

        let entry = ServerEntry {
            id: row.id.clone(),
            name: row.name.clone(),
            url: row.url.clone(),
            headers: parse_headers(&row.headers_json),
            tools: Arc::new(Vec::new()),
        };

        let response = self.post(&entry, &request).await.map_err(|err| {
            AppError::Upstream(format!("mcp server '{}' failed: {err}", row.name))
        })?;
        if let Some(error) = response.get("error") {
            return Err(AppError::Upstream(format!(
                "mcp server '{}' returned an error: {}",
                row.name,
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
            )));
        }

        let listed = response
            .get("result")
            .and_then(|result| result.get("tools"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let tools = listed
            .iter()
            .filter_map(|tool| {
                let name = tool.get("name").and_then(Value::as_str)?.to_string();
                let description = tool
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| description_for(&name));
                Some(ToolInfo { name, description })
            })
            .collect::<Vec<_>>();

        Ok(filter_allowed(&row.allowed_tools_json, tools))
    }

    async fn persist(
        &self,
        row: &crate::entities::mcp_server::Model,
        tools: Option<&[ToolInfo]>,
        status: &str,
    ) -> AppResult<()> {
        let cached = CachedTools::from_tools(tools.unwrap_or_default());
        let update = crate::entities::mcp_server::ActiveModel {
            id: Set(row.id.clone()),
            tools_cache_json: Set(tools.map(|_| cached.to_json())),
            last_status: Set(Some(status.to_string())),
            ..Default::default()
        };
        update.update(&self.db).await?;
        Ok(())
    }

    async fn post(&self, server: &ServerEntry, body: &Value) -> Result<Value, String> {
        let mut request = self
            .http
            .post(&server.url)
            .timeout(DISCOVERY_TIMEOUT)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(body);

        for (name, value) in &server.headers {
            request = request.header(name.as_str(), value.as_str());
        }

        let response = request.send().await.map_err(|err| err.to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("upstream returned {status}"));
        }

        response
            .json::<Value>()
            .await
            .map_err(|err| format!("invalid JSON-RPC response: {err}"))
    }
}

pub fn decode_cached_tools(raw: Option<&str>) -> Vec<ToolInfo> {
    let Some(raw) = raw else {
        return Vec::new();
    };

    match serde_json::from_str::<CachedTools>(raw) {
        Ok(cached) => cached.into_tools(),
        Err(_) => Vec::new(),
    }
}

fn cached_from_row(row: &crate::entities::mcp_server::Model) -> Option<Vec<ToolInfo>> {
    if row.last_status.as_deref() != Some(STATUS_READY) {
        return None;
    }

    let raw = row.tools_cache_json.as_deref()?;
    let cached: CachedTools = serde_json::from_str(raw).ok()?;
    Some(filter_allowed(&row.allowed_tools_json, cached.into_tools()))
}

fn filter_allowed(allowed_tools_json: &str, tools: Vec<ToolInfo>) -> Vec<ToolInfo> {
    let allowed = parse_string_list(allowed_tools_json);
    if allowed.is_empty() {
        return tools;
    }

    tools
        .into_iter()
        .filter(|tool| allowed.iter().any(|name| name == &tool.name))
        .collect()
}

pub fn parse_string_list(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

fn parse_headers(raw: &str) -> Vec<(String, String)> {
    serde_json::from_str::<HashMap<String, String>>(raw)
        .map(|headers| headers.into_iter().collect())
        .unwrap_or_default()
}

pub fn parse_transport(raw: Option<&str>) -> AppResult<String> {
    let candidate = raw.map(str::trim).filter(|value| !value.is_empty());

    match candidate {
        None => Ok(TRANSPORT_HTTP.to_string()),
        Some(value @ (TRANSPORT_HTTP | TRANSPORT_SSE)) => Ok(value.to_string()),
        Some(other) => Err(AppError::BadRequest(format!(
            "transport must be '{TRANSPORT_HTTP}' or '{TRANSPORT_SSE}', got '{other}'"
        ))),
    }
}

pub fn acl_allows(allowed_mcp: &[String], server: &str, qualified: &str) -> bool {
    if allowed_mcp.is_empty() {
        return true;
    }

    let matches = |pattern: &str| {
        pattern == server
            || pattern == qualified
            || pattern
                .strip_suffix('*')
                .is_some_and(|prefix| qualified.starts_with(prefix) || server.starts_with(prefix))
    };

    allowed_mcp.iter().any(|pattern| matches(pattern.trim()))
}
