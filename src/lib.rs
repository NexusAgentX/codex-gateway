pub mod api;
pub mod auth;
pub mod config;
pub mod proxy;
pub mod routing;
pub mod storage;
pub mod telemetry;
pub mod upstream;
pub mod usage;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use reqwest::Client;
use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: SqlitePool,
    pub http: Client,
}

pub async fn run() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    telemetry::init(&config.log_level);

    let db = storage::connect_and_migrate(&config.database_url).await?;
    storage::seed_bootstrap_admin(&db, &config).await?;

    let http = Client::builder()
        .user_agent("codex-gateway/0.1")
        .build()
        .context("building reqwest client")?;

    let state = AppState {
        config: Arc::new(config.clone()),
        db,
        http,
    };

    let app = build_app(state);
    let bind: SocketAddr = config.bind.parse().context("parsing CODEX_GATEWAY_BIND")?;
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    info!(%bind, "codex-gateway listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving axum app")
}

pub fn build_app(state: AppState) -> Router {
    api::router(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
