mod render;

pub use render::{
    encode_variables, parse_variables, render_template, RenderedPrompt, TemplateVariable,
};

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::{
    entities::prompt_template,
    error::{AppError, AppResult},
    providers::ChatMessage,
    AppState,
};

#[derive(Clone, Debug)]
pub struct PromptTemplate {
    pub row: prompt_template::Model,
    pub variables: Vec<TemplateVariable>,
}

impl PromptTemplate {
    pub fn from_row(row: prompt_template::Model) -> Self {
        let variables = TemplateVariable::parse_list(&row.variables_json);
        Self { row, variables }
    }

    pub fn render(&self, values: &serde_json::Map<String, serde_json::Value>) -> RenderedPrompt {
        render_template(
            self.variables.as_slice(),
            &self.row.system_prompt,
            &self.row.user_template,
            values,
        )
    }

    pub fn messages(&self, rendered: &RenderedPrompt) -> Vec<ChatMessage> {
        let mut messages = Vec::with_capacity(2);
        if !rendered.system.is_empty() {
            messages.push(ChatMessage::text("system", rendered.system.as_str()));
        }
        messages.push(ChatMessage::text("user", rendered.user.as_str()));
        messages
    }
}

pub async fn versions(state: &AppState, slug: &str) -> AppResult<Vec<prompt_template::Model>> {
    prompt_template::Entity::find()
        .filter(prompt_template::Column::Slug.eq(slug))
        .order_by_desc(prompt_template::Column::Version)
        .all(&state.db)
        .await
        .map_err(AppError::from)
}

pub async fn version(
    state: &AppState,
    slug: &str,
    version: i64,
) -> AppResult<prompt_template::Model> {
    prompt_template::Entity::find()
        .filter(prompt_template::Column::Slug.eq(slug))
        .filter(prompt_template::Column::Version.eq(version))
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("prompt template '{slug}' v{version} not found")))
}

pub async fn latest_version(state: &AppState, slug: &str) -> AppResult<Option<i64>> {
    Ok(prompt_template::Entity::find()
        .filter(prompt_template::Column::Slug.eq(slug))
        .order_by_desc(prompt_template::Column::Version)
        .one(&state.db)
        .await?
        .map(|row| row.version))
}

pub async fn select(
    state: &AppState,
    slug: &str,
    requested: Option<i64>,
) -> AppResult<PromptTemplate> {
    if let Some(requested_version) = requested {
        return version(state, slug, requested_version)
            .await
            .map(PromptTemplate::from_row);
    }

    let rows = versions(state, slug).await?;
    let row = rows
        .iter()
        .find(|row| row.is_active != 0)
        .or_else(|| rows.first())
        .ok_or_else(|| AppError::NotFound(format!("prompt template '{slug}' not found")))?;
    Ok(PromptTemplate::from_row(row.clone()))
}
