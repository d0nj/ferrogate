use std::sync::atomic::{AtomicU64, Ordering};

use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;

use super::*;

const NOW: u64 = 1_750_000_000;

fn ensure_test_key() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::env::set_var(
            "FERROGATE_ENCRYPTION_KEY",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        );
    });
}

async fn database(url: &str) -> DatabaseConnection {
    ensure_test_key();
    if let Some(path) = url.strip_prefix("sqlite://") {
        if path != ":memory:" {
            std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(path)
                .expect("create database file");
        }
    }

    let mut options = ConnectOptions::new(url);
    options.max_connections(1);
    let db = Database::connect(options).await.expect("sqlite");
    crate::migration::Migrator::up(&db, None)
        .await
        .expect("migrate");
    db
}

async fn memory_db() -> DatabaseConnection {
    database("sqlite::memory:").await
}

async fn store() -> SessionStore {
    SessionStore::new(memory_db().await)
}

async fn fixed_store(now: Arc<AtomicU64>) -> SessionStore {
    let store = SessionStore::new(memory_db().await);
    store.set_clock(move || now.load(Ordering::Relaxed));
    store
}

async fn row_exists(db: &DatabaseConnection, token: &str) -> bool {
    row::Entity::find_by_id(digest_token(token))
        .one(db)
        .await
        .expect("query sessions")
        .is_some()
}

async fn row_is_raw(db: &DatabaseConnection, token: &str) -> bool {
    row::Entity::find_by_id(token)
        .one(db)
        .await
        .expect("query sessions")
        .is_some()
}

#[tokio::test]
async fn mint_then_get_returns_identity() {
    let store = store().await;
    let session = store
        .mint("user-1".into(), "ops@example.com".into(), Role::Owner)
        .await;

    assert!(session.token.starts_with(TOKEN_PREFIX));
    assert!(session.expires_at > session.issued_at);
    assert!(session.expires_at >= unix_now());

    let fetched = store.get(&session.token).await.expect("session must live");
    assert_eq!(fetched.user_id, "user-1");
    assert_eq!(fetched.email, "ops@example.com");
    assert_eq!(fetched.role, Role::Owner);
    assert_eq!(fetched.expires_at, session.expires_at);
    assert_eq!(fetched.token, digest_token(&session.token));
    assert!(!row_is_raw(&store.db, &session.token).await);
}

#[tokio::test]
async fn clock_past_ttl_expires_session() {
    let now = Arc::new(AtomicU64::new(NOW));
    let store = fixed_store(Arc::clone(&now)).await;
    let session = store
        .mint("user-1".into(), "ops@example.com".into(), Role::Owner)
        .await;
    assert_eq!(session.expires_at, NOW + SESSION_TTL_SECS);
    assert!(store.get(&session.token).await.is_some());

    now.store(NOW + SESSION_TTL_SECS, Ordering::Relaxed);
    assert!(store.get(&session.token).await.is_none());
    assert!(store.get(&session.token).await.is_none());
}

#[tokio::test]
async fn expired_session_is_not_returned_and_its_row_is_gone() {
    let now = Arc::new(AtomicU64::new(NOW));
    let db = memory_db().await;
    let store = SessionStore::new(db.clone());
    store.set_clock({
        let now = Arc::clone(&now);
        move || now.load(Ordering::Relaxed)
    });

    let session = store
        .mint("user-1".into(), "ops@example.com".into(), Role::Viewer)
        .await;
    assert!(row_exists(&db, &session.token).await);

    now.store(NOW + SESSION_TTL_SECS + 1, Ordering::Relaxed);
    assert!(store.get(&session.token).await.is_none());
    assert!(
        !row_exists(&db, &session.token).await,
        "reading an expired session must delete its row"
    );
}

#[tokio::test]
async fn session_survives_a_store_rebuilt_over_the_same_file() {
    let path = std::env::temp_dir().join(format!("ferrogate-sessions-{}.db", uuid::Uuid::new_v4()));
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));

    let first = SessionStore::new(database(&url).await);
    let session = first
        .mint("user-1".into(), "ops@example.com".into(), Role::Admin)
        .await;
    drop(first);

    let restarted = SessionStore::new(database(&url).await);
    let recovered = restarted
        .get(&session.token)
        .await
        .expect("a minted session must survive a restart");
    assert_eq!(recovered.user_id, session.user_id);
    assert_eq!(recovered.email, session.email);
    assert_eq!(recovered.role, session.role);
    assert_eq!(recovered.expires_at, session.expires_at);

    drop(restarted);
    let _ = std::fs::remove_file(&path);
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[tokio::test]
async fn revoke_kills_session_and_removes_the_row() {
    let db = memory_db().await;
    let store = SessionStore::new(db.clone());
    let session = store
        .mint("user-1".into(), "ops@example.com".into(), Role::Admin)
        .await;

    store.revoke(&session.token).await;
    assert!(store.get(&session.token).await.is_none());
    assert!(!row_exists(&db, &session.token).await);
}

#[tokio::test]
async fn revoke_user_kills_every_session_for_that_user_only() {
    let db = memory_db().await;
    let store = SessionStore::new(db.clone());
    let first = store
        .mint("user-1".into(), "ops@example.com".into(), Role::Admin)
        .await;
    let second = store
        .mint("user-1".into(), "ops@example.com".into(), Role::Admin)
        .await;
    let other = store
        .mint("user-2".into(), "guest@example.com".into(), Role::Viewer)
        .await;

    store.revoke_user("user-1").await;

    assert!(store.get(&first.token).await.is_none());
    assert!(store.get(&second.token).await.is_none());
    assert!(store.get(&other.token).await.is_some());
    assert!(!row_exists(&db, &first.token).await);
    assert!(row_exists(&db, &other.token).await);
}

#[tokio::test]
async fn prune_expired_drops_only_stale_rows() {
    let now = Arc::new(AtomicU64::new(NOW));
    let store = fixed_store(Arc::clone(&now)).await;
    let live = store
        .mint("user-1".into(), "ops@example.com".into(), Role::Admin)
        .await;

    now.store(NOW + SESSION_TTL_SECS + 1, Ordering::Relaxed);
    assert_eq!(store.prune_expired().await, 1);
    assert!(store.get(&live.token).await.is_none());
}

#[tokio::test]
async fn tokens_are_unique_and_prefixed() {
    let store = store().await;
    let mut tokens = std::collections::HashSet::new();

    for _ in 0..100 {
        let session = store
            .mint("user-1".into(), "ops@example.com".into(), Role::Owner)
            .await;
        assert!(session.token.starts_with(TOKEN_PREFIX));
        assert_eq!(session.token.len(), TOKEN_PREFIX.len() + 64);
        assert!(tokens.insert(session.token));
    }
}

#[test]
fn principals_expose_identity_and_grants() {
    let viewer = Principal {
        user_id: "user-1".into(),
        email: "ops@example.com".into(),
        role: Role::Viewer,
        must_change_password: false,
    };
    assert_eq!(viewer.label(), "ops@example.com");
    assert_eq!(viewer.user_id, "user-1");
    assert_eq!(viewer.role, Role::Viewer);
    assert!(!viewer.can(Capability::ManageUsers));

    let owner = Principal {
        role: Role::Owner,
        ..viewer
    };
    assert!(owner.can(Capability::ManageUsers));
}
