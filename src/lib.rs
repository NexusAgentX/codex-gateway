pub mod api;
pub mod auth;
pub mod clock;
pub mod config;
mod finalization;
mod http_error;
mod proxy;
mod routing;
pub mod secrets;
pub mod storage;
mod telemetry;
pub mod upstream;
pub mod usage;
#[cfg(feature = "embedded-frontend")]
mod web;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::{
    extract::DefaultBodyLimit,
    extract::Request,
    middleware::{self, Next},
    response::Response,
};
use http::{HeaderName, HeaderValue, Method, header};
use reqwest::Client;
use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::Instrument;
use tracing::info;

use crate::config::Config;

pub use finalization::{FinalizationDrainReport, FinalizationLifecycle, FinalizationTracker};

pub const JSON_BODY_LIMIT_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: SqlitePool,
    pub http: Client,
    pub finalizations: FinalizationTracker,
    pub clock: clock::SharedClock,
}

#[derive(Clone, Debug)]
pub struct RequestId(pub String);

pub async fn run() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    telemetry::init(&config.log_level).context("initializing telemetry")?;

    let db = storage::connect_and_migrate(&config.database_url).await?;
    storage::upgrade_legacy_upstream_secrets(&db, &config).await?;
    storage::seed_bootstrap_admin(&db, &config).await?;
    let startup_runtime = storage::runtime_config(&db, &config).await?;
    if config.retention_run_on_startup {
        let result = storage::apply_retention(
            &db,
            &storage::RetentionPolicy {
                request_log_retention_days: startup_runtime.effective.request_log_retention_days,
                daily_usage_retention_days: startup_runtime.effective.daily_usage_retention_days,
            },
        )
        .await?;
        info!(
            request_logs_deleted = result.request_logs_deleted,
            daily_usage_deleted = result.daily_usage_deleted,
            "retention policy applied"
        );
    }

    let http = Client::builder()
        .user_agent("codex-gateway/0.1")
        .build()
        .context("building reqwest client")?;

    let (finalization_lifecycle, finalizations) = FinalizationLifecycle::new();
    let state = AppState {
        config: Arc::new(config.clone()),
        db,
        http,
        finalizations,
        clock: clock::system_clock(),
    };

    let health_worker = upstream::spawn_health_worker(state.clone());
    let app = build_app(state);
    let bind: SocketAddr = config.bind.parse().context("parsing CODEX_GATEWAY_BIND")?;
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    info!(%bind, "codex-gateway listening");

    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving axum app");
    if let Some(handle) = health_worker {
        handle.abort();
        if let Err(error) = handle.await
            && !error.is_cancelled()
        {
            tracing::warn!(?error, "health worker failed during shutdown");
        }
    }
    let finalization_report = finalization_lifecycle.drain().await;
    if finalization_report.panicked_tasks > 0 {
        tracing::warn!(
            panicked_tasks = finalization_report.panicked_tasks,
            "finalization drain completed with panicked tasks"
        );
    }
    result
}

pub fn build_app(state: AppState) -> Router {
    let cors = cors_layer(&state.config);
    let app = api::router(state.clone()).merge(proxy::router(state));
    #[cfg(feature = "embedded-frontend")]
    let app = app.fallback(web::serve);

    app.layer(cors)
        .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT_BYTES))
        .layer(middleware::from_fn(request_id_middleware))
        .layer(TraceLayer::new_for_http())
}

async fn request_id_middleware(mut request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get(request_id_header())
        .and_then(|value| value.to_str().ok())
        .and_then(sanitize_request_id)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));
    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %method,
        path = %path
    );
    let mut response = next.run(request).instrument(span).await;
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(request_id_header(), value);
    }
    response
}

fn sanitize_request_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 {
        return None;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
        .then(|| value.to_string())
}

pub fn request_id_header() -> HeaderName {
    HeaderName::from_static("x-request-id")
}

fn cors_layer(config: &Config) -> CorsLayer {
    let origins: Vec<HeaderValue> = config
        .cors_allowed_origins
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
        .collect();
    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::OPTIONS])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT])
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
