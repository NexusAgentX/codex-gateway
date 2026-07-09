use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Json,
    http::{Request, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use codex_gateway::{
    AppState, auth, build_app,
    config::{Config, RouteStrategy},
    storage::{
        self, CreateApiKey, CreateUser, RequestLogInsert, UpsertModel, UpsertModelMapping,
        UpsertUpstream,
    },
    usage::UsageSnapshot,
};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt;

#[tokio::test]
async fn health_endpoint_is_public() {
    let (app, _) = test_app(None).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn models_endpoint_returns_visible_gateway_models() {
    let (app, key) = test_app(None).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_json(response).await;
    assert_eq!(body["data"][0]["id"], "codex-mini");
}

#[tokio::test]
async fn proxy_rewrites_model_and_authorization() {
    let upstream = spawn_mock_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/responses")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/json")
                .body(Body::from(
                    json!({
                        "model": "codex-mini",
                        "stream": false,
                        "input": [],
                        "client_metadata": {
                            "session_id": "session-secret",
                            "thread_id": "thread-secret",
                            "x-codex-turn-metadata": "raw-turn-secret"
                        },
                        "unknown_field": { "preserve": true }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_json(response).await;
    assert_eq!(body["model_seen"], "upstream-codex-mini");
    assert_eq!(body["auth_seen"], "Bearer sk-upstream-test");
    assert_eq!(body["unknown_seen"]["preserve"], true);

    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    let metadata = logs[0].client_metadata_sanitized.as_deref().unwrap();
    assert!(metadata.contains("session_id_hash"));
    assert!(metadata.contains("thread_id_hash"));
    assert!(!metadata.contains("session-secret"));
    assert!(!metadata.contains("thread-secret"));
    assert!(!metadata.contains("raw-turn-secret"));
}

#[tokio::test]
async fn non_streaming_proxy_falls_back_and_logs_each_attempt() {
    let failing = spawn_status_upstream(StatusCode::SERVICE_UNAVAILABLE).await;
    let healthy = spawn_mock_upstream().await;
    let (app, key, pool) = app_with_two_upstreams(&failing, &healthy).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/responses")
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
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_json(response).await;
    assert_eq!(body["model_seen"], "second-upstream-model");

    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 2);
    assert!(logs.iter().any(|log| {
        log.status_code == Some(503)
            && log.error_code.as_deref() == Some("upstream_error")
            && log.usage_source == "unknown"
    }));
    assert!(logs.iter().any(|log| {
        log.status_code == Some(200) && log.error_code.is_none() && log.usage_source == "upstream"
    }));

    let usage = storage::list_daily_usage(&pool, None).await.unwrap();
    assert_eq!(usage.iter().map(|row| row.request_count).sum::<i64>(), 2);
}

#[tokio::test]
async fn connect_error_attempt_is_logged_with_unknown_usage() {
    let (app, key, pool) = test_app_with_pool(Some("http://127.0.0.1:9")).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/responses")
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
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].status_code, Some(502));
    assert_eq!(logs[0].error_code.as_deref(), Some("upstream_error"));
    assert_eq!(logs[0].usage_source, "unknown");
}

#[tokio::test]
async fn admin_health_check_updates_upstream_status() {
    let upstream = spawn_mock_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    let upstream_id: (String,) = sqlx::query_as("SELECT id FROM upstreams LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/admin/upstreams/{}/health", upstream_id.0))
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let health: (String,) = sqlx::query_as("SELECT last_health_status FROM upstreams WHERE id = ?")
        .bind(upstream_id.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(health.0, "healthy");
}

#[tokio::test]
async fn admin_settings_returns_sanitized_config_summary() {
    let (app, key) = test_app(None).await;
    let response = app
        .oneshot(empty_request("GET", "/api/admin/settings", &key))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_json(response).await;
    assert_eq!(body["service"], "codex-gateway");
    assert_eq!(body["route_strategy"], "priority");
    assert_eq!(body["database"]["kind"], "sqlite");
    assert!(body["counts"]["users"].as_i64().unwrap() >= 1);
    assert!(body.get("app_secret").is_none());
    assert!(body.get("bootstrap_admin_key").is_none());
}

#[tokio::test]
async fn admin_operator_crud_updates_disables_and_revokes() {
    let upstream = spawn_mock_upstream().await;
    let (app, admin_key, pool) = test_app_with_pool(Some(&upstream)).await;
    let user_id = storage::ensure_user(
        &pool,
        &CreateUser {
            email: "operator-target@example.com".into(),
            password: "old-pass-123".into(),
            role: "user".into(),
            display_name: Some("Old Name".into()),
        },
    )
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/users/{user_id}"),
            &admin_key,
            json!({
                "role": "admin",
                "status": "disabled",
                "display_name": "New Name"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_json(response).await;
    assert_eq!(body["role"], "admin");
    assert_eq!(body["status"], "disabled");
    assert_eq!(body["display_name"], "New Name");

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            format!("/api/admin/users/{user_id}/password"),
            &admin_key,
            json!({ "password": "new-pass-123" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let credentials = storage::find_user_credentials_by_email(&pool, "operator-target@example.com")
        .await
        .unwrap()
        .unwrap();
    assert!(auth::verify_password(
        "new-pass-123",
        &credentials.password_hash
    ));

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/api-keys",
            &admin_key,
            json!({
                "user_id": user_id,
                "name": "created-by-admin",
                "expires_at": "2099-01-01T00:00:00Z"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let created_key = to_json(response).await;
    assert!(
        created_key["plaintext"]
            .as_str()
            .unwrap()
            .starts_with("cgk_live_")
    );
    let api_key_id = created_key["key"]["id"].as_str().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(empty_request("GET", "/api/admin/api-keys", &admin_key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let listed_keys = to_json(response).await;
    assert!(
        listed_keys
            .as_array()
            .unwrap()
            .iter()
            .all(|key| { key.get("plaintext").is_none() && key.get("key_hash").is_none() })
    );

    let response = app
        .clone()
        .oneshot(empty_request(
            "POST",
            format!("/api/admin/api-keys/{api_key_id}/disable"),
            &admin_key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_json(response).await["status"], "disabled");

    let response = app
        .clone()
        .oneshot(empty_request(
            "POST",
            format!("/api/admin/api-keys/{api_key_id}/revoke"),
            &admin_key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let revoked = to_json(response).await;
    assert_eq!(revoked["status"], "revoked");
    assert!(revoked["revoked_at"].is_string());

    let upstream_id: (String,) = sqlx::query_as("SELECT id FROM upstreams LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/upstreams/{}", upstream_id.0),
            &admin_key,
            json!({
                "base_url": upstream,
                "api_key": "sk-rotated",
                "enabled": true,
                "priority": 9,
                "weight": 3,
                "timeout_ms": 7000,
                "max_retries": 2,
                "health_check_path": "/v1/models"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let updated_upstream = to_json(response).await;
    assert_eq!(updated_upstream["priority"], 9);
    assert_eq!(updated_upstream["weight"], 3);
    assert_eq!(updated_upstream["api_key_ciphertext"], Value::Null);

    let response = app
        .clone()
        .oneshot(empty_request(
            "POST",
            format!("/api/admin/upstreams/{}/disable", upstream_id.0),
            &admin_key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_json(response).await["enabled"], 0);

    let model_id: (String,) = sqlx::query_as("SELECT id FROM models LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/models/{}", model_id.0),
            &admin_key,
            json!({
                "description": "operator updated",
                "enabled": true,
                "visible_to_users": false
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let updated_model = to_json(response).await;
    assert_eq!(updated_model["description"], "operator updated");
    assert_eq!(updated_model["visible_to_users"], 0);

    let mapping_id: (String,) = sqlx::query_as("SELECT id FROM upstream_models LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/model-mappings/{}", mapping_id.0),
            &admin_key,
            json!({
                "upstream_model_name": "operator-model",
                "priority": 4,
                "weight": 5
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let updated_mapping = to_json(response).await;
    assert_eq!(updated_mapping["upstream_model_name"], "operator-model");
    assert_eq!(updated_mapping["priority"], 4);

    let response = app
        .oneshot(empty_request(
            "POST",
            format!("/api/admin/model-mappings/{}/disable", mapping_id.0),
            &admin_key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_json(response).await["enabled"], 0);
}

#[tokio::test]
async fn operator_crud_enforces_auth_scope_and_validation() {
    let (app, admin_key, pool) = test_app_with_pool(None).await;
    let user_id = storage::ensure_user(
        &pool,
        &CreateUser {
            email: "plain-user@example.com".into(),
            password: "password".into(),
            role: "user".into(),
            display_name: None,
        },
    )
    .await
    .unwrap();
    let (_, user_key) = storage::create_api_key(
        &pool,
        "test-secret",
        &user_id,
        &CreateApiKey {
            name: "user-key".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();
    let admin_key_id: (String,) = sqlx::query_as(
        "SELECT api_keys.id FROM api_keys JOIN users ON users.id = api_keys.user_id
         WHERE users.role = 'admin' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(empty_request("GET", "/api/admin/users", &user_key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(to_json(response).await["error"]["code"], "forbidden");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/users")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(empty_request(
            "POST",
            format!("/api/api-keys/{}/revoke", admin_key_id.0),
            &user_key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = app
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/users/{user_id}"),
            &admin_key,
            json!({ "role": "owner" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(to_json(response).await["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn admin_upstream_rejects_header_invalid_api_keys() {
    let (app, admin_key, pool) = test_app_with_pool(None).await;

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/upstreams",
            &admin_key,
            json!({
                "name": "bad-create",
                "base_url": "http://127.0.0.1:9",
                "api_key": "sk-bad\r\nx-leak: yes"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(to_json(response).await["error"]["code"], "invalid_request");

    let upstream_id: (String,) = sqlx::query_as("SELECT id FROM upstreams LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let response = app
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/upstreams/{}", upstream_id.0),
            &admin_key,
            json!({
                "api_key": "sk-bad\u{7f}"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(to_json(response).await["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn proxy_fails_safely_when_stored_upstream_key_is_invalid() {
    let upstream = spawn_mock_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    sqlx::query("UPDATE upstreams SET api_key_ciphertext = ?")
        .bind("sk-bad\r\nx-leak: yes")
        .execute(&pool)
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/responses")
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
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_json(response).await;
    assert_eq!(body["error"]["code"], "upstream_unavailable");

    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].status_code, Some(502));
    assert_eq!(logs[0].error_code.as_deref(), Some("upstream_unavailable"));
}

#[tokio::test]
async fn bootstrap_seed_reconciles_admin_and_updates_key_in_place() {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let mut config = test_config();
    config.admin_email = Some("admin@example.com".into());
    config.admin_password = Some("new-password".into());
    config.bootstrap_admin_key = Some("cgk_live_bootstrap1_secret1".into());

    let user_id = storage::ensure_user(
        &pool,
        &CreateUser {
            email: "admin@example.com".into(),
            password: "old-password".into(),
            role: "user".into(),
            display_name: None,
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE users SET status = 'disabled' WHERE id = ?")
        .bind(&user_id)
        .execute(&pool)
        .await
        .unwrap();

    storage::seed_bootstrap_admin(&pool, &config).await.unwrap();
    let key_before: (String,) =
        sqlx::query_as("SELECT id FROM api_keys WHERE name = 'bootstrap-admin'")
            .fetch_one(&pool)
            .await
            .unwrap();
    storage::insert_request_log(
        &pool,
        RequestLogInsert {
            request_id: "bootstrap-log".into(),
            user_id: user_id.clone(),
            api_key_id: key_before.0.clone(),
            model_id: None,
            upstream_id: None,
            method: "POST".into(),
            path: "/responses".into(),
            status_code: Some(200),
            error_code: None,
            stream: false,
            usage: UsageSnapshot::default(),
            input_chars: 0,
            output_chars: 0,
            latency_ms: 1,
            started_at: storage::now_string(),
            finished_at: storage::now_string(),
            client_ip_hash: None,
            user_agent: None,
            client_metadata_sanitized: None,
        },
    )
    .await
    .unwrap();

    config.bootstrap_admin_key = Some("cgk_live_bootstrap2_secret2".into());
    storage::seed_bootstrap_admin(&pool, &config).await.unwrap();

    let key_after: (String, String, String) = sqlx::query_as(
        "SELECT id, key_prefix, status FROM api_keys WHERE name = 'bootstrap-admin'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(key_after.0, key_before.0);
    assert_eq!(key_after.1, "bootstrap2");
    assert_eq!(key_after.2, "active");

    let credentials = storage::find_user_credentials_by_email(&pool, "admin@example.com")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(credentials.role, "admin");
    assert_eq!(credentials.status, "active");
    assert!(auth::verify_password(
        "new-password",
        &credentials.password_hash
    ));

    let log_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM request_logs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(log_count.0, 1);
}

async fn test_app(upstream_url: Option<&str>) -> (Router, String) {
    let (app, key, _) = test_app_with_pool(upstream_url).await;
    (app, key)
}

async fn test_app_with_pool(upstream_url: Option<&str>) -> (Router, String, SqlitePool) {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let config = test_config();

    let user_id = seed_user_model(&pool, upstream_url).await;
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
    };
    (build_app(state), plaintext, pool)
}

async fn app_with_two_upstreams(first_url: &str, second_url: &str) -> (Router, String, SqlitePool) {
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
        &UpsertUpstream {
            name: "first".into(),
            base_url: first_url.into(),
            api_key: "sk-first".into(),
            enabled: Some(true),
            priority: Some(1),
            weight: Some(1),
            timeout_ms: Some(5_000),
            max_retries: Some(1),
            health_check_path: None,
        },
    )
    .await
    .unwrap();
    let second = storage::create_upstream(
        &pool,
        &UpsertUpstream {
            name: "second".into(),
            base_url: second_url.into(),
            api_key: "sk-second".into(),
            enabled: Some(true),
            priority: Some(2),
            weight: Some(1),
            timeout_ms: Some(5_000),
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
    };
    (build_app(state), plaintext, pool)
}

fn test_config() -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        database_url: "sqlite://:memory:".into(),
        app_secret: "test-secret".into(),
        public_url: "http://localhost".into(),
        log_level: "info".into(),
        route_strategy: RouteStrategy::Priority,
        admin_email: None,
        admin_password: None,
        bootstrap_admin_key: None,
    }
}

async fn seed_user_model(pool: &SqlitePool, upstream_url: Option<&str>) -> String {
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
        &UpsertUpstream {
            name: "mock".into(),
            base_url: upstream_url.unwrap_or("http://127.0.0.1:9").into(),
            api_key: "sk-upstream-test".into(),
            enabled: Some(true),
            priority: Some(1),
            weight: Some(1),
            timeout_ms: Some(5_000),
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

async fn spawn_mock_upstream() -> String {
    let app = Router::new()
        .route("/responses", post(mock_responses))
        .route("/v1/models", get(mock_models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_status_upstream(status: StatusCode) -> String {
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

async fn mock_responses(
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

async fn mock_models() -> impl IntoResponse {
    Json(json!({ "object": "list", "data": [] }))
}

async fn to_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn json_request(
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

fn empty_request(method: &'static str, uri: impl AsRef<str>, key: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap()
}
