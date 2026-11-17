use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, ModelTrait, PaginatorTrait,
    QueryFilter, QueryOrder, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{
        password::{hash_password, validate_password, verify_password},
        role::Role,
        session::Principal,
    },
    entities::user,
    error::{AppError, AppResult},
    AppState,
};

use super::{audit, auth::CurrentPrincipal, now_rfc3339};

const MAX_EMAIL_LEN: usize = 254;

#[derive(Debug, Serialize)]
pub struct UserDto {
    pub id: String,
    pub email: String,
    pub name: String,
    pub role: Role,
    pub is_active: bool,
    pub created_at: String,
    pub last_login_at: Option<String>,
}

impl UserDto {
    fn from_row(row: user::Model) -> Self {
        Self {
            role: Role::from_str_strict(&row.role).unwrap_or(Role::Viewer),
            is_active: row.is_active != 0,
            id: row.id,
            email: row.email,
            name: row.name,
            created_at: row.created_at,
            last_login_at: row.last_login_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateUser {
    pub email: String,
    pub name: String,
    pub password: String,
    pub role: Role,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUser {
    pub name: Option<String>,
    pub email: Option<String>,
    pub role: Option<Role>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ResetPassword {
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePassword {
    pub current_password: String,
    pub new_password: String,
}

fn not_found(id: &str) -> AppError {
    AppError::NotFound(format!("user '{id}' not found"))
}

fn conflict(message: String) -> AppError {
    AppError::Conflict(message)
}

fn is_duplicate_email(error: &sea_orm::DbErr) -> bool {
    error
        .to_string()
        .to_ascii_lowercase()
        .contains("users.email")
}

fn normalise_email(raw: &str) -> AppResult<String> {
    let email = raw.trim().to_lowercase();
    if email.is_empty() {
        return Err(AppError::BadRequest("email must not be empty".into()));
    }
    if !email.contains('@') {
        return Err(AppError::BadRequest("email must contain an '@'".into()));
    }
    if email.chars().count() > MAX_EMAIL_LEN {
        return Err(AppError::BadRequest(format!(
            "email must be at most {MAX_EMAIL_LEN} characters"
        )));
    }
    Ok(email)
}

fn normalise_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("name must not be empty".into()));
    }
    Ok(name.to_string())
}

fn check_password(password: &str) -> AppResult<()> {
    validate_password(password).map_err(|reason| AppError::BadRequest(reason.to_string()))
}

async fn hash_offloaded(password: String) -> AppResult<String> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|error| AppError::Internal(format!("password hashing task failed: {error}")))?
        .map_err(|error| AppError::Internal(format!("password hashing failed: {error}")))
}

async fn verify_offloaded(password: String, phc: String) -> AppResult<bool> {
    tokio::task::spawn_blocking(move || verify_password(&password, &phc))
        .await
        .map_err(|error| AppError::Internal(format!("password check task failed: {error}")))
}

async fn find_user<C: ConnectionTrait>(db: &C, id: &str) -> AppResult<user::Model> {
    user::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| not_found(id))
}

async fn count_active_owners<C: ConnectionTrait>(db: &C) -> AppResult<u64> {
    Ok(user::Entity::find()
        .filter(user::Column::Role.eq(Role::Owner.as_str()))
        .filter(user::Column::IsActive.eq(1))
        .count(db)
        .await?)
}

async fn ensure_owner_remains<C: ConnectionTrait>(db: &C) -> AppResult<()> {
    if count_active_owners(db).await? == 0 {
        return Err(conflict(
            "at least one active owner must remain; promote another owner first".into(),
        ));
    }
    Ok(())
}

fn ensure_not_self(principal: &Principal, target: &str, action: &str) -> AppResult<()> {
    if principal.user_id == target {
        return Err(conflict(format!("you cannot {action} your own account")));
    }
    Ok(())
}

async fn audit_action<C: ConnectionTrait>(
    db: &C,
    principal: &Principal,
    action: &str,
    target: &str,
    detail: &str,
) -> AppResult<()> {
    audit::record(
        db,
        audit::Actor::Principal(principal),
        action,
        Some(target),
        audit::OUTCOME_ALLOW,
        Some(detail),
    )
    .await
}

pub async fn list(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows = user::Entity::find()
        .order_by_asc(user::Column::CreatedAt)
        .order_by_asc(user::Column::Email)
        .all(&state.db)
        .await?;

    let users: Vec<UserDto> = rows.into_iter().map(UserDto::from_row).collect();
    Ok(Json(json!({ "users": users })))
}

pub async fn create(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Json(body): Json<CreateUser>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let email = normalise_email(&body.email)?;
    let name = normalise_name(&body.name)?;
    check_password(&body.password)?;

    let txn = state.db.begin().await?;

    let existing = user::Entity::find()
        .filter(user::Column::Email.eq(email.as_str()))
        .one(&txn)
        .await?;
    if existing.is_some() {
        return Err(conflict("a user with that email already exists".into()));
    }

    let password_hash = hash_offloaded(body.password).await?;
    let id = uuid::Uuid::new_v4().to_string();

    let row = user::ActiveModel {
        id: Set(id.clone()),
        email: Set(email),
        name: Set(name),
        password_hash: Set(password_hash),
        role: Set(body.role.as_str().to_string()),
        is_active: Set(1),
        must_change_password: Set(0),
        created_at: Set(now_rfc3339()),
        last_login_at: Set(None),
    }
    .insert(&txn)
    .await
    .map_err(|error| {
        if is_duplicate_email(&error) {
            conflict("a user with that email already exists".into())
        } else {
            error.into()
        }
    })?;

    audit_action(
        &txn,
        &principal,
        "user.create",
        &row.id,
        &format!("created {} as {}", row.email, row.role),
    )
    .await?;
    txn.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "user": UserDto::from_row(row) })),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(id): Path<String>,
    Json(body): Json<UpdateUser>,
) -> AppResult<Json<UserDto>> {
    let txn = state.db.begin().await?;
    let existing = find_user(&txn, &id).await?;
    let stored_role = Role::from_str_strict(&existing.role).unwrap_or(Role::Viewer);
    let stored_active = existing.is_active != 0;

    let email = match body.email.as_deref() {
        Some(raw) => {
            let email = normalise_email(raw)?;
            let taken = user::Entity::find()
                .filter(user::Column::Email.eq(email.as_str()))
                .filter(user::Column::Id.ne(id.as_str()))
                .one(&txn)
                .await?;
            if taken.is_some() {
                return Err(conflict("a user with that email already exists".into()));
            }
            Some(email)
        }
        None => None,
    };
    let name = match body.name.as_deref() {
        Some(raw) => Some(normalise_name(raw)?),
        None => None,
    };

    let role_changed = body.role.is_some_and(|role| role != stored_role);
    let deactivated = body.is_active == Some(false) && stored_active;
    let loses_owner = role_changed && body.role != Some(Role::Owner) && stored_role == Role::Owner;

    if role_changed {
        ensure_not_self(&principal, &id, "change the role of")?;
    }
    if deactivated {
        ensure_not_self(&principal, &id, "deactivate")?;
    }
    if stored_active && stored_role == Role::Owner && (loses_owner || deactivated) {
        ensure_owner_remains(&txn).await?;
    }

    let mut active: user::ActiveModel = existing.into();
    if let Some(email) = email {
        active.email = Set(email);
    }
    if let Some(name) = name {
        active.name = Set(name);
    }
    if let Some(role) = body.role {
        active.role = Set(role.as_str().to_string());
    }
    if let Some(is_active) = body.is_active {
        active.is_active = Set(i64::from(is_active));
    }

    let row = active.update(&txn).await.map_err(|error| {
        if is_duplicate_email(&error) {
            conflict("a user with that email already exists".into())
        } else {
            error.into()
        }
    })?;

    audit_action(
        &txn,
        &principal,
        "user.update",
        &row.id,
        &format!("updated {}", row.email),
    )
    .await?;
    txn.commit().await?;

    if role_changed || deactivated {
        state.sessions().revoke_user(&row.id).await;
    }

    Ok(Json(UserDto::from_row(row)))
}

pub async fn remove(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let txn = state.db.begin().await?;
    let existing = find_user(&txn, &id).await?;

    ensure_not_self(&principal, &id, "delete")?;
    let is_active_owner = existing.role == Role::Owner.as_str() && existing.is_active != 0;
    if is_active_owner {
        ensure_owner_remains(&txn).await?;
    }

    let email = existing.email.clone();
    existing.delete(&txn).await?;

    audit_action(
        &txn,
        &principal,
        "user.delete",
        &id,
        &format!("deleted {email}"),
    )
    .await?;
    txn.commit().await?;

    state.sessions().revoke_user(&id).await;

    Ok(Json(json!({ "deleted": true, "id": id })))
}

pub async fn reset_password(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(id): Path<String>,
    Json(body): Json<ResetPassword>,
) -> AppResult<Json<Value>> {
    let txn = state.db.begin().await?;
    let existing = find_user(&txn, &id).await?;
    check_password(&body.password)?;

    let password_hash = hash_offloaded(body.password).await?;
    let mut active: user::ActiveModel = existing.into();
    active.password_hash = Set(password_hash);
    active.must_change_password = Set(0);
    active.update(&txn).await?;

    audit_action(
        &txn,
        &principal,
        "user.password_reset",
        &id,
        "password reset by an owner",
    )
    .await?;
    txn.commit().await?;

    state.sessions().revoke_user(&id).await;

    Ok(Json(json!({ "updated": true, "id": id })))
}

pub async fn me(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
) -> AppResult<Json<UserDto>> {
    let row = find_user(&state.db, &principal.user_id).await?;
    Ok(Json(UserDto::from_row(row)))
}

pub async fn change_own_password(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Json(body): Json<ChangePassword>,
) -> AppResult<Json<Value>> {
    let user_id = principal.user_id.clone();

    let txn = state.db.begin().await?;
    let existing = find_user(&txn, &user_id).await?;
    check_password(&body.new_password)?;

    let verified = verify_offloaded(body.current_password, existing.password_hash.clone()).await?;
    if !verified {
        return Err(AppError::Unauthorized(
            "current password is incorrect".into(),
        ));
    }

    let password_hash = hash_offloaded(body.new_password).await?;
    let mut active: user::ActiveModel = existing.into();
    active.password_hash = Set(password_hash);
    active.must_change_password = Set(0);
    active.update(&txn).await?;

    audit_action(
        &txn,
        &principal,
        "user.password_change",
        &user_id,
        "changed their own password",
    )
    .await?;
    txn.commit().await?;

    state.sessions().revoke_user(&user_id).await;

    Ok(Json(json!({ "updated": true })))
}

#[cfg(test)]
#[path = "users_tests.rs"]
mod tests;
