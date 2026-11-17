pub mod admin;
pub mod assets;
pub mod auth;
pub mod cache;
pub mod config;
pub mod crypto;
pub mod db;
pub mod egress;
pub mod entities;
pub mod error;
pub mod guardrails;
pub mod infra;
pub mod mcp;
pub mod metrics;
pub mod migration;
pub mod observability;
pub mod prompts;
pub mod providers;
pub mod proxy;
pub mod routing;
pub mod semantic;
pub mod telemetry;
pub mod tokenizer;

use std::{
    sync::{
        atomic::{AtomicBool, AtomicI64},
        Arc,
    },
    time::Duration,
};

use axum::{
    http::{HeaderValue, StatusCode},
    routing::get,
    Json, Router,
};
use dashmap::DashMap;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde_json::{json, Value};
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{
    compression::CompressionLayer, cors::CorsLayer, limit::RequestBodyLimitLayer,
    timeout::TimeoutLayer, trace::TraceLayer,
};

use crate::{
    cache::ResponseCache,
    config::AppConfig,
    entities::app_setting,
    error::AppResult,
    routing::{CircuitBreakerRegistry, RoutingService},
    telemetry::{spawn_telemetry_worker, TelemetrySender},
    tokenizer::PricingRegistry,
};

const TEXT_EXPOSITION: &str = "text/plain; version=0.0.4; charset=utf-8";

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub config: AppConfig,
    pub http_client: reqwest::Client,
    pub telemetry: TelemetrySender,
    pub secrets: Arc<crate::crypto::SecretBox>,
    pub routing: Arc<RoutingService>,
    pub routing_policy: Arc<crate::routing::PolicyStore>,
    pub pricing: Arc<PricingRegistry>,
    pub response_cache: Arc<ResponseCache>,
    pub semantic: Arc<semantic::SemanticCache>,
    pub redact_prompts: Arc<AtomicBool>,
    pub live_spend: Arc<DashMap<String, Arc<AtomicI64>>>,
    pub auth: Arc<crate::auth::AuthService>,
    pub redis: Option<Arc<crate::infra::redis::RedisStore>>,
    pub key_pool: Arc<crate::routing::ProviderKeyPool>,
    pub ewma: Arc<crate::routing::EwmaRegistry>,
    pub affinity: Arc<crate::routing::SessionAffinity>,
    pub guardrails: Arc<crate::guardrails::GuardrailEngine>,
    pub mcp: Arc<crate::mcp::registry::McpRegistry>,
    pub metrics: Arc<crate::metrics::Metrics>,
    pub observability: Arc<crate::observability::Observability>,
    pub login_throttle: Arc<crate::auth::throttle::LoginThrottle>,
    session_store: Arc<crate::auth::session::SessionStore>,
}

impl AppState {
    pub fn new(
        db: DatabaseConnection,
        config: AppConfig,
    ) -> (Self, tokio::sync::mpsc::Receiver<telemetry::TelemetryEvent>) {
        let (telemetry, rx) = TelemetrySender::create_channel();
        let circuit = Arc::new(CircuitBreakerRegistry::new());
        let secrets = Arc::new(
            crate::crypto::SecretBox::from_env()
                .unwrap_or_else(|_| crate::crypto::SecretBox::test_key()),
        );
        let routing_policy = Arc::new(crate::routing::PolicyStore::default());
        let routing = Arc::new(RoutingService::new(
            db.clone(),
            circuit,
            secrets.clone(),
            routing_policy.clone(),
        ));
        let pricing = Arc::new(PricingRegistry::new(db.clone()));
        let live_spend = Arc::new(DashMap::new());
        let live_spend_for_auth = live_spend.clone();
        let http_client = providers::build_http_client();

        let key_pool = Arc::new(crate::routing::ProviderKeyPool::new(
            db.clone(),
            secrets.clone(),
        ));

        let state = Self {
            http_client: http_client.clone(),
            secrets,
            telemetry,
            routing,
            routing_policy,
            pricing,
            response_cache: Arc::new(ResponseCache::new()),
            semantic: Arc::new(semantic::SemanticCache::new(
                db.clone(),
                config.qdrant_url.clone(),
                config.semantic_embedding_model.clone(),
            )),
            redact_prompts: Arc::new(AtomicBool::new(false)),
            live_spend,
            auth: Arc::new(crate::auth::AuthService::new(
                db.clone(),
                live_spend_for_auth,
            )),
            redis: None,
            key_pool,
            ewma: Arc::new(crate::routing::EwmaRegistry::new()),
            affinity: Arc::new(crate::routing::SessionAffinity::new()),
            guardrails: Arc::new(crate::guardrails::GuardrailEngine::new(
                db.clone(),
                crate::guardrails::webhooks::WebhookSender::disabled(),
            )),
            mcp: Arc::new(crate::mcp::registry::McpRegistry::new(
                db.clone(),
                http_client,
            )),
            metrics: Arc::new(crate::metrics::Metrics::new()),
            observability: Arc::new(crate::observability::Observability::from_config(&config)),
            login_throttle: Arc::new(crate::auth::throttle::LoginThrottle::new()),
            session_store: Arc::new(crate::auth::session::SessionStore::new(db.clone())),
            db,
            config,
        };

        (state, rx)
    }

    pub async fn from_db_and_config(
        db: DatabaseConnection,
        config: AppConfig,
    ) -> (Self, tokio::task::JoinHandle<()>) {
        let (mut state, rx) = Self::new(db, config);
        state.redact_prompts = Arc::new(AtomicBool::new(state.load_redact_prompts().await));
        state.semantic.load_threshold(&state.db).await;
        state.semantic.load_embedding_model(&state.db).await;
        state.routing_policy.load(&state.db).await;
        state
            .routing
            .apply_policy(&state.routing_policy.get())
            .await;
        state.attach_redis_best_effort().await;
        state.enforce_egress_endpoints().await;
        if state.config.qdrant_url.is_some() {
            spawn_production_semantic_sweep(state.semantic.clone());
        }

        let (sender, receiver) = crate::guardrails::webhooks::WebhookSender::create_channel();
        tokio::spawn(crate::guardrails::webhooks::run_worker(
            state.db.clone(),
            state.http_client.clone(),
            state.secrets.clone(),
            receiver,
        ));
        state.guardrails = Arc::new(crate::guardrails::GuardrailEngine::new(
            state.db.clone(),
            sender,
        ));
        state.spawn_production_log_retention();

        let worker = spawn_telemetry_worker(
            state.db.clone(),
            rx,
            state.live_spend.clone(),
            state.metrics.clone(),
            state.observability.clone(),
            state.http_client.clone(),
        );
        (state, worker)
    }

    async fn attach_redis_best_effort(&mut self) {
        let Some(url) = self.config.redis_url.as_deref() else {
            return;
        };

        match crate::infra::redis::RedisStore::connect(url).await {
            Ok(store) => {
                if !store.ping().await {
                    tracing::warn!("redis reachable but unresponsive; continuing degraded");
                }
                let store = Arc::new(store);
                self.response_cache = Arc::new(ResponseCache::with_redis(Some(store.clone())));
                self.auth = Arc::new((*self.auth).clone().with_redis(Some(store.clone())));
                self.session_store = Arc::new(
                    crate::auth::session::SessionStore::new(self.db.clone())
                        .with_redis(Some(store.clone())),
                );
                self.redis = Some(store);
            }
            Err(err) => {
                tracing::warn!(error = %err, "redis unavailable; running without shared state");
            }
        }
    }

    async fn enforce_egress_endpoints(&mut self) {
        if let Some(url) = self.config.qdrant_url.clone() {
            if crate::egress::validate_url(&url, self.config.allow_private_upstreams)
                .await
                .is_err()
            {
                tracing::warn!("ignoring blocked QDRANT_URL; semantic cache running local-only");
                self.semantic = Arc::new(semantic::SemanticCache::new(
                    self.db.clone(),
                    None,
                    self.config.semantic_embedding_model.clone(),
                ));
                self.config.qdrant_url = None;
            }
        }
        if let Some(url) = self.config.otlp_endpoint.clone() {
            if crate::egress::validate_url(&url, self.config.allow_private_upstreams)
                .await
                .is_err()
            {
                tracing::warn!(
                    "ignoring blocked OTEL_EXPORTER_OTLP_ENDPOINT; trace export disabled"
                );
                let mut config = self.config.clone();
                config.otlp_endpoint = None;
                self.observability =
                    Arc::new(crate::observability::Observability::from_config(&config));
                self.config.otlp_endpoint = None;
            }
        }
    }

    pub fn sessions(&self) -> &Arc<crate::auth::session::SessionStore> {
        &self.session_store
    }

    fn spawn_production_log_retention(&self) {
        let Some(days) = self.config.log_retention_days.filter(|days| *days > 0) else {
            return;
        };

        let db = self.db.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(60 * 60));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                match purge_request_logs(&db, days).await {
                    Ok(0) => {}
                    Ok(deleted) => tracing::info!(deleted, days, "pruned request logs"),
                    Err(err) => tracing::warn!(error = %err, days, "request log retention failed"),
                }
            }
        });
    }

    async fn load_redact_prompts(&self) -> bool {
        let value = app_setting::Entity::find_by_id("redact_prompts")
            .one(&self.db)
            .await
            .ok()
            .flatten()
            .map(|row| row.value);

        matches!(value.as_deref(), Some("true") | Some("1"))
    }
}

fn spawn_production_semantic_sweep(semantic: Arc<semantic::SemanticCache>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(60 * 60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match semantic.sweep().await {
                Ok(0) => {}
                Ok(removed) => tracing::debug!(removed, "swept semantic cache"),
                Err(err) => tracing::warn!(error = %err, "semantic cache sweep failed"),
            }
        }
    });
}

async fn purge_request_logs(db: &DatabaseConnection, days: u64) -> AppResult<u64> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(days as i64))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let outcome = crate::entities::request_log::Entity::delete_many()
        .filter(crate::entities::request_log::Column::CreatedAt.lt(cutoff))
        .exec(db)
        .await?;

    Ok(outcome.rows_affected)
}

const PROXY_TIMEOUT_SECS: u64 = 600;
const CONTROL_TIMEOUT_SECS: u64 = 30;
const PROXY_CONCURRENCY: usize = 512;
const CONTROL_CONCURRENCY: usize = 256;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

pub fn build_app(state: AppState) -> Router {
    let cors = cors_layer(&state.config);

    let protected_proxy = proxy::router()
        .merge(mcp::router())
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_virtual_key,
        ))
        .layer(ConcurrencyLimitLayer::new(PROXY_CONCURRENCY))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_secs(PROXY_TIMEOUT_SECS),
        ));

    let guarded_admin = admin::router().route_layer(axum::middleware::from_fn_with_state(
        state.clone(),
        admin::auth::require_admin,
    ));

    let guarded_metrics = Router::new().route("/metrics", get(metrics)).route_layer(
        axum::middleware::from_fn_with_state(state.clone(), admin::auth::require_admin),
    );

    let control = Router::new()
        .route("/health", get(health))
        .merge(guarded_metrics)
        .merge(guarded_admin)
        .fallback(assets::serve_spa)
        .layer(ConcurrencyLimitLayer::new(CONTROL_CONCURRENCY))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_secs(CONTROL_TIMEOUT_SECS),
        ));

    Router::new()
        .merge(protected_proxy)
        .merge(control)
        .with_state(state)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

fn cors_layer(config: &AppConfig) -> CorsLayer {
    use tower_http::cors::Any;

    let layer = CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_credentials(false);

    let origins = config
        .cors_origins
        .iter()
        .filter_map(|origin| origin.parse::<HeaderValue>().ok())
        .collect::<Vec<_>>();

    layer.allow_origin(origins)
}

async fn metrics(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl axum::response::IntoResponse {
    let body = state.metrics.snapshot(&state).await;
    ([(axum::http::header::CONTENT_TYPE, TEXT_EXPOSITION)], body)
}

async fn health(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> (StatusCode, Json<Value>) {
    let db_ok = state.db.ping().await.is_ok();
    if !state.config.health_detailed {
        return match db_ok {
            true => (StatusCode::OK, Json(json!({ "status": "ok" }))),
            false => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "degraded" })),
            ),
        };
    }
    let redis = match state.config.redis_url.is_some() {
        false => None,
        true => Some(match state.redis.as_ref() {
            Some(store) if store.is_healthy() => "connected",
            _ => "degraded",
        }),
    };

    let mut body = json!({ "status": if db_ok { "ok" } else { "degraded" } });
    if !db_ok {
        body["database"] = json!("unavailable");
    }
    if let Some(redis) = redis {
        body["redis"] = json!(redis);
    }

    if db_ok {
        (StatusCode::OK, Json(body))
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(body))
    }
}

pub fn init_tracing(filter: &str) {
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let env_filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer())
        .try_init();
}
