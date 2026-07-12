#![allow(dead_code, unused_imports)]

pub use std::{
    convert::Infallible,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

pub use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Json,
    http::{Request, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
pub use chrono::{DateTime, Duration as ChronoDuration, Utc};
pub use codex_gateway::{
    AppState, FinalizationLifecycle, FinalizationTracker, JSON_BODY_LIMIT_BYTES, auth, build_app,
    clock::{Clock, SharedClock, system_clock},
    config::{
        Config, RUNTIME_CONFIG_DESCRIPTORS, RouteStrategy, RuntimeConfigKey,
        default_request_timeout_ms,
    },
    routing,
    storage::{
        self, CreateApiKey, CreateUser, RequestLogInsert, UpsertModel, UpsertModelMapping,
        UpsertUpstream,
    },
    usage::UsageSnapshot,
};
pub use futures_util::{Stream, StreamExt};
pub use serde_json::{Value, json};
pub use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
pub use tower::ServiceExt;

#[derive(Clone)]
pub struct TestClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

impl TestClock {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }

    pub fn advance(&self, duration: ChronoDuration) {
        let mut now = self.now.lock().unwrap();
        *now += duration;
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap()
    }
}

pub struct TestAppBuilder {
    upstream_url: Option<String>,
    clock: SharedClock,
}

impl Default for TestAppBuilder {
    fn default() -> Self {
        Self {
            upstream_url: None,
            clock: system_clock(),
        }
    }
}

impl TestAppBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upstream(mut self, upstream_url: impl Into<String>) -> Self {
        self.upstream_url = Some(upstream_url.into());
        self
    }

    pub fn clock(mut self, clock: impl Clock + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    pub async fn build(self) -> (Router, String, SqlitePool) {
        let (app, key, pool, _lifecycle) = self.build_tracked().await;
        (app, key, pool)
    }

    pub async fn build_tracked(self) -> (Router, String, SqlitePool, FinalizationLifecycle) {
        let pool = storage::connect_and_migrate("sqlite://:memory:")
            .await
            .unwrap();
        let config = test_config();
        let user_id = seed_user_model(&pool, self.upstream_url.as_deref()).await;
        let (_, plaintext) = storage::create_api_key(
            &pool,
            &config.app_secret,
            &user_id,
            &CreateApiKey {
                name: "test".into(),
                expires_at: None,
            },
        )
        .await
        .unwrap();
        let (lifecycle, finalizations) = FinalizationLifecycle::new();
        let state = AppState {
            config: Arc::new(config),
            db: pool.clone(),
            http: reqwest::Client::new(),
            finalizations,
            clock: self.clock,
        };
        (build_app(state), plaintext, pool, lifecycle)
    }
}

pub async fn test_app(upstream_url: Option<&str>) -> (Router, String) {
    let (app, key, _) = test_app_with_pool(upstream_url).await;
    (app, key)
}

pub async fn test_app_with_pool(upstream_url: Option<&str>) -> (Router, String, SqlitePool) {
    let builder = upstream_url.map_or_else(TestAppBuilder::new, |url| {
        TestAppBuilder::new().upstream(url)
    });
    builder.build().await
}

pub async fn tracked_test_app_with_pool(
    upstream_url: Option<&str>,
) -> (Router, String, SqlitePool, FinalizationLifecycle) {
    let builder = upstream_url.map_or_else(TestAppBuilder::new, |url| {
        TestAppBuilder::new().upstream(url)
    });
    builder.build_tracked().await
}

pub async fn await_finalizations(lifecycle: &FinalizationLifecycle, expected: u64) {
    tokio::time::timeout(
        Duration::from_secs(5),
        lifecycle.wait_for_completed_tasks(expected),
    )
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {expected} finalization tasks"));
}

pub async fn seeded_user_and_key_ids(pool: &SqlitePool) -> (String, String) {
    let key = storage::list_api_keys(pool).await.unwrap().remove(0);
    (key.user_id, key.id)
}

pub async fn assert_no_api_key_limit_policies(pool: &SqlitePool) {
    for key in storage::list_api_keys(pool).await.unwrap() {
        assert!(
            storage::get_limit_policy(pool, "api_key", &key.id)
                .await
                .unwrap()
                .is_none()
        );
    }
}

pub async fn app_with_single_upstream_timeout(
    upstream_url: &str,
    timeout_ms: Option<i64>,
) -> (Router, String, SqlitePool) {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let config = test_config();
    let user_id = storage::ensure_user(
        &pool,
        &CreateUser {
            email: "user@example.com".into(),
            password: "password".into(),
            role: "admin".into(),
            display_name: Some("Test User".into()),
        },
    )
    .await
    .unwrap();
    let (_, plaintext) = storage::create_api_key(
        &pool,
        &config.app_secret,
        &user_id,
        &CreateApiKey {
            name: "test".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();
    let upstream = storage::create_upstream(
        &pool,
        &config.app_secret,
        config.secret_key_version,
        &UpsertUpstream {
            name: "single".into(),
            base_url: upstream_url.into(),
            api_key: "sk-single".into(),
            enabled: Some(true),
            priority: Some(1),
            weight: Some(1),
            timeout_ms: timeout_ms.map_or_else(timeout_default, timeout_explicit),
            max_retries: Some(0),
            health_check_path: None,
        },
    )
    .await
    .unwrap();
    storage::create_model(
        &pool,
        &UpsertModel {
            public_name: "codex-mini".into(),
            description: Some("test model".into()),
            enabled: Some(true),
            visible_to_users: Some(true),
            upstream_mappings: Some(vec![UpsertModelMapping {
                upstream_id: upstream.id,
                upstream_model_name: "upstream-codex-mini".into(),
                enabled: Some(true),
                priority: Some(1),
                weight: Some(1),
            }]),
        },
    )
    .await
    .unwrap();

    let state = AppState {
        config: Arc::new(config),
        db: pool.clone(),
        http: reqwest::Client::new(),
        finalizations: FinalizationTracker::default(),
        clock: codex_gateway::clock::system_clock(),
    };
    (build_app(state), plaintext, pool)
}

pub async fn app_with_two_upstreams(
    first_url: &str,
    second_url: &str,
) -> (Router, String, SqlitePool) {
    app_with_two_upstreams_and_retries(first_url, second_url, 1).await
}

pub async fn app_with_two_upstreams_and_retries(
    first_url: &str,
    second_url: &str,
    first_max_retries: i64,
) -> (Router, String, SqlitePool) {
    app_with_two_upstreams_and_retries_timeout(first_url, second_url, first_max_retries, 5_000)
        .await
}

pub async fn app_with_two_upstreams_and_retries_timeout(
    first_url: &str,
    second_url: &str,
    first_max_retries: i64,
    first_timeout_ms: i64,
) -> (Router, String, SqlitePool) {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let config = test_config();
    let user_id = storage::ensure_user(
        &pool,
        &CreateUser {
            email: "user@example.com".into(),
            password: "password".into(),
            role: "admin".into(),
            display_name: Some("Test User".into()),
        },
    )
    .await
    .unwrap();
    let first = storage::create_upstream(
        &pool,
        &config.app_secret,
        config.secret_key_version,
        &UpsertUpstream {
            name: "first".into(),
            base_url: first_url.into(),
            api_key: "sk-first".into(),
            enabled: Some(true),
            priority: Some(1),
            weight: Some(1),
            timeout_ms: timeout_explicit(first_timeout_ms),
            max_retries: Some(first_max_retries),
            health_check_path: None,
        },
    )
    .await
    .unwrap();
    let second = storage::create_upstream(
        &pool,
        &config.app_secret,
        config.secret_key_version,
        &UpsertUpstream {
            name: "second".into(),
            base_url: second_url.into(),
            api_key: "sk-second".into(),
            enabled: Some(true),
            priority: Some(2),
            weight: Some(1),
            timeout_ms: timeout_explicit(5_000),
            max_retries: Some(1),
            health_check_path: None,
        },
    )
    .await
    .unwrap();
    storage::create_model(
        &pool,
        &UpsertModel {
            public_name: "codex-mini".into(),
            description: None,
            enabled: Some(true),
            visible_to_users: Some(true),
            upstream_mappings: Some(vec![
                UpsertModelMapping {
                    upstream_id: first.id,
                    upstream_model_name: "first-upstream-model".into(),
                    enabled: Some(true),
                    priority: Some(1),
                    weight: Some(1),
                },
                UpsertModelMapping {
                    upstream_id: second.id,
                    upstream_model_name: "second-upstream-model".into(),
                    enabled: Some(true),
                    priority: Some(2),
                    weight: Some(1),
                },
            ]),
        },
    )
    .await
    .unwrap();
    let (_, plaintext) = storage::create_api_key(
        &pool,
        &config.app_secret,
        &user_id,
        &CreateApiKey {
            name: "test".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();
    let state = AppState {
        config: std::sync::Arc::new(config),
        db: pool.clone(),
        http: reqwest::Client::new(),
        finalizations: FinalizationTracker::default(),
        clock: codex_gateway::clock::system_clock(),
    };
    (build_app(state), plaintext, pool)
}

pub async fn seed_weighted_model(pool: &SqlitePool, config: &Config) {
    let light = storage::create_upstream(
        pool,
        &config.app_secret,
        config.secret_key_version,
        &UpsertUpstream {
            name: "light".into(),
            base_url: "http://127.0.0.1:9".into(),
            api_key: "sk-light".into(),
            enabled: Some(true),
            priority: Some(1),
            weight: Some(1),
            timeout_ms: timeout_explicit(5_000),
            max_retries: Some(1),
            health_check_path: None,
        },
    )
    .await
    .unwrap();
    let heavy = storage::create_upstream(
        pool,
        &config.app_secret,
        config.secret_key_version,
        &UpsertUpstream {
            name: "heavy".into(),
            base_url: "http://127.0.0.1:9".into(),
            api_key: "sk-heavy".into(),
            enabled: Some(true),
            priority: Some(2),
            weight: Some(8),
            timeout_ms: timeout_explicit(5_000),
            max_retries: Some(1),
            health_check_path: None,
        },
    )
    .await
    .unwrap();
    storage::create_model(
        pool,
        &UpsertModel {
            public_name: "codex-mini".into(),
            description: None,
            enabled: Some(true),
            visible_to_users: Some(true),
            upstream_mappings: Some(vec![
                UpsertModelMapping {
                    upstream_id: light.id,
                    upstream_model_name: "light-model".into(),
                    enabled: Some(true),
                    priority: Some(1),
                    weight: Some(1),
                },
                UpsertModelMapping {
                    upstream_id: heavy.id,
                    upstream_model_name: "heavy-model".into(),
                    enabled: Some(true),
                    priority: Some(2),
                    weight: Some(1),
                },
            ]),
        },
    )
    .await
    .unwrap();
}

pub fn test_config() -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        database_url: "sqlite://:memory:".into(),
        app_secret: "test-secret".into(),
        secret_key_version: 1,
        public_url: "http://localhost".into(),
        cors_allowed_origins: vec!["http://localhost".into()],
        log_level: "info".into(),
        route_strategy: RouteStrategy::Priority,
        default_request_timeout_ms: default_request_timeout_ms(),
        max_request_body_bytes: 10 * 1024 * 1024,
        health_checks_enabled: false,
        health_check_interval_ms: 30_000,
        request_log_retention_days: 90,
        daily_usage_retention_days: 730,
        retention_run_on_startup: true,
        expose_debug_headers: false,
        admin_email: None,
        admin_password: None,
        bootstrap_admin_key: None,
        runtime_env: Default::default(),
    }
}

pub async fn insert_test_log(
    pool: &SqlitePool,
    request_id: &str,
    user_id: &str,
    api_key_id: &str,
    model_id: Option<&str>,
    upstream_id: Option<&str>,
    outcome: (i64, &str),
) {
    let (status_code, started_at) = outcome;
    storage::insert_request_log(
        pool,
        RequestLogInsert {
            request_id: request_id.into(),
            user_id: user_id.into(),
            api_key_id: api_key_id.into(),
            model_id: model_id.map(str::to_string),
            upstream_id: upstream_id.map(str::to_string),
            method: "POST".into(),
            path: "/responses".into(),
            status_code: Some(status_code),
            error_code: (status_code >= 400).then(|| "upstream_error".into()),
            stream: false,
            usage: UsageSnapshot {
                prompt_tokens: 2,
                completion_tokens: 3,
                total_tokens: 5,
                ..UsageSnapshot::default()
            },
            input_chars: 10,
            output_chars: 20,
            latency_ms: 25,
            started_at: started_at.into(),
            finished_at: started_at.into(),
            client_ip_hash: None,
            user_agent: None,
            client_metadata_sanitized: None,
            route_strategy: None,
            route_decision_json: None,
        },
    )
    .await
    .unwrap();
}

pub async fn insert_test_log_with_latency(
    pool: &SqlitePool,
    request_id: &str,
    user_id: &str,
    api_key_id: &str,
    model_id: Option<&str>,
    upstream_id: Option<&str>,
    outcome: (i64, i64, &str),
) {
    let (status_code, latency_ms, started_at) = outcome;
    storage::insert_request_log(
        pool,
        RequestLogInsert {
            request_id: request_id.into(),
            user_id: user_id.into(),
            api_key_id: api_key_id.into(),
            model_id: model_id.map(str::to_string),
            upstream_id: upstream_id.map(str::to_string),
            method: "POST".into(),
            path: "/responses".into(),
            status_code: Some(status_code),
            error_code: (status_code >= 400).then(|| "upstream_error".into()),
            stream: false,
            usage: UsageSnapshot {
                prompt_tokens: 2,
                completion_tokens: 3,
                total_tokens: 5,
                ..UsageSnapshot::default()
            },
            input_chars: 10,
            output_chars: 20,
            latency_ms,
            started_at: started_at.into(),
            finished_at: started_at.into(),
            client_ip_hash: None,
            user_agent: None,
            client_metadata_sanitized: None,
            route_strategy: None,
            route_decision_json: None,
        },
    )
    .await
    .unwrap();
}

pub async fn seed_user_model(pool: &SqlitePool, upstream_url: Option<&str>) -> String {
    let user_id = storage::ensure_user(
        pool,
        &CreateUser {
            email: "user@example.com".into(),
            password: "password".into(),
            role: "admin".into(),
            display_name: Some("Test User".into()),
        },
    )
    .await
    .unwrap();

    let upstream = storage::create_upstream(
        pool,
        "test-secret",
        1,
        &UpsertUpstream {
            name: "mock".into(),
            base_url: upstream_url.unwrap_or("http://127.0.0.1:9").into(),
            api_key: "sk-upstream-test".into(),
            enabled: Some(true),
            priority: Some(1),
            weight: Some(1),
            timeout_ms: timeout_explicit(5_000),
            max_retries: Some(1),
            health_check_path: None,
        },
    )
    .await
    .unwrap();

    storage::create_model(
        pool,
        &UpsertModel {
            public_name: "codex-mini".into(),
            description: Some("test model".into()),
            enabled: Some(true),
            visible_to_users: Some(true),
            upstream_mappings: Some(vec![UpsertModelMapping {
                upstream_id: upstream.id,
                upstream_model_name: "upstream-codex-mini".into(),
                enabled: Some(true),
                priority: Some(1),
                weight: Some(1),
            }]),
        },
    )
    .await
    .unwrap();

    user_id
}

pub async fn spawn_mock_upstream() -> String {
    MockUpstream::spawn().await.url
}

pub struct MockUpstream {
    pub url: String,
}

impl MockUpstream {
    pub async fn spawn() -> Self {
        let app = Router::new()
            .route("/responses", post(mock_responses))
            .route("/responses/compact", post(mock_compact))
            .route("/v1/models", get(mock_models));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url: format!("http://{addr}"),
        }
    }
}

pub async fn spawn_gateway_server(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), handle)
}

pub async fn spawn_cancellable_sse_upstream() -> (String, tokio::sync::oneshot::Receiver<()>) {
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let dropped_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(dropped_tx)));
    let app = Router::new()
        .route(
            "/responses",
            post({
                let dropped_tx = dropped_tx.clone();
                move || {
                    let dropped_tx = dropped_tx.clone();
                    async move {
                        let on_drop = dropped_tx
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .take();
                        let body = Body::from_stream(CancelAwareSse {
                            sent_first: false,
                            interval: tokio::time::interval(Duration::from_millis(50)),
                            on_drop,
                        });
                        ([(header::CONTENT_TYPE, "text/event-stream")], body)
                    }
                }
            }),
        )
        .route("/v1/models", get(mock_models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), dropped_rx)
}

pub async fn spawn_status_upstream(status: StatusCode) -> String {
    let app = Router::new()
        .route(
            "/responses",
            post(move || async move { (status, Json(json!({"error":{"type":"api_error"}}))) }),
        )
        .route("/v1/models", get(mock_models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

pub async fn spawn_usage_sse_upstream(
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
) -> String {
    let app = Router::new()
        .route(
            "/responses",
            post(move || async move {
                let event = json!({
                    "type": "response.completed",
                    "response": {
                        "id": "resp_stream_usage",
                        "status": "completed",
                        "usage": {
                            "input_tokens": prompt_tokens,
                            "output_tokens": completion_tokens,
                            "total_tokens": total_tokens
                        }
                    }
                });
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    format!("data: {event}\n\ndata: [DONE]\n\n"),
                )
            }),
        )
        .route("/v1/models", get(mock_models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

pub async fn spawn_completed_then_stalling_sse_upstream(
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
) -> String {
    let app = Router::new()
        .route(
            "/responses",
            post(move || async move {
                let event = json!({
                    "type": "response.completed",
                    "response": {
                        "id": "resp_stream_stall",
                        "status": "completed",
                        "usage": {
                            "input_tokens": prompt_tokens,
                            "output_tokens": completion_tokens,
                            "total_tokens": total_tokens
                        }
                    }
                });
                let body = Body::from_stream(async_stream::stream! {
                    yield Ok::<_, Infallible>(bytes::Bytes::from(format!("data: {event}\n\n")));
                    std::future::pending::<()>().await;
                });
                ([(header::CONTENT_TYPE, "text/event-stream")], body)
            }),
        )
        .route("/v1/models", get(mock_models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

pub async fn spawn_counting_upstream() -> (String, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let route_calls = calls.clone();
    let compact_calls = calls.clone();
    let app = Router::new()
        .route(
            "/responses",
            post(move || {
                let calls = route_calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "model_seen": "counted",
                        "usage": {
                            "input_tokens": 1,
                            "output_tokens": 2,
                            "total_tokens": 3
                        }
                    }))
                }
            }),
        )
        .route(
            "/responses/compact",
            post(move || {
                let calls = compact_calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "compact_seen": true,
                        "usage": {
                            "input_tokens": 1,
                            "output_tokens": 2,
                            "total_tokens": 3
                        }
                    }))
                }
            }),
        )
        .route("/v1/models", get(mock_models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), calls)
}

pub async fn spawn_blocking_counting_upstream() -> (
    String,
    Arc<AtomicUsize>,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let route_calls = calls.clone();
    let upstream_entered = Arc::new(tokio::sync::Notify::new());
    let route_entered = upstream_entered.clone();
    let release_upstream = Arc::new(tokio::sync::Notify::new());
    let route_release = release_upstream.clone();
    let app = Router::new()
        .route(
            "/responses",
            post(move || {
                let calls = route_calls.clone();
                let entered = route_entered.clone();
                let release = route_release.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    entered.notify_one();
                    release.notified().await;
                    Json(json!({
                        "model_seen": "counted",
                        "usage": {
                            "input_tokens": 1,
                            "output_tokens": 2,
                            "total_tokens": 3
                        }
                    }))
                }
            }),
        )
        .route("/v1/models", get(mock_models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (
        format!("http://{addr}"),
        calls,
        upstream_entered,
        release_upstream,
    )
}

pub async fn spawn_stalling_upstream() -> String {
    let app = Router::new()
        .route(
            "/responses",
            post(|| async move {
                std::future::pending::<()>().await;
                Json(json!({
                    "model_seen": "delayed-model",
                    "usage": {
                        "input_tokens": 1,
                        "output_tokens": 2,
                        "total_tokens": 3
                    }
                }))
            }),
        )
        .route("/v1/models", get(mock_models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

pub async fn spawn_stalling_health_upstream() -> String {
    let app = Router::new()
        .route("/responses", post(mock_responses))
        .route(
            "/v1/models",
            get(|| async move {
                std::future::pending::<()>().await;
                Json(json!({ "object": "list", "data": [] }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

pub async fn spawn_blocking_health_upstream()
-> (String, Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>) {
    let entered = Arc::new(tokio::sync::Notify::new());
    let route_entered = entered.clone();
    let release = Arc::new(tokio::sync::Notify::new());
    let route_release = release.clone();
    let app = Router::new()
        .route("/responses", post(mock_responses))
        .route(
            "/v1/models",
            get(move || {
                let entered = route_entered.clone();
                let release = route_release.clone();
                async move {
                    entered.notify_one();
                    release.notified().await;
                    Json(json!({ "object": "list", "data": [] }))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), entered, release)
}

pub async fn spawn_body_stall_upstream() -> String {
    let app = Router::new()
        .route(
            "/responses",
            post(|| async move {
                let body = Body::from_stream(async_stream::stream! {
                    yield Ok::<_, std::convert::Infallible>(bytes::Bytes::from_static(b"{\"partial\":"));
                    std::future::pending::<()>().await;
                    yield Ok::<_, std::convert::Infallible>(bytes::Bytes::from_static(b"true}"));
                });
                ([(header::CONTENT_TYPE, "application/json")], body)
            }),
        )
        .route("/v1/models", get(mock_models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

pub async fn spawn_sse_status_upstream(status: StatusCode) -> String {
    let app = Router::new()
        .route(
            "/responses",
            post(move || async move {
                (
                    status,
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    "event: error\ndata: {}\n\n",
                )
            }),
        )
        .route("/v1/models", get(mock_models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

pub async fn mock_responses(
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    Json(json!({
        "model_seen": body["model"],
        "auth_seen": headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "unknown_seen": body["unknown_field"],
        "usage": {
            "input_tokens": 1,
            "output_tokens": 2,
            "total_tokens": 3
        }
    }))
}

pub async fn mock_compact(
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    Json(json!({
        "compact_seen": true,
        "model_seen": body["model"],
        "auth_seen": header_string(&headers, "authorization").unwrap_or_default(),
        "unknown_seen": body["unknown_compact_field"],
        "headers_seen": {
            "openai_beta": header_string(&headers, "openai-beta"),
            "traceparent": header_string(&headers, "traceparent"),
            "tracestate": header_string(&headers, "tracestate"),
            "x_codex_installation_id": header_string(&headers, "x-codex-installation-id"),
            "x_codex_turn_state": header_string(&headers, "x-codex-turn-state"),
            "x_codex_turn_metadata": header_string(&headers, "x-codex-turn-metadata"),
            "x_codex_parent_thread_id": header_string(&headers, "x-codex-parent-thread-id"),
            "x_codex_window_id": header_string(&headers, "x-codex-window-id"),
            "x_openai_memgen_request": header_string(&headers, "x-openai-memgen-request"),
            "x_openai_subagent": header_string(&headers, "x-openai-subagent"),
            "x_responsesapi_include_timing_metrics": header_string(&headers, "x-responsesapi-include-timing-metrics"),
            "x_codex_beta_features": header_string(&headers, "x-codex-beta-features"),
            "x_openai_internal_codex_responses_lite": header_string(&headers, "x-openai-internal-codex-responses-lite"),
            "x_openai_api_key": header_string(&headers, "x-openai-api-key")
        },
        "usage": {
            "input_tokens": 4,
            "output_tokens": 5,
            "total_tokens": 9
        }
    }))
}

pub async fn mock_models() -> impl IntoResponse {
    Json(json!({ "object": "list", "data": [] }))
}

pub async fn to_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

pub async fn assert_status_json(response: axum::response::Response, expected: StatusCode) -> Value {
    assert_eq!(response.status(), expected);
    to_json(response).await
}

pub async fn assert_limit_settlement(
    pool: &SqlitePool,
    expected_events: i64,
    expected_tokens: i64,
) {
    let key = storage::list_api_keys(pool).await.unwrap().remove(0);
    let state = storage::user_limit_state(pool, &key.user_id, Some(&key.id))
        .await
        .unwrap();
    let current = state.current_key.expect("seeded key has limit state");
    assert_eq!(
        current.request_quota.used, expected_events,
        "unexpected admission count"
    );
    assert_eq!(
        current.token_budget.used, expected_tokens,
        "unexpected settled tokens"
    );
    assert_eq!(
        current.concurrency.in_flight, 0,
        "admission remained in flight"
    );
}

pub fn header_string(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

pub struct CancelAwareSse {
    sent_first: bool,
    interval: tokio::time::Interval,
    on_drop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Stream for CancelAwareSse {
    type Item = Result<bytes::Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if !self.sent_first {
            self.sent_first = true;
            return Poll::Ready(Some(Ok(bytes::Bytes::from_static(
                b"data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_disconnect\",\"status\":\"in_progress\"}}\n\n",
            ))));
        }

        match Pin::new(&mut self.interval).poll_tick(cx) {
            Poll::Ready(_) => Poll::Ready(Some(Ok(bytes::Bytes::from_static(b": keepalive\n\n")))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for CancelAwareSse {
    fn drop(&mut self) {
        if let Some(on_drop) = self.on_drop.take() {
            let _ = on_drop.send(());
        }
    }
}

pub fn json_request(
    method: &'static str,
    uri: impl AsRef<str>,
    key: &str,
    body: Value,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

pub fn proxy_request(uri: impl AsRef<str>, key: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri.as_ref())
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "model": "codex-mini",
                "stream": false,
                "input": []
            })
            .to_string(),
        ))
        .unwrap()
}

pub fn limit_set(value: i64) -> storage::LimitPatchValue {
    storage::LimitPatchValue::Set(value)
}

pub fn timeout_default() -> storage::TimeoutPatchValue {
    storage::TimeoutPatchValue::Default
}

pub fn timeout_missing() -> storage::TimeoutPatchValue {
    storage::TimeoutPatchValue::Missing
}

pub fn timeout_explicit(value: i64) -> storage::TimeoutPatchValue {
    storage::TimeoutPatchValue::Explicit(value)
}

pub fn empty_request(method: &'static str, uri: impl AsRef<str>, key: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap()
}
