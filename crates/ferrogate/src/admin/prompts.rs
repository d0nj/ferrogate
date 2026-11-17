use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::{
    entities::prompt_template,
    error::{AppError, AppResult},
    prompts::{self, encode_variables, RenderedPrompt, TemplateVariable},
    AppState,
};

use super::now_rfc3339;

#[derive(Debug, Serialize)]
pub struct PromptVersionDto {
    pub id: String,
    pub slug: String,
    pub version: i64,
    pub name: String,
    pub system_prompt: String,
    pub user_template: String,
    pub model: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i64>,
    pub variables: Vec<TemplateVariable>,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl PromptVersionDto {
    fn build(row: prompt_template::Model, variables: Vec<TemplateVariable>) -> Self {
        Self {
            id: row.id,
            slug: row.slug,
            version: row.version,
            name: row.name,
            system_prompt: row.system_prompt,
            user_template: row.user_template,
            model: row.model,
            temperature: row.temperature,
            max_tokens: row.max_tokens,
            variables,
            is_active: row.is_active != 0,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

impl From<prompt_template::Model> for PromptVersionDto {
    fn from(row: prompt_template::Model) -> Self {
        let variables = TemplateVariable::parse_list(&row.variables_json);
        Self::build(row, variables)
    }
}

#[derive(Debug, Serialize)]
pub struct PromptSummaryDto {
    pub slug: String,
    pub name: String,
    pub latest_version: i64,
    pub versions: i64,
    pub is_active: bool,
    pub model: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i64>,
    pub variables: Vec<TemplateVariable>,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePrompt {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub user_template: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<i64>,
    #[serde(default)]
    pub variables: Option<Vec<TemplateVariable>>,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePrompt {
    pub name: Option<String>,
    pub system_prompt: Option<String>,
    pub user_template: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_nullable")]
    pub model: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_nullable_f64")]
    pub temperature: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_nullable_i64")]
    pub max_tokens: Option<Option<i64>>,
    pub variables: Option<Vec<TemplateVariable>>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RenderPrompt {
    #[serde(default)]
    pub variables: Map<String, Value>,
    #[serde(default)]
    pub version: Option<i64>,
}

fn deserialize_optional_nullable<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

fn deserialize_optional_nullable_f64<'de, D>(
    deserializer: D,
) -> Result<Option<Option<f64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<f64>::deserialize(deserializer).map(Some)
}

fn deserialize_optional_nullable_i64<'de, D>(
    deserializer: D,
) -> Result<Option<Option<i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<i64>::deserialize(deserializer).map(Some)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/api/prompts", get(list).post(create))
        .route(
            "/admin/api/prompts/{slug}",
            get(list_versions).delete(remove),
        )
        .route(
            "/admin/api/prompts/{slug}/{version}",
            get(show).patch(update),
        )
        .route(
            "/admin/api/prompts/{slug}/activate/{version}",
            post(activate),
        )
        .route("/admin/api/prompts/{slug}/render", post(render))
}

pub async fn list(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows = prompt_template::Entity::find()
        .order_by_asc(prompt_template::Column::Slug)
        .order_by_desc(prompt_template::Column::Version)
        .all(&state.db)
        .await?;

    let mut summaries: Vec<PromptSummaryDto> = Vec::new();
    for row in rows {
        match summaries.iter_mut().find(|item| item.slug == row.slug) {
            Some(summary) => summary.versions += 1,
            None => summaries.push(PromptSummaryDto {
                slug: row.slug.clone(),
                name: row.name.clone(),
                latest_version: row.version,
                versions: 1,
                is_active: row.is_active != 0,
                model: row.model.clone(),
                temperature: row.temperature,
                max_tokens: row.max_tokens,
                variables: TemplateVariable::parse_list(&row.variables_json),
                updated_at: row.updated_at.clone(),
            }),
        }
    }

    Ok(Json(json!({ "prompts": summaries })))
}

pub async fn list_versions(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> AppResult<Json<Value>> {
    let rows = prompts::versions(&state, &slug).await?;
    if rows.is_empty() {
        return Err(AppError::NotFound(format!(
            "prompt template '{slug}' not found"
        )));
    }

    let versions: Vec<PromptVersionDto> = rows.into_iter().map(PromptVersionDto::from).collect();
    Ok(Json(json!({ "slug": slug, "versions": versions })))
}

pub async fn show(
    State(state): State<AppState>,
    Path((slug, version)): Path<(String, i64)>,
) -> AppResult<Json<Value>> {
    let row = prompts::version(&state, &slug, version).await?;
    Ok(Json(json!({ "prompt": PromptVersionDto::from(row) })))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreatePrompt>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let slug = require_slug(&body.slug)?;
    let name = require_name(&body.name)?;
    let variables = body.variables.unwrap_or_default();
    let version = prompts::latest_version(&state, &slug)
        .await?
        .map_or(1, |latest| latest + 1);
    let is_active = body.is_active.unwrap_or(true);

    let now = now_rfc3339();
    let row = prompt_template::ActiveModel {
        id: Set(format!("prompt-{}", uuid::Uuid::new_v4())),
        slug: Set(slug.clone()),
        version: Set(version),
        name: Set(name),
        system_prompt: Set(body.system_prompt),
        user_template: Set(body.user_template),
        model: Set(body.model),
        temperature: Set(body.temperature),
        max_tokens: Set(body.max_tokens),
        variables_json: Set(encode_variables(&variables)),
        is_active: Set(i64::from(is_active)),
        created_at: Set(now.clone()),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    if is_active {
        deactivate_siblings(&state, &slug, &row.id).await?;
    }

    Ok((
        StatusCode::CREATED,
        Json(json!({ "prompt": PromptVersionDto::build(row, variables) })),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    Path((slug, version)): Path<(String, i64)>,
    Json(body): Json<UpdatePrompt>,
) -> AppResult<Json<Value>> {
    let existing = prompts::version(&state, &slug, version).await?;
    let mut variables = TemplateVariable::parse_list(&existing.variables_json);

    let mut active: prompt_template::ActiveModel = existing.clone().into();
    if let Some(name) = body.name.as_deref() {
        active.name = Set(require_name(name)?);
    }
    if let Some(system_prompt) = body.system_prompt {
        active.system_prompt = Set(system_prompt);
    }
    if let Some(user_template) = body.user_template {
        active.user_template = Set(user_template);
    }
    if let Some(model) = body.model {
        active.model = Set(model.filter(|value| !value.trim().is_empty()));
    }
    if let Some(temperature) = body.temperature {
        active.temperature = Set(temperature);
    }
    if let Some(max_tokens) = body.max_tokens {
        active.max_tokens = Set(max_tokens);
    }
    if let Some(next) = body.variables {
        variables = next;
        active.variables_json = Set(encode_variables(&variables));
    }
    if let Some(is_active) = body.is_active {
        active.is_active = Set(i64::from(is_active));
    }
    active.updated_at = Set(now_rfc3339());

    let row = active.update(&state.db).await?;
    if row.is_active != 0 {
        deactivate_siblings(&state, &slug, &row.id).await?;
    }

    Ok(Json(
        json!({ "prompt": PromptVersionDto::build(row, variables) }),
    ))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> AppResult<Json<Value>> {
    let result = prompt_template::Entity::delete_many()
        .filter(prompt_template::Column::Slug.eq(&slug))
        .exec(&state.db)
        .await?;

    if result.rows_affected == 0 {
        return Err(AppError::NotFound(format!(
            "prompt template '{slug}' not found"
        )));
    }

    Ok(Json(
        json!({ "deleted": true, "versions": result.rows_affected }),
    ))
}

pub async fn activate(
    State(state): State<AppState>,
    Path((slug, version)): Path<(String, i64)>,
) -> AppResult<Json<Value>> {
    let existing = prompts::version(&state, &slug, version).await?;
    let mut active: prompt_template::ActiveModel = existing.into();
    active.is_active = Set(1);
    active.updated_at = Set(now_rfc3339());
    let row = active.update(&state.db).await?;

    deactivate_siblings(&state, &slug, &row.id).await?;
    Ok(Json(json!({ "prompt": PromptVersionDto::from(row) })))
}

pub async fn render(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<RenderPrompt>,
) -> AppResult<Json<RenderedPrompt>> {
    let template = prompts::select(&state, &slug, body.version).await?;
    Ok(Json(template.render(&body.variables)))
}

async fn deactivate_siblings(state: &AppState, slug: &str, keep_id: &str) -> AppResult<()> {
    let rows = prompt_template::Entity::find()
        .filter(prompt_template::Column::Slug.eq(slug))
        .filter(prompt_template::Column::Id.ne(keep_id))
        .filter(prompt_template::Column::IsActive.ne(0))
        .all(&state.db)
        .await?;

    for row in rows {
        let mut active: prompt_template::ActiveModel = row.into();
        active.is_active = Set(0);
        active.updated_at = Set(now_rfc3339());
        active.update(&state.db).await?;
    }
    Ok(())
}

fn require_slug(slug: &str) -> AppResult<String> {
    let trimmed = slug.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "slug must be a non-empty string".into(),
        ));
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(AppError::BadRequest(
            "slug may only contain letters, digits, '-', '_' and '.'".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn require_name(name: &str) -> AppResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "name must be a non-empty string".into(),
        ));
    }
    Ok(trimmed.to_string())
}
