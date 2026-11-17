use serde_json::{json, Value};

use crate::{
    auth::{
        password::{hash_password, verify_password},
        role::Role,
        session::Principal,
    },
    entities::user,
    AppState,
};

use super::*;

fn ensure_test_key() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::env::set_var(
            "FERROGATE_ENCRYPTION_KEY",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        );
    });
}

struct Harness {
    state: AppState,
    db: sea_orm::DatabaseConnection,
}

type Reply = (axum::http::StatusCode, Value);

trait BodyJson {
    fn body_json(self) -> Value;
}

impl<T: serde::Serialize> BodyJson for Json<T> {
    fn body_json(self) -> Value {
        serde_json::to_value(self.0).expect("serialize")
    }
}

fn status_of<T: BodyJson>(result: AppResult<T>) -> Reply {
    match result {
        Ok(value) => (axum::http::StatusCode::OK, value.body_json()),
        Err(error) => (error.status(), error.envelope()),
    }
}

fn status_of_created<T: BodyJson>(result: AppResult<(axum::http::StatusCode, T)>) -> Reply {
    match result {
        Ok((status, value)) => (status, value.body_json()),
        Err(error) => (error.status(), error.envelope()),
    }
}

impl Harness {
    async fn start() -> Self {
        ensure_test_key();
        let db = memory_db().await;
        let (state, _worker) = AppState::new(db.clone(), crate::config::AppConfig::default());
        Self { state, db }
    }

    async fn get_users(&self) -> Reply {
        status_of(list(State(self.state.clone())).await)
    }

    async fn create(&self, actor: &Principal, body: Value) -> Reply {
        status_of_created(
            create(
                State(self.state.clone()),
                CurrentPrincipal(actor.clone()),
                Json(serde_json::from_value(body).expect("create body")),
            )
            .await,
        )
    }

    async fn update(&self, actor: &Principal, id: &str, body: Value) -> Reply {
        status_of(
            update(
                State(self.state.clone()),
                CurrentPrincipal(actor.clone()),
                Path(id.to_string()),
                Json(serde_json::from_value(body).expect("update body")),
            )
            .await,
        )
    }

    async fn delete_user(&self, actor: &Principal, id: &str) -> Reply {
        status_of(
            remove(
                State(self.state.clone()),
                CurrentPrincipal(actor.clone()),
                Path(id.to_string()),
            )
            .await,
        )
    }

    async fn reset_password(&self, actor: &Principal, id: &str, password: &str) -> Reply {
        status_of(
            reset_password(
                State(self.state.clone()),
                CurrentPrincipal(actor.clone()),
                Path(id.to_string()),
                Json(ResetPassword {
                    password: password.to_string(),
                }),
            )
            .await,
        )
    }

    async fn change_password(&self, actor: &Principal, current: &str, new: &str) -> Reply {
        status_of(
            change_own_password(
                State(self.state.clone()),
                CurrentPrincipal(actor.clone()),
                Json(ChangePassword {
                    current_password: current.to_string(),
                    new_password: new.to_string(),
                }),
            )
            .await,
        )
    }

    async fn me(&self, actor: &Principal) -> Reply {
        status_of(me(State(self.state.clone()), CurrentPrincipal(actor.clone())).await)
    }

    async fn seed(&self, id: &str, email: &str, role: Role, is_active: bool) -> user::Model {
        use sea_orm::{ActiveModelTrait, Set};

        user::ActiveModel {
            id: Set(id.to_string()),
            email: Set(email.to_string()),
            name: Set(email.to_string()),
            password_hash: Set(hash_password("seeded-Password-9").expect("hash")),
            role: Set(role.as_str().to_string()),
            is_active: Set(i64::from(is_active)),
            must_change_password: Set(0),
            created_at: Set("2026-01-01T00:00:00.000Z".to_string()),
            last_login_at: Set(None),
        }
        .insert(&self.db)
        .await
        .expect("seed user")
    }

    async fn stored(&self, id: &str) -> Option<user::Model> {
        user::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .expect("query")
    }

    async fn stored_by_email(&self, email: &str) -> Option<user::Model> {
        user::Entity::find()
            .filter(user::Column::Email.eq(email))
            .one(&self.db)
            .await
            .expect("query")
    }

    async fn active_owner_count(&self) -> u64 {
        use sea_orm::PaginatorTrait;

        user::Entity::find()
            .filter(user::Column::Role.eq(Role::Owner.as_str()))
            .filter(user::Column::IsActive.eq(1))
            .count(&self.db)
            .await
            .expect("count owners")
    }
}

async fn memory_db() -> sea_orm::DatabaseConnection {
    ensure_test_key();
    use sea_orm::{ConnectOptions, Database};
    use sea_orm_migration::MigratorTrait;

    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1);
    let db = Database::connect(options).await.expect("in-memory sqlite");
    crate::migration::Migrator::up(&db, None)
        .await
        .expect("migrate");
    db
}

const SEEDED_OWNER_EMAIL: &str = "admin@ferrogate.local";

fn owner(id: &str) -> Principal {
    Principal {
        user_id: id.to_string(),
        email: format!("{id}@example.com"),
        role: Role::Owner,
        must_change_password: false,
    }
}

fn strong_password() -> &'static str {
    "Correct-Horse-9"
}

#[tokio::test]
async fn create_list_update_delete_round_trip() {
    let harness = Harness::start().await;
    let actor = owner("owner-1");

    let (status, created) = harness
        .create(
            &actor,
            json!({
                "email": "  New.Operator@Example.COM ",
                "name": " New Operator ",
                "password": strong_password(),
                "role": "operator",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "body={created}");
    let created = &created["user"];
    assert_eq!(created["email"], "new.operator@example.com");
    assert_eq!(created["name"], "New Operator");
    assert_eq!(created["role"], "operator");
    assert_eq!(created["is_active"], true);
    assert!(created["created_at"].is_string());
    assert!(created["last_login_at"].is_null());
    assert!(created.get("password_hash").is_none());

    let id = created["id"].as_str().expect("id").to_string();

    let (status, listed) = harness.get_users().await;
    assert_eq!(status, StatusCode::OK);
    let emails: Vec<&str> = listed["users"]
        .as_array()
        .expect("users")
        .iter()
        .map(|row| row["email"].as_str().unwrap())
        .collect();
    assert!(emails.contains(&"new.operator@example.com"));
    assert!(emails.contains(&SEEDED_OWNER_EMAIL));
    let serialized = listed.to_string();
    assert!(!serialized.contains("password_hash"), "{serialized}");
    assert!(
        !serialized.contains("$argon2"),
        "no hash leaks: {serialized}"
    );

    let (status, updated) = harness
        .update(
            &actor,
            &id,
            json!({ "name": "Renamed", "email": "renamed@example.com", "role": "admin" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body={updated}");
    assert_eq!(updated["name"], "Renamed");
    assert_eq!(updated["email"], "renamed@example.com");
    assert_eq!(updated["role"], "admin");
    assert!(updated.get("password_hash").is_none());
    assert!(!updated.to_string().contains("$argon2"));

    let (status, deleted) = harness.delete_user(&actor, &id).await;
    assert_eq!(status, StatusCode::OK, "body={deleted}");
    assert_eq!(deleted["deleted"], true);
    assert!(harness.stored(&id).await.is_none());
}

#[tokio::test]
async fn duplicate_email_is_a_conflict() {
    let harness = Harness::start().await;
    let actor = owner("owner-1");

    harness
        .seed("existing", "taken@example.com", Role::Viewer, true)
        .await;

    let (status, body) = harness
        .create(
            &actor,
            json!({
                "email": "  TAKEN@example.com ",
                "name": "Second",
                "password": strong_password(),
                "role": "viewer",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("already exists"));

    harness
        .seed("other", "other@example.com", Role::Viewer, true)
        .await;
    let (status, _) = harness
        .update(&actor, "other", json!({ "email": "taken@example.com" }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn weak_passwords_are_rejected() {
    let harness = Harness::start().await;
    let actor = owner("owner-1");

    for password in ["Sh0rt!", "ferrogate", "plainlowercasepassword"] {
        let (status, body) = harness
            .create(
                &actor,
                json!({
                    "email": format!("weak-{password}@example.com"),
                    "name": "Weak",
                    "password": password,
                    "role": "viewer",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{password}: {body}");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .starts_with("Password"),
            "{password}: {body}"
        );
    }

    let (status, body) = harness
        .create(
            &actor,
            json!({
                "email": "bootstrap@example.com",
                "name": "Weak",
                "password": "ferrogate",
                "role": "viewer",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"]["message"],
        "Password must be at least 12 characters."
    );
}

#[tokio::test]
async fn invalid_email_and_name_are_rejected() {
    let harness = Harness::start().await;
    let actor = owner("owner-1");

    for (email, name) in [
        ("", "Name"),
        ("no-at-sign", "Name"),
        ("ok@example.com", "  "),
    ] {
        let (status, body) = harness
            .create(
                &actor,
                json!({
                    "email": email,
                    "name": name,
                    "password": strong_password(),
                    "role": "viewer",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{email}/{name}: {body}");
    }

    let long_email = format!("{}@example.com", "a".repeat(250));
    let (status, _) = harness
        .create(
            &actor,
            json!({
                "email": long_email,
                "name": "Too Long",
                "password": strong_password(),
                "role": "viewer",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn accepting_the_last_owner_is_refused() {
    let harness = Harness::start().await;
    let seeded = harness
        .stored_by_email(SEEDED_OWNER_EMAIL)
        .await
        .expect("seeded owner");
    let actor = owner(&seeded.id);

    assert_eq!(harness.active_owner_count().await, 1);

    let (status, body) = harness.delete_user(&actor, &seeded.id).await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(harness.active_owner_count().await, 1);

    let (status, body) = harness
        .update(&actor, &seeded.id, json!({ "is_active": false }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(harness.active_owner_count().await, 1);

    let (status, body) = harness
        .update(&actor, &seeded.id, json!({ "role": "admin" }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(harness.active_owner_count().await, 1);

    let (status, body) = harness
        .update(&actor, &seeded.id, json!({ "name": "Bootstrap Owner" }))
        .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
}

#[tokio::test]
async fn a_second_owner_allows_demoting_the_first() {
    let harness = Harness::start().await;
    let seeded = harness
        .stored_by_email(SEEDED_OWNER_EMAIL)
        .await
        .expect("seeded owner");
    let second = harness
        .seed("owner-2", "owner2@example.com", Role::Owner, true)
        .await;
    assert_eq!(harness.active_owner_count().await, 2);

    let (status, body) = harness
        .update(&owner(&second.id), &seeded.id, json!({ "role": "admin" }))
        .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["role"], "admin");
    assert_eq!(harness.active_owner_count().await, 1);

    let (status, _) = harness.delete_user(&owner(&second.id), &seeded.id).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the demoted account is now deletable"
    );
}

#[tokio::test]
async fn self_lockout_is_refused_even_with_another_owner() {
    let harness = Harness::start().await;
    harness
        .seed("owner-2", "owner2@example.com", Role::Owner, true)
        .await;
    assert_eq!(harness.active_owner_count().await, 2);

    let actor = owner("owner-2");
    let (status, body) = harness
        .update(&actor, "owner-2", json!({ "is_active": false }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");

    let (status, body) = harness.delete_user(&actor, "owner-2").await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");

    let (status, body) = harness
        .update(&actor, "owner-2", json!({ "role": "admin" }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");

    let row = harness.stored("owner-2").await.expect("still present");
    assert_eq!(row.role, "owner");
    assert_eq!(row.is_active, 1);
}

#[tokio::test]
async fn a_self_edit_that_is_not_a_demotion_is_allowed() {
    let harness = Harness::start().await;
    harness
        .seed("owner-2", "owner2@example.com", Role::Owner, true)
        .await;

    let (status, body) = harness
        .update(
            &owner("owner-2"),
            "owner-2",
            json!({ "name": "Renamed Myself", "role": "owner" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["name"], "Renamed Myself");
}

#[tokio::test]
async fn role_change_revokes_live_sessions() {
    let harness = Harness::start().await;
    harness
        .seed("owner-2", "owner2@example.com", Role::Owner, true)
        .await;
    harness
        .seed("target", "target@example.com", Role::Admin, true)
        .await;

    let session = harness
        .state
        .sessions()
        .mint(
            "target".to_string(),
            "target@example.com".to_string(),
            Role::Admin,
        )
        .await;
    assert!(harness.state.sessions().get(&session.token).await.is_some());

    let (status, _) = harness
        .update(&owner("owner-2"), "target", json!({ "role": "viewer" }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        harness.state.sessions().get(&session.token).await.is_none(),
        "role change must kill the old token"
    );
}

#[tokio::test]
async fn deactivation_reset_and_delete_revoke_live_sessions() {
    let harness = Harness::start().await;
    harness
        .seed("owner-2", "owner2@example.com", Role::Owner, true)
        .await;
    harness
        .seed("target", "target@example.com", Role::Admin, true)
        .await;
    let actor = owner("owner-2");

    let deactivate = harness
        .state
        .sessions()
        .mint(
            "target".to_string(),
            "target@example.com".to_string(),
            Role::Admin,
        )
        .await;
    let (status, _) = harness
        .update(&actor, "target", json!({ "is_active": false }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(harness
        .state
        .sessions()
        .get(&deactivate.token)
        .await
        .is_none());

    let reset = harness
        .state
        .sessions()
        .mint(
            "target".to_string(),
            "target@example.com".to_string(),
            Role::Admin,
        )
        .await;
    let (status, _) = harness
        .reset_password(&actor, "target", "Another-Secret-9")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(harness.state.sessions().get(&reset.token).await.is_none());
    let row = harness.stored("target").await.expect("row");
    assert!(verify_password("Another-Secret-9", &row.password_hash));

    let deleted = harness
        .state
        .sessions()
        .mint(
            "target".to_string(),
            "target@example.com".to_string(),
            Role::Admin,
        )
        .await;
    let (status, _) = harness.delete_user(&actor, "target").await;
    assert_eq!(status, StatusCode::OK);
    assert!(harness.state.sessions().get(&deleted.token).await.is_none());
    assert!(harness.stored("target").await.is_none());
}

#[tokio::test]
async fn self_service_change_requires_the_current_password() {
    let harness = Harness::start().await;
    harness
        .seed("owner-1", "owner1@example.com", Role::Owner, true)
        .await;
    let actor = owner("owner-1");

    let (status, body) = harness
        .change_password(&actor, "wrong-password", "Brand-New-Secret-9")
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");

    let (status, body) = harness
        .change_password(&actor, "seeded-Password-9", "Brand-New-Secret-9")
        .await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let row = harness.stored("owner-1").await.expect("row");
    assert!(verify_password("Brand-New-Secret-9", &row.password_hash));
    assert!(!verify_password("seeded-Password-9", &row.password_hash));

    let (status, _) = harness
        .change_password(&actor, "Brand-New-Secret-9", "ferrogate")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "policy still applies");
}

#[tokio::test]
async fn self_service_change_only_touches_the_caller() {
    let harness = Harness::start().await;
    let other = harness
        .seed("other", "other@example.com", Role::Owner, true)
        .await;
    harness
        .seed("owner-1", "owner1@example.com", Role::Owner, true)
        .await;

    let (status, _) = harness
        .change_password(&owner("owner-1"), "seeded-Password-9", "Brand-New-Secret-9")
        .await;
    assert_eq!(status, StatusCode::OK);

    let after = harness.stored("other").await.expect("other");
    assert_eq!(other.password_hash, after.password_hash);
}

#[tokio::test]
async fn me_renders_the_caller_row() {
    let harness = Harness::start().await;
    let seeded = harness
        .stored_by_email(SEEDED_OWNER_EMAIL)
        .await
        .expect("seeded owner");
    harness
        .seed("viewer-1", "viewer@example.com", Role::Viewer, true)
        .await;

    let (status, body) = harness.me(&owner(&seeded.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], seeded.id);
    assert_eq!(body["email"], SEEDED_OWNER_EMAIL);
    assert_eq!(body["role"], "owner");
    assert!(body.get("password_hash").is_none());

    let (status, body) = harness.me(&owner("viewer-1")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], "viewer-1");
    assert_eq!(body["email"], "viewer@example.com");
    assert_eq!(body["role"], "viewer");
    assert_eq!(body["is_active"], true);
}

#[tokio::test]
async fn me_rejects_a_principal_without_a_row() {
    let harness = Harness::start().await;

    let (status, _) = harness.me(&owner("ghost")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = harness
        .change_password(&owner("ghost"), "x", "Brand-New-Secret-9")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_ids_are_not_found() {
    let harness = Harness::start().await;
    let actor = owner("owner-1");

    for (status, _) in [
        harness
            .update(&actor, "ghost", json!({ "name": "x" }))
            .await,
        harness.delete_user(&actor, "ghost").await,
        harness
            .reset_password(&actor, "ghost", strong_password())
            .await,
    ] {
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn mutations_are_audited() {
    let harness = Harness::start().await;
    let actor = owner("owner-1");

    let (_, created) = harness
        .create(
            &actor,
            json!({
                "email": "audited@example.com",
                "name": "Audited",
                "password": strong_password(),
                "role": "viewer",
            }),
        )
        .await;
    let id = created["user"]["id"].as_str().expect("id").to_string();
    let (_, deleted) = harness.delete_user(&actor, &id).await;
    assert_eq!(deleted["deleted"], true);

    let actions = |target: &str| {
        let db = harness.db.clone();
        let target = target.to_string();
        async move {
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

            crate::entities::audit_log::Entity::find()
                .filter(crate::entities::audit_log::Column::Target.eq(target))
                .all(&db)
                .await
                .expect("audit rows")
                .into_iter()
                .map(|row| row.action)
                .collect::<Vec<String>>()
        }
    };

    let mut recorded = actions(&id).await;
    recorded.sort();
    assert_eq!(
        recorded,
        ["user.create".to_string(), "user.delete".to_string()],
        "audit table is {recorded:?}"
    );
}

fn login_parts(client: &str) -> axum::http::request::Parts {
    let mut request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/admin/api/session/login")
        .body(())
        .expect("request");
    request
        .headers_mut()
        .insert("sec-fetch-site", "same-origin".parse().unwrap());
    request
        .headers_mut()
        .insert("x-forwarded-for", client.parse().unwrap());
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::new(
            client.parse().unwrap(),
            50000,
        )));
    request.into_parts().0
}

#[tokio::test]
async fn login_stamps_last_login() {
    let harness = Harness::start().await;
    let row = harness
        .seed("login", "login@example.com", Role::Viewer, true)
        .await;
    assert!(row.last_login_at.is_none());

    let reply = super::super::auth::login(
        State(harness.state.clone()),
        login_parts("198.51.100.20"),
        Json(super::super::auth::LoginRequest {
            email: Some("login@example.com".to_string()),
            password: Some("seeded-Password-9".to_string()),
        }),
    )
    .await;
    assert!(reply.is_ok(), "login should succeed");

    let stamped = harness.stored("login").await.expect("row");
    assert!(stamped.last_login_at.is_some());
}

#[tokio::test]
async fn login_rejects_inactive_accounts() {
    let harness = Harness::start().await;
    harness
        .seed("off", "off@example.com", Role::Viewer, false)
        .await;

    let reply = super::super::auth::login(
        State(harness.state.clone()),
        login_parts("198.51.100.21"),
        Json(super::super::auth::LoginRequest {
            email: Some("off@example.com".to_string()),
            password: Some("seeded-Password-9".to_string()),
        }),
    )
    .await;

    assert!(reply.is_err(), "inactive account must not log in");
    assert!(harness
        .stored("off")
        .await
        .expect("row")
        .last_login_at
        .is_none());
}
