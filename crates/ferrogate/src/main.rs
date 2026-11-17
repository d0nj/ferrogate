use std::process::ExitCode;

use ferrogate::{build_app, config::AppConfig, db, error::AppResult, init_tracing, AppState};

#[tokio::main]
async fn main() -> ExitCode {
    let config = AppConfig::from_env();
    init_tracing(&config.log_filter);

    match run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %err, "ferrogate terminated");
            ExitCode::FAILURE
        }
    }
}

async fn run(config: AppConfig) -> AppResult<()> {
    ferrogate::crypto::SecretBox::from_env()
        .map_err(|err| ferrogate::error::AppError::BadRequest(err.to_string()))?;

    tracing::info!(database_url = %config.database_url, "initialising database");

    let db = db::init_db(&config.database_url).await?;
    let (state, telemetry_worker) = AppState::from_db_and_config(db.clone(), config.clone()).await;
    let telemetry_tx = state.telemetry.clone();
    let app = build_app(state);

    let addr = config.socket_addr();
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!("ferrogate listening on http://{addr}");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    tracing::info!("shutdown requested; flushing telemetry");

    drop(telemetry_tx);
    match tokio::time::timeout(std::time::Duration::from_secs(10), telemetry_worker).await {
        Ok(Ok(())) => tracing::info!("telemetry flushed"),
        Ok(Err(err)) => tracing::error!(error = %err, "telemetry worker panicked"),
        Err(_) => tracing::warn!("telemetry flush timed out"),
    }

    if let Err(err) = db.close().await {
        tracing::warn!(error = %err, "database close failed");
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %err, "failed to install Ctrl+C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => tracing::error!(error = %err, "failed to install SIGTERM handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
