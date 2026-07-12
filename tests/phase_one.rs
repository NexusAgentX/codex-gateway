use std::{
    convert::Infallible,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Json,
    http::{Request, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use codex_gateway::{
    AppState, JSON_BODY_LIMIT_BYTES, auth, build_app,
    config::{Config, RouteStrategy},
    routing,
    storage::{
        self, CreateApiKey, CreateUser, RequestLogInsert, UpsertModel, UpsertModelMapping,
        UpsertUpstream,
    },
    usage::UsageSnapshot,
};
use futures_util::{Stream, StreamExt};
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
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
                .header("x-request-id", "client-req-1")
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
    let response_request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap()
        .to_string();
    assert_eq!(response_request_id, "client-req-1");
    let body = to_json(response).await;
    assert_eq!(body["model_seen"], "upstream-codex-mini");
    assert_eq!(body["auth_seen"], "Bearer sk-upstream-test");
    assert_eq!(body["unknown_seen"]["preserve"], true);

    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].request_id, response_request_id);
    let metadata = logs[0].client_metadata_sanitized.as_deref().unwrap();
    assert!(metadata.contains("session_id_hash"));
    assert!(metadata.contains("thread_id_hash"));
    assert!(!metadata.contains("session-secret"));
    assert!(!metadata.contains("thread-secret"));
    assert!(!metadata.contains("raw-turn-secret"));
    assert_eq!(logs[0].route_strategy.as_deref(), Some("priority"));
    let route_decision = logs[0].route_decision_json.as_deref().unwrap();
    assert!(route_decision.contains("upstream_id"));
    assert!(route_decision.contains("upstream_model_id"));
    assert!(!route_decision.contains("sk-upstream-test"));
    assert!(!route_decision.contains(&upstream));
}

#[tokio::test]
async fn compact_routes_proxy_json_payload_and_tracing_headers() {
    let upstream = spawn_mock_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;

    for path in ["/responses/compact", "/v1/responses/compact"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header(header::AUTHORIZATION, format!("Bearer {key}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("OpenAI-Beta", "responses_websockets=2026-02-06")
                    .header(
                        "traceparent",
                        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00",
                    )
                    .header("tracestate", "codex=tui")
                    .header("x-codex-installation-id", "install-123")
                    .header("x-codex-turn-state", "turn-state-123")
                    .header("x-codex-turn-metadata", "turn-metadata-123")
                    .header("x-codex-parent-thread-id", "thread-parent-123")
                    .header("x-codex-window-id", "window-123")
                    .header("x-openai-memgen-request", "memgen-123")
                    .header("x-openai-subagent", "subagent-123")
                    .header("x-responsesapi-include-timing-metrics", "true")
                    .header("x-codex-beta-features", "compact")
                    .header("x-openai-internal-codex-responses-lite", "1")
                    .header("x-openai-api-key", "must-not-forward")
                    .body(Body::from(
                        json!({
                            "model": "codex-mini",
                            "input": [
                                {"type": "message", "role": "user", "content": "compact-secret"}
                            ],
                            "tools": [
                                {"type": "custom", "name": "tool", "format": {"type": "grammar"}}
                            ],
                            "reasoning": {"effort": "high"},
                            "unknown_compact_field": {"preserve": true}
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_json(response).await;
        assert_eq!(body["compact_seen"], true);
        assert_eq!(body["model_seen"], "upstream-codex-mini");
        assert_eq!(body["auth_seen"], "Bearer sk-upstream-test");
        assert_eq!(body["unknown_seen"]["preserve"], true);
        assert_eq!(
            body["headers_seen"]["openai_beta"],
            "responses_websockets=2026-02-06"
        );
        assert_eq!(
            body["headers_seen"]["traceparent"],
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00"
        );
        assert_eq!(body["headers_seen"]["tracestate"], "codex=tui");
        assert_eq!(
            body["headers_seen"]["x_codex_installation_id"],
            "install-123"
        );
        assert_eq!(body["headers_seen"]["x_codex_turn_state"], "turn-state-123");
        assert_eq!(
            body["headers_seen"]["x_codex_turn_metadata"],
            "turn-metadata-123"
        );
        assert_eq!(
            body["headers_seen"]["x_codex_parent_thread_id"],
            "thread-parent-123"
        );
        assert_eq!(body["headers_seen"]["x_codex_window_id"], "window-123");
        assert_eq!(
            body["headers_seen"]["x_openai_memgen_request"],
            "memgen-123"
        );
        assert_eq!(body["headers_seen"]["x_openai_subagent"], "subagent-123");
        assert_eq!(
            body["headers_seen"]["x_responsesapi_include_timing_metrics"],
            "true"
        );
        assert_eq!(body["headers_seen"]["x_codex_beta_features"], "compact");
        assert_eq!(
            body["headers_seen"]["x_openai_internal_codex_responses_lite"],
            "1"
        );
        assert_eq!(body["headers_seen"]["x_openai_api_key"], Value::Null);
    }

    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 2);
    assert!(logs.iter().any(|log| {
        log.path == "/responses/compact"
            && log.status_code == Some(200)
            && log.stream == 0
            && log.usage_source == "upstream"
    }));
    assert!(logs.iter().any(|log| {
        log.path == "/v1/responses/compact"
            && log.status_code == Some(200)
            && log.stream == 0
            && log.usage_source == "upstream"
    }));
    let db_text = database_text_dump(&pool).await;
    assert!(!db_text.contains("compact-secret"));
    assert!(!db_text.contains("must-not-forward"));
}

#[tokio::test]
async fn request_and_token_quotas_enforce_and_reset_by_window() {
    let upstream = spawn_mock_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;

    storage::upsert_limit_policy(
        &pool,
        "system",
        "",
        &storage::LimitPolicyPatch {
            request_quota: limit_set(1),
            request_window_seconds: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .clone()
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(to_json(response).await["error"]["code"], "quota_exceeded");
    assert_eq!(
        storage::list_request_logs(&pool, None).await.unwrap().len(),
        1
    );

    tokio::time::sleep(Duration::from_millis(1100)).await;
    let response = app
        .clone()
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    storage::upsert_limit_policy(
        &pool,
        "system",
        "",
        &storage::LimitPolicyPatch {
            request_quota: limit_set(100),
            request_window_seconds: Some(60),
            token_quota: limit_set(3),
            token_window_seconds: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(to_json(response).await["error"]["code"], "quota_exceeded");

    tokio::time::sleep(Duration::from_millis(1100)).await;
    let response = app
        .clone()
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn limit_policy_patch_preserves_omitted_fields_and_null_clears_limits() {
    let (app, admin_key, pool) = test_app_with_pool(None).await;
    let user_id: String = sqlx::query_scalar("SELECT user_id FROM api_keys LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let key_id: String = sqlx::query_scalar("SELECT id FROM api_keys LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/limits/system",
            &admin_key,
            json!({
                "request_quota": 10,
                "request_window_seconds": 60,
                "token_quota": 20,
                "rate_limit_requests": 30,
                "concurrency_limit": 2
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/limits/system",
            &admin_key,
            json!({ "request_window_seconds": 120 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let system = to_json(response).await;
    assert_eq!(system["request_quota"], 10);
    assert_eq!(system["token_quota"], 20);
    assert_eq!(system["rate_limit_requests"], 30);
    assert_eq!(system["concurrency_limit"], 2);
    assert_eq!(system["request_window_seconds"], 120);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/limits/system",
            &admin_key,
            json!({ "request_quota": null }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let system = to_json(response).await;
    assert_eq!(system["request_quota"], Value::Null);
    assert_eq!(system["request_quota_mode"], "unlimited");
    assert_eq!(system["token_quota"], 20);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/users/{user_id}/limits"),
            &admin_key,
            json!({
                "request_quota": 5,
                "token_quota": 6,
                "rate_limit_requests": 7,
                "concurrency_limit": 1
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/users/{user_id}/limits"),
            &admin_key,
            json!({ "token_window_seconds": 90 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let user_limits = to_json(response).await;
    assert_eq!(user_limits["user"]["policy"]["request_quota"], 5);
    assert_eq!(user_limits["user"]["policy"]["token_quota"], 6);
    assert_eq!(user_limits["user"]["policy"]["rate_limit_requests"], 7);
    assert_eq!(user_limits["user"]["policy"]["concurrency_limit"], 1);
    assert_eq!(user_limits["user"]["policy"]["token_window_seconds"], 90);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/users/{user_id}/limits"),
            &admin_key,
            json!({ "request_quota": null }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let user_limits = to_json(response).await;
    assert_eq!(user_limits["user"]["policy"]["request_quota"], Value::Null);
    assert_eq!(
        user_limits["user"]["policy"]["request_quota_mode"],
        "unlimited"
    );
    assert_eq!(user_limits["user"]["policy"]["token_quota"], 6);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/api-keys/{key_id}/limits"),
            &admin_key,
            json!({
                "request_quota": 8,
                "token_quota": 9,
                "rate_limit_requests": 10,
                "concurrency_limit": 3
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/api-keys/{key_id}/limits"),
            &admin_key,
            json!({ "rate_limit_window_seconds": 45 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let key_limits = to_json(response).await;
    assert_eq!(key_limits["policy"]["request_quota"], 8);
    assert_eq!(key_limits["policy"]["token_quota"], 9);
    assert_eq!(key_limits["policy"]["rate_limit_requests"], 10);
    assert_eq!(key_limits["policy"]["concurrency_limit"], 3);
    assert_eq!(key_limits["policy"]["rate_limit_window_seconds"], 45);

    let response = app
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/api-keys/{key_id}/limits"),
            &admin_key,
            json!({ "request_quota": null }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let key_limits = to_json(response).await;
    assert_eq!(key_limits["policy"]["request_quota"], Value::Null);
    assert_eq!(key_limits["policy"]["request_quota_mode"], "unlimited");
    assert_eq!(key_limits["policy"]["token_quota"], 9);
}

#[tokio::test]
async fn user_and_key_limit_overrides_can_reset_to_inherit() {
    let (upstream, upstream_calls) = spawn_counting_upstream(Duration::ZERO).await;
    let (app, admin_key, pool) = test_app_with_pool(Some(&upstream)).await;
    let user_id: String = sqlx::query_scalar("SELECT user_id FROM api_keys LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let key_id: String = sqlx::query_scalar("SELECT id FROM api_keys LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/limits/system",
            &admin_key,
            json!({ "request_quota": 1, "request_window_seconds": 60 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    for (path, first_body, second_body) in [
        (
            format!("/api/admin/users/{user_id}/limits"),
            json!({ "request_quota": 0 }),
            json!({ "request_quota": null }),
        ),
        (
            format!("/api/admin/api-keys/{key_id}/limits"),
            json!({ "request_quota": 0 }),
            json!({ "request_quota": null }),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(json_request("PATCH", &path, &admin_key, first_body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = app
            .clone()
            .oneshot(json_request("PATCH", &path, &admin_key, second_body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/users/{user_id}/limits"),
            &admin_key,
            json!({ "request_quota": { "mode": "inherit" } }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let user_limits = to_json(response).await;
    assert_eq!(
        user_limits["user"]["policy"]["request_quota_mode"],
        "inherit"
    );
    assert_eq!(user_limits["user"]["request_quota"]["limit"], 1);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/api-keys/{key_id}/limits"),
            &admin_key,
            json!({ "request_quota": { "mode": "inherit" } }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let key_limits = to_json(response).await;
    assert_eq!(key_limits["policy"]["request_quota_mode"], "inherit");
    assert_eq!(key_limits["request_quota"]["limit"], 1);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/limits/system",
            &admin_key,
            json!({ "request_quota": 2, "request_window_seconds": 2 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let limits = app
        .clone()
        .oneshot(empty_request("GET", "/api/limits", &admin_key))
        .await
        .unwrap();
    assert_eq!(limits.status(), StatusCode::OK);
    let limits = to_json(limits).await;
    assert_eq!(limits["user"]["policy"]["request_quota_mode"], "inherit");
    assert_eq!(limits["user"]["request_quota"]["limit"], 2);
    assert_eq!(limits["user"]["policy"]["request_window_seconds"], 2);
    assert_eq!(
        limits["user"]["effective_policy"]["request_window_seconds"],
        2
    );
    assert_eq!(limits["user"]["request_quota"]["window_seconds"], 2);
    assert_eq!(
        limits["current_key"]["policy"]["request_quota_mode"],
        "inherit"
    );
    assert_eq!(limits["current_key"]["request_quota"]["limit"], 2);
    assert_eq!(
        limits["current_key"]["effective_policy"]["request_window_seconds"],
        2
    );
    assert_eq!(limits["current_key"]["request_quota"]["window_seconds"], 2);

    assert_eq!(
        app.clone()
            .oneshot(proxy_request("/responses", &admin_key))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.clone()
            .oneshot(proxy_request("/responses", &admin_key))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let rejected = app
        .oneshot(proxy_request("/responses", &admin_key))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert_eq!(upstream_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn inherited_limit_windows_match_system_without_policy_rows() {
    let (upstream, _upstream_calls) = spawn_counting_upstream(Duration::ZERO).await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    storage::upsert_limit_policy(
        &pool,
        "system",
        "",
        &storage::LimitPolicyPatch {
            request_quota: limit_set(1),
            request_window_seconds: Some(1),
            token_quota: limit_set(100),
            token_window_seconds: Some(2),
            rate_limit_requests: limit_set(10),
            rate_limit_window_seconds: Some(3),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let key_policy_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM limit_policies WHERE scope = 'api_key'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(key_policy_rows, 0);

    let user_limits = app
        .clone()
        .oneshot(empty_request("GET", "/api/limits", &key))
        .await
        .unwrap();
    assert_eq!(user_limits.status(), StatusCode::OK);
    let user_limits = to_json(user_limits).await;
    for subject in [&user_limits["user"], &user_limits["current_key"]] {
        assert_eq!(subject["policy"]["request_quota_mode"], "inherit");
        assert_eq!(subject["policy"]["request_window_seconds"], 1);
        assert_eq!(subject["effective_policy"]["request_window_seconds"], 1);
        assert_eq!(subject["request_quota"]["window_seconds"], 1);
        assert_eq!(subject["policy"]["token_window_seconds"], 2);
        assert_eq!(subject["effective_policy"]["token_window_seconds"], 2);
        assert_eq!(subject["token_budget"]["window_seconds"], 2);
        assert_eq!(subject["policy"]["rate_limit_window_seconds"], 3);
        assert_eq!(subject["effective_policy"]["rate_limit_window_seconds"], 3);
        assert_eq!(subject["rate_limit"]["window_seconds"], 3);
    }

    let admin_key = key.clone();
    let admin_limits = app
        .clone()
        .oneshot(empty_request("GET", "/api/admin/limits", &admin_key))
        .await
        .unwrap();
    assert_eq!(admin_limits.status(), StatusCode::OK);
    let admin_limits = to_json(admin_limits).await;
    assert_eq!(
        admin_limits["users"][0]["effective_policy"]["request_window_seconds"],
        1
    );
    assert_eq!(
        admin_limits["api_keys"][0]["effective_policy"]["request_window_seconds"],
        1
    );

    let key_policy_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM limit_policies WHERE scope = 'api_key'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(key_policy_rows, 0);

    assert_eq!(
        app.clone()
            .oneshot(proxy_request("/responses", &key))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let rejected = app
        .clone()
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);

    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        app.oneshot(proxy_request("/responses", &key))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn negative_limit_windows_are_rejected_for_system_user_and_key_policies() {
    let (app, admin_key, pool) = test_app_with_pool(None).await;
    let user_id: String = sqlx::query_scalar("SELECT user_id FROM api_keys LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let key_id: String = sqlx::query_scalar("SELECT id FROM api_keys LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    for (path, body) in [
        (
            "/api/admin/limits/system".to_string(),
            json!({ "request_window_seconds": -1 }),
        ),
        (
            format!("/api/admin/users/{user_id}/limits"),
            json!({ "token_window_seconds": -1 }),
        ),
        (
            format!("/api/admin/api-keys/{key_id}/limits"),
            json!({ "rate_limit_window_seconds": -1 }),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(json_request("PATCH", path, &admin_key, body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(to_json(response).await["error"]["code"], "invalid_request");
    }
}

#[tokio::test]
async fn api_key_default_state_matches_per_key_enforcement_for_multiple_keys() {
    let (upstream, upstream_calls) = spawn_counting_upstream(Duration::ZERO).await;
    let (app, key_one, pool) = test_app_with_pool(Some(&upstream)).await;
    let config = test_config();
    let user_id: String = sqlx::query_scalar("SELECT user_id FROM api_keys LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (_, key_two) = storage::create_api_key(
        &pool,
        &config.app_secret,
        &user_id,
        &CreateApiKey {
            name: "second".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();

    storage::upsert_limit_policy(
        &pool,
        "system",
        "",
        &storage::LimitPolicyPatch {
            request_quota: limit_set(1),
            request_window_seconds: Some(60),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    storage::upsert_limit_policy(
        &pool,
        "user",
        &user_id,
        &storage::LimitPolicyPatch {
            request_quota: storage::LimitPatchValue::Clear,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let key_policy_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM limit_policies WHERE scope = 'api_key'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(key_policy_rows, 0);

    let limits = app
        .clone()
        .oneshot(empty_request("GET", "/api/limits", &key_one))
        .await
        .unwrap();
    assert_eq!(limits.status(), StatusCode::OK);
    let limits = to_json(limits).await;
    assert_eq!(limits["user"]["request_quota"]["limit"], Value::Null);
    assert_eq!(limits["api_keys"].as_array().unwrap().len(), 2);
    assert!(
        limits["api_keys"]
            .as_array()
            .unwrap()
            .iter()
            .all(|state| state["request_quota"]["limit"] == 1)
    );

    assert_eq!(
        app.clone()
            .oneshot(proxy_request("/responses", &key_one))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let rejected = app
        .clone()
        .oneshot(proxy_request("/responses", &key_one))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        to_json(rejected).await["error"]["details"]["scope"],
        "api_key"
    );
    assert_eq!(
        app.oneshot(proxy_request("/responses", &key_two))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(upstream_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn user_and_key_overrides_can_make_system_default_unlimited() {
    let (upstream, upstream_calls) = spawn_counting_upstream(Duration::ZERO).await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    let user_id: String = sqlx::query_scalar("SELECT user_id FROM api_keys LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let key_id: String = sqlx::query_scalar("SELECT id FROM api_keys LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    storage::upsert_limit_policy(
        &pool,
        "system",
        "",
        &storage::LimitPolicyPatch {
            request_quota: limit_set(1),
            request_window_seconds: Some(60),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    storage::upsert_limit_policy(
        &pool,
        "user",
        &user_id,
        &storage::LimitPolicyPatch {
            request_quota: storage::LimitPatchValue::Clear,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    storage::upsert_limit_policy(
        &pool,
        "api_key",
        &key_id,
        &storage::LimitPolicyPatch {
            request_quota: storage::LimitPatchValue::Clear,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let limits = app
        .clone()
        .oneshot(empty_request("GET", "/api/limits", &key))
        .await
        .unwrap();
    assert_eq!(limits.status(), StatusCode::OK);
    let limits = to_json(limits).await;
    assert_eq!(limits["user"]["request_quota"]["limit"], Value::Null);
    assert_eq!(limits["current_key"]["request_quota"]["limit"], Value::Null);

    assert_eq!(
        app.clone()
            .oneshot(proxy_request("/responses", &key))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.oneshot(proxy_request("/responses", &key))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(upstream_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn rate_limit_rejects_without_upstream_or_usage_charge() {
    let (upstream, upstream_calls) = spawn_counting_upstream(Duration::ZERO).await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    storage::upsert_limit_policy(
        &pool,
        "system",
        "",
        &storage::LimitPolicyPatch {
            rate_limit_requests: limit_set(1),
            rate_limit_window_seconds: Some(60),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .oneshot(proxy_request("/v1/responses", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(to_json(response).await["error"]["code"], "rate_limited");
    assert_eq!(upstream_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        storage::list_request_logs(&pool, None).await.unwrap().len(),
        1
    );
    let usage = storage::list_daily_usage(&pool, None).await.unwrap();
    assert_eq!(usage.iter().map(|row| row.request_count).sum::<i64>(), 1);
}

#[tokio::test]
async fn concurrency_limit_rejects_without_upstream_call() {
    let (upstream, upstream_calls, upstream_entered, release_upstream) =
        spawn_blocking_counting_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    storage::upsert_limit_policy(
        &pool,
        "system",
        "",
        &storage::LimitPolicyPatch {
            concurrency_limit: limit_set(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let first = tokio::spawn({
        let app = app.clone();
        let key = key.clone();
        async move {
            app.oneshot(proxy_request("/responses", &key))
                .await
                .unwrap()
        }
    });
    upstream_entered.notified().await;
    let second = app
        .oneshot(proxy_request("/responses/compact", &key))
        .await
        .unwrap();
    release_upstream.notify_one();

    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        to_json(second).await["error"]["code"],
        "concurrency_limited"
    );
    assert_eq!(upstream_calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.await.unwrap().status(), StatusCode::OK);
    assert_eq!(
        storage::list_request_logs(&pool, None).await.unwrap().len(),
        1
    );
    let usage = storage::list_daily_usage(&pool, None).await.unwrap();
    assert_eq!(usage.iter().map(|row| row.request_count).sum::<i64>(), 1);
    assert_eq!(usage.iter().map(|row| row.stream_count).sum::<i64>(), 0);
}

#[tokio::test]
async fn quota_limited_and_disabled_principals_cannot_bypass_routes_or_panel_tokens() {
    let (upstream, upstream_calls) = spawn_counting_upstream(Duration::ZERO).await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    let user_id: String = sqlx::query_scalar("SELECT user_id FROM api_keys LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let key_id: String = sqlx::query_scalar("SELECT id FROM api_keys LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    storage::upsert_limit_policy(
        &pool,
        "user",
        &user_id,
        &storage::LimitPolicyPatch {
            request_quota: limit_set(0),
            request_window_seconds: Some(60),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    for path in ["/responses", "/v1/responses", "/responses/compact"] {
        let response = app
            .clone()
            .oneshot(proxy_request(path, &key))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(to_json(response).await["error"]["code"], "quota_exceeded");
    }
    assert_eq!(upstream_calls.load(Ordering::SeqCst), 0);
    assert!(
        storage::list_request_logs(&pool, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        storage::list_daily_usage(&pool, None)
            .await
            .unwrap()
            .is_empty()
    );

    let login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/login",
            "",
            json!({ "email": "user@example.com", "password": "password" }),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let panel_token = to_json(login).await["token"].as_str().unwrap().to_string();
    let response = app
        .clone()
        .oneshot(proxy_request("/responses", &panel_token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    storage::set_api_key_status(&pool, &key_id, "disabled")
        .await
        .unwrap();
    let response = app
        .clone()
        .oneshot(empty_request("GET", "/v1/models", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(to_json(response).await["error"]["code"], "disabled_api_key");

    storage::update_user(
        &pool,
        &user_id,
        &codex_gateway::storage::UpdateUser {
            role: None,
            status: Some("disabled".into()),
            display_name: None,
        },
    )
    .await
    .unwrap();
    let response = app
        .oneshot(empty_request("GET", "/api/limits", &panel_token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(to_json(response).await["error"]["code"], "disabled_user");
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
                .header("x-request-id", "retry-correlation")
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
    let response_request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap()
        .to_string();
    assert_eq!(response_request_id, "retry-correlation");
    let body = to_json(response).await;
    assert_eq!(body["model_seen"], "second-upstream-model");

    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 2);
    let mut log_request_ids = logs
        .iter()
        .map(|log| log.request_id.as_str())
        .collect::<Vec<_>>();
    log_request_ids.sort_unstable();
    assert_eq!(
        log_request_ids,
        vec!["retry-correlation", "retry-correlation-2"]
    );
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
async fn multi_candidate_first_attempt_success_logs_response_request_id_without_suffix() {
    let healthy = spawn_mock_upstream().await;
    let unused = spawn_mock_upstream().await;
    let (app, key, pool) = app_with_two_upstreams(&healthy, &unused).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/responses")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-request-id", "first-success-correlation")
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
    let response_request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap()
        .to_string();
    assert_eq!(response_request_id, "first-success-correlation");

    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].request_id, response_request_id);
    assert!(!logs[0].request_id.ends_with("-1"));
}

#[tokio::test]
async fn disabled_and_down_upstreams_are_skipped_by_routing() {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let config = test_config();
    let disabled = storage::create_upstream(
        &pool,
        &config.app_secret,
        config.secret_key_version,
        &UpsertUpstream {
            name: "disabled".into(),
            base_url: "http://127.0.0.1:9".into(),
            api_key: "sk-disabled".into(),
            enabled: Some(false),
            priority: Some(1),
            weight: Some(1),
            timeout_ms: timeout_default(),
            max_retries: None,
            health_check_path: None,
        },
    )
    .await
    .unwrap();
    let down = storage::create_upstream(
        &pool,
        &config.app_secret,
        config.secret_key_version,
        &UpsertUpstream {
            name: "down".into(),
            base_url: "http://127.0.0.1:9".into(),
            api_key: "sk-down".into(),
            enabled: Some(true),
            priority: Some(2),
            weight: Some(1),
            timeout_ms: timeout_default(),
            max_retries: None,
            health_check_path: None,
        },
    )
    .await
    .unwrap();
    let healthy = storage::create_upstream(
        &pool,
        &config.app_secret,
        config.secret_key_version,
        &UpsertUpstream {
            name: "healthy".into(),
            base_url: "http://127.0.0.1:9".into(),
            api_key: "sk-healthy".into(),
            enabled: Some(true),
            priority: Some(3),
            weight: Some(1),
            timeout_ms: timeout_default(),
            max_retries: None,
            health_check_path: None,
        },
    )
    .await
    .unwrap();
    storage::record_upstream_health(&pool, &down.id, "down", Some("upstream_timeout"))
        .await
        .unwrap();
    let down_row = storage::get_upstream(&pool, &down.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(down_row.last_health_status, "down");
    assert!(down_row.health_status_changed_at.is_some());
    assert!(down_row.last_down_at.is_some());
    assert!(down_row.recent_error_samples.contains("upstream_timeout"));
    storage::create_model(
        &pool,
        &UpsertModel {
            public_name: "codex-mini".into(),
            description: None,
            enabled: Some(true),
            visible_to_users: Some(true),
            upstream_mappings: Some(vec![
                UpsertModelMapping {
                    upstream_id: disabled.id,
                    upstream_model_name: "disabled-model".into(),
                    enabled: Some(true),
                    priority: Some(1),
                    weight: Some(1),
                },
                UpsertModelMapping {
                    upstream_id: down.id,
                    upstream_model_name: "down-model".into(),
                    enabled: Some(true),
                    priority: Some(2),
                    weight: Some(1),
                },
                UpsertModelMapping {
                    upstream_id: healthy.id.clone(),
                    upstream_model_name: "healthy-model".into(),
                    enabled: Some(true),
                    priority: Some(3),
                    weight: Some(1),
                },
            ]),
        },
    )
    .await
    .unwrap();

    let candidates = routing::route_candidates(
        &pool,
        &config,
        "codex-mini",
        config.default_request_timeout_ms,
    )
    .await
    .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].upstream_id, healthy.id);
}

#[tokio::test]
async fn health_transition_timestamps_refresh_on_repeated_transitions() {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let config = test_config();
    let upstream = storage::create_upstream(
        &pool,
        &config.app_secret,
        config.secret_key_version,
        &UpsertUpstream {
            name: "flaky".into(),
            base_url: "http://127.0.0.1:9".into(),
            api_key: "sk-flaky".into(),
            enabled: Some(true),
            priority: Some(1),
            weight: Some(1),
            timeout_ms: timeout_default(),
            max_retries: None,
            health_check_path: None,
        },
    )
    .await
    .unwrap();

    storage::record_upstream_health(&pool, &upstream.id, "down", Some("first_down"))
        .await
        .unwrap();
    let first_down = storage::get_upstream(&pool, &upstream.id)
        .await
        .unwrap()
        .unwrap();
    let first_changed_at = first_down.health_status_changed_at.clone().unwrap();
    let first_down_at = first_down.last_down_at.clone().unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    storage::record_upstream_health(&pool, &upstream.id, "healthy", None)
        .await
        .unwrap();
    let healthy = storage::get_upstream(&pool, &upstream.id)
        .await
        .unwrap()
        .unwrap();
    let healthy_changed_at = healthy.health_status_changed_at.clone().unwrap();
    assert_ne!(healthy_changed_at, first_changed_at);
    assert_eq!(
        healthy.last_down_at.as_deref(),
        Some(first_down_at.as_str())
    );

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    storage::record_upstream_health(&pool, &upstream.id, "down", Some("second_down"))
        .await
        .unwrap();
    let second_down = storage::get_upstream(&pool, &upstream.id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        second_down.health_status_changed_at.as_deref(),
        Some(healthy_changed_at.as_str())
    );
    assert_ne!(
        second_down.last_down_at.as_deref(),
        Some(first_down_at.as_str())
    );
    assert!(second_down.recent_error_samples.contains("second_down"));

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    storage::record_upstream_health(&pool, &upstream.id, "healthy", None)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    storage::record_upstream_health(&pool, &upstream.id, "degraded", Some("first_degraded"))
        .await
        .unwrap();
    let first_degraded = storage::get_upstream(&pool, &upstream.id)
        .await
        .unwrap()
        .unwrap();
    let first_degraded_at = first_degraded.last_degraded_at.clone().unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    storage::record_upstream_health(&pool, &upstream.id, "healthy", None)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    storage::record_upstream_health(&pool, &upstream.id, "degraded", Some("second_degraded"))
        .await
        .unwrap();
    let second_degraded = storage::get_upstream(&pool, &upstream.id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        second_degraded.last_degraded_at.as_deref(),
        Some(first_degraded_at.as_str())
    );
    assert!(
        second_degraded
            .recent_error_samples
            .contains("second_degraded")
    );
}

#[tokio::test]
async fn weighted_and_sticky_routing_are_deterministic_and_weighted() {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let config = test_config();
    seed_weighted_model(&pool, &config).await;
    let candidates = routing::route_candidates(
        &pool,
        &config,
        "codex-mini",
        config.default_request_timeout_ms,
    )
    .await
    .unwrap();

    let sticky_a = routing::order_candidates(&candidates, RouteStrategy::StickyByKey, "session-a");
    let sticky_b = routing::order_candidates(&candidates, RouteStrategy::StickyByKey, "session-a");
    assert_eq!(sticky_a[0].upstream_id, sticky_b[0].upstream_id);

    let mut heavy_first = 0;
    let mut light_first = 0;
    for index in 0..100 {
        let ordered = routing::order_candidates(
            &candidates,
            RouteStrategy::Weighted,
            &format!("request-{index}"),
        );
        if ordered[0].upstream_name == "heavy" {
            heavy_first += 1;
        } else if ordered[0].upstream_name == "light" {
            light_first += 1;
        }
    }
    assert!(
        heavy_first > light_first * 3,
        "heavy={heavy_first} light={light_first}"
    );
}

#[tokio::test]
async fn connect_error_retries_next_eligible_upstream() {
    let healthy = spawn_mock_upstream().await;
    let (app, key, pool) = app_with_two_upstreams("http://127.0.0.1:9", &healthy).await;

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
    assert_eq!(
        to_json(response).await["model_seen"],
        "second-upstream-model"
    );
    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 2);
    assert!(logs.iter().any(|log| log.status_code == Some(502)));
    assert!(logs.iter().any(|log| log.status_code == Some(200)));
}

#[tokio::test]
async fn timeout_error_retries_next_eligible_upstream() {
    let slow = spawn_delayed_upstream(std::time::Duration::from_millis(150)).await;
    let healthy = spawn_mock_upstream().await;
    let (app, key, pool) = app_with_two_upstreams_and_retries_timeout(&slow, &healthy, 1, 20).await;

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
    assert_eq!(
        to_json(response).await["model_seen"],
        "second-upstream-model"
    );
    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 2);
    assert!(logs.iter().any(|log| {
        log.status_code == Some(504) && log.error_code.as_deref() == Some("upstream_timeout")
    }));
    assert!(logs.iter().any(|log| log.status_code == Some(200)));

    let first = sqlx::query_as::<_, (String, String)>(
        "SELECT last_health_status, recent_error_samples FROM upstreams WHERE name = 'first'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(first.0, "down");
    assert!(first.1.contains("upstream_timeout"));
}

#[tokio::test]
async fn runtime_default_timeout_live_reloads_for_existing_defaulted_upstream() {
    let slow = spawn_delayed_upstream(std::time::Duration::from_millis(150)).await;
    let (app, key, pool) = app_with_single_upstream_timeout(&slow, None).await;

    let response = app
        .clone()
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_json(response).await;
    let upstream = storage::list_upstreams(&pool).await.unwrap().pop().unwrap();
    assert_eq!(upstream.timeout_ms_is_explicit, 0);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/settings",
            &key,
            json!({ "default_request_timeout_ms": 20 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(to_json(response).await["error"]["code"], "upstream_timeout");
}

#[tokio::test]
async fn admin_create_upstream_can_use_runtime_default_timeout_mode() {
    let (app, key, pool) = test_app_with_pool(None).await;

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/upstreams",
            &key,
            json!({
                "name": "api-default-timeout",
                "base_url": "http://127.0.0.1:9",
                "api_key": "sk-default-timeout",
                "enabled": true,
                "priority": 7,
                "weight": 1,
                "max_retries": 0,
                "health_check_path": "/v1/models"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let created = to_json(response).await;
    assert_eq!(created["timeout_ms"], 120_000);
    assert_eq!(created["timeout_ms_is_explicit"], 0);

    app.clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/settings",
            &key,
            json!({ "default_request_timeout_ms": 42 }),
        ))
        .await
        .unwrap();
    let response = app
        .oneshot(empty_request("GET", "/api/admin/upstreams", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let upstreams = to_json(response).await;
    let listed = upstreams
        .as_array()
        .unwrap()
        .iter()
        .find(|upstream| upstream["name"] == "api-default-timeout")
        .unwrap();
    assert_eq!(listed["timeout_ms"], 42);
    assert_eq!(listed["timeout_ms_is_explicit"], 0);

    let stored: (i64,) =
        sqlx::query_as("SELECT timeout_ms_is_explicit FROM upstreams WHERE name = ?")
            .bind("api-default-timeout")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored.0, 0);
}

#[tokio::test]
async fn admin_patch_preserves_default_timeout_mode_when_omitted() {
    let slow = spawn_delayed_upstream(std::time::Duration::from_millis(150)).await;
    let (app, key, pool) = app_with_single_upstream_timeout(&slow, None).await;
    let upstream = storage::list_upstreams(&pool).await.unwrap().pop().unwrap();

    let response = app
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/upstreams/{}", upstream.id),
            &key,
            json!({ "priority": 11 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let updated = to_json(response).await;
    assert_eq!(updated["priority"], 11);
    assert_eq!(updated["timeout_ms_is_explicit"], 0);

    let stored = storage::get_upstream(&pool, &upstream.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.timeout_ms_is_explicit, 0);
}

#[tokio::test]
async fn explicit_upstream_timeout_ignores_runtime_default_changes() {
    let slow = spawn_delayed_upstream(std::time::Duration::from_millis(150)).await;
    let (app, key, pool) = app_with_single_upstream_timeout(&slow, Some(500)).await;
    let upstream = storage::list_upstreams(&pool).await.unwrap().pop().unwrap();
    assert_eq!(upstream.timeout_ms_is_explicit, 1);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/settings",
            &key,
            json!({ "default_request_timeout_ms": 20 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_json(response).await["model_seen"], "delayed-model");
}

#[tokio::test]
async fn admin_patch_can_reset_explicit_timeout_to_runtime_default() {
    let slow = spawn_delayed_upstream(std::time::Duration::from_millis(150)).await;
    let (app, key, pool) = app_with_single_upstream_timeout(&slow, Some(500)).await;
    let upstream = storage::list_upstreams(&pool).await.unwrap().pop().unwrap();
    assert_eq!(upstream.timeout_ms_is_explicit, 1);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/upstreams/{}", upstream.id),
            &key,
            json!({ "timeout_ms": null }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let updated = to_json(response).await;
    assert_eq!(updated["timeout_ms_is_explicit"], 0);

    app.clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/settings",
            &key,
            json!({ "default_request_timeout_ms": 20 }),
        ))
        .await
        .unwrap();

    let response = app
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(to_json(response).await["error"]["code"], "upstream_timeout");
}

#[tokio::test]
async fn body_read_timeout_retries_next_eligible_upstream() {
    let stalled = spawn_body_stall_upstream(std::time::Duration::from_millis(150)).await;
    let healthy = spawn_mock_upstream().await;
    let (app, key, pool) =
        app_with_two_upstreams_and_retries_timeout(&stalled, &healthy, 1, 20).await;

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
    assert_eq!(
        to_json(response).await["model_seen"],
        "second-upstream-model"
    );
    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 2);
    assert!(logs.iter().any(|log| {
        log.status_code == Some(504) && log.error_code.as_deref() == Some("upstream_timeout")
    }));
    assert!(logs.iter().any(|log| log.status_code == Some(200)));

    let first = sqlx::query_as::<_, (String, String)>(
        "SELECT last_health_status, recent_error_samples FROM upstreams WHERE name = 'first'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(first.0, "down");
    assert!(first.1.contains("upstream_timeout"));
}

#[tokio::test]
async fn upstream_max_retries_limits_fallback_attempts() {
    let failing = spawn_status_upstream(StatusCode::SERVICE_UNAVAILABLE).await;
    let healthy = spawn_mock_upstream().await;
    let (app, key, pool) = app_with_two_upstreams_and_retries(&failing, &healthy, 0).await;

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

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].status_code, Some(503));
}

#[tokio::test]
async fn streaming_response_is_not_retried() {
    let failing_stream = spawn_sse_status_upstream(StatusCode::SERVICE_UNAVAILABLE).await;
    let healthy = spawn_mock_upstream().await;
    let (app, key, pool) = app_with_two_upstreams(&failing_stream, &healthy).await;

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
                        "stream": true,
                        "input": []
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let _ = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].status_code, Some(503));
}

#[tokio::test]
async fn successful_streaming_response_updates_daily_usage_with_tokens() {
    let upstream = spawn_usage_sse_upstream(11, 13, 24).await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    let (gateway_url, gateway_handle) = spawn_gateway_server(app).await;

    let response = reqwest::Client::new()
        .post(format!("{gateway_url}/responses"))
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "text/event-stream")
        .json(&json!({
            "model": "codex-mini",
            "stream": true,
            "input": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.unwrap();
    assert!(body.contains("response.completed"));

    let logs = wait_for_request_logs(&pool, 1).await;
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].status_code, Some(200));
    assert_eq!(logs[0].stream, 1);
    assert_eq!(logs[0].usage_source, "upstream");
    assert_eq!(logs[0].prompt_tokens, 11);
    assert_eq!(logs[0].completion_tokens, 13);
    assert_eq!(logs[0].total_tokens, 24);

    let usage = storage::list_daily_usage(&pool, None).await.unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].request_count, 1);
    assert_eq!(usage[0].stream_count, 1);
    assert_eq!(usage[0].prompt_tokens, 11);
    assert_eq!(usage[0].completion_tokens, 13);
    assert_eq!(usage[0].total_tokens, 24);
    let (event_tokens, finalized_at) = wait_for_limit_usage_event(&pool, 24).await;
    assert_eq!(event_tokens, 24);
    assert!(finalized_at.is_some());
    assert_eq!(limit_inflight_count(&pool).await, 0);

    gateway_handle.abort();
}

#[tokio::test]
async fn streaming_response_finalizes_when_client_drops_after_completed_event() {
    let upstream = spawn_completed_then_stalling_sse_upstream(17, 19, 36).await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    let (gateway_url, gateway_handle) = spawn_gateway_server(app).await;

    let response = reqwest::Client::new()
        .post(format!("{gateway_url}/responses"))
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "text/event-stream")
        .json(&json!({
            "model": "codex-mini",
            "stream": true,
            "input": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.bytes_stream();
    let first_chunk = stream.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first_chunk).contains("response.completed"));
    drop(stream);

    let logs = wait_for_request_logs(&pool, 1).await;
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].status_code, Some(200));
    assert_eq!(logs[0].error_code, None);
    assert_eq!(logs[0].stream, 1);
    assert_eq!(logs[0].usage_source, "upstream");
    assert_eq!(logs[0].prompt_tokens, 17);
    assert_eq!(logs[0].completion_tokens, 19);
    assert_eq!(logs[0].total_tokens, 36);

    let usage = storage::list_daily_usage(&pool, None).await.unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].request_count, 1);
    assert_eq!(usage[0].stream_count, 1);
    assert_eq!(usage[0].prompt_tokens, 17);
    assert_eq!(usage[0].completion_tokens, 19);
    assert_eq!(usage[0].total_tokens, 36);
    let (event_tokens, finalized_at) = wait_for_limit_usage_event(&pool, 36).await;
    assert_eq!(event_tokens, 36);
    assert!(finalized_at.is_some());
    assert_eq!(limit_inflight_count(&pool).await, 0);

    gateway_handle.abort();
}

#[tokio::test]
async fn sse_client_disconnect_finalizes_log_and_cancels_upstream() {
    let (upstream, upstream_dropped) = spawn_cancellable_sse_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    let (gateway_url, gateway_handle) = spawn_gateway_server(app).await;

    let response = reqwest::Client::new()
        .post(format!("{gateway_url}/responses"))
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "text/event-stream")
        .json(&json!({
            "model": "codex-mini",
            "stream": true,
            "input": "stream-secret should not persist"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let mut stream = response.bytes_stream();
    let first_chunk = stream.next().await.unwrap().unwrap();
    assert!(
        String::from_utf8_lossy(&first_chunk).contains("response.created"),
        "first SSE chunk was {first_chunk:?}"
    );
    drop(stream);

    tokio::time::timeout(Duration::from_secs(2), upstream_dropped)
        .await
        .unwrap()
        .unwrap();
    let logs = wait_for_request_logs(&pool, 1).await;
    assert_eq!(logs[0].status_code, Some(499));
    assert_eq!(logs[0].error_code.as_deref(), Some("client_disconnected"));
    assert_eq!(logs[0].stream, 1);
    assert_eq!(logs[0].usage_source, "unknown");
    assert!(logs[0].finished_at.is_some());
    assert!(logs[0].output_chars > 0);

    let db_text = database_text_dump(&pool).await;
    assert!(!db_text.contains("stream-secret should not persist"));
    gateway_handle.abort();
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
async fn defaulted_health_checks_use_live_runtime_timeout() {
    let delayed_health = spawn_delayed_health_upstream(std::time::Duration::from_millis(150)).await;
    let (app, key, pool) = app_with_single_upstream_timeout(&delayed_health, None).await;
    let upstream_id: (String,) = sqlx::query_as("SELECT id FROM upstreams LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/settings",
            &key,
            json!({ "default_request_timeout_ms": 20 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(empty_request(
            "POST",
            format!("/api/admin/upstreams/{}/health", upstream_id.0),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_json(response).await["health"], "down");
    let health: (String, String) = sqlx::query_as(
        "SELECT last_health_status, recent_error_samples FROM upstreams WHERE id = ?",
    )
    .bind(&upstream_id.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(health.0, "down");
    assert!(health.1.contains("upstream_timeout"));

    storage::record_upstream_health(&pool, &upstream_id.0, "healthy", None)
        .await
        .unwrap();
    let config = test_config();
    let checked = codex_gateway::upstream::check_all_enabled_upstreams(&AppState {
        config: Arc::new(config),
        db: pool.clone(),
        http: reqwest::Client::new(),
    })
    .await
    .unwrap();
    assert_eq!(checked, 1);
    let health: (String, String) = sqlx::query_as(
        "SELECT last_health_status, recent_error_samples FROM upstreams WHERE id = ?",
    )
    .bind(&upstream_id.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(health.0, "down");
    assert!(health.1.contains("upstream_timeout"));
}

#[tokio::test]
async fn explicit_health_timeout_ignores_runtime_default_changes() {
    let delayed_health = spawn_delayed_health_upstream(std::time::Duration::from_millis(150)).await;
    let (app, key, pool) = app_with_single_upstream_timeout(&delayed_health, Some(500)).await;
    let upstream_id: (String,) = sqlx::query_as("SELECT id FROM upstreams LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/settings",
            &key,
            json!({ "default_request_timeout_ms": 20 }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(empty_request(
            "POST",
            format!("/api/admin/upstreams/{}/health", upstream_id.0),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_json(response).await["health"], "healthy");

    storage::record_upstream_health(&pool, &upstream_id.0, "down", Some("reset"))
        .await
        .unwrap();
    let checked = codex_gateway::upstream::check_all_enabled_upstreams(&AppState {
        config: Arc::new(test_config()),
        db: pool.clone(),
        http: reqwest::Client::new(),
    })
    .await
    .unwrap();
    assert_eq!(checked, 1);
    let health: (String,) = sqlx::query_as("SELECT last_health_status FROM upstreams WHERE id = ?")
        .bind(&upstream_id.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(health.0, "healthy");
}

#[tokio::test]
async fn health_worker_can_be_enabled_or_disabled_in_config() {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let mut config = test_config();
    config.health_checks_enabled = false;
    let disabled_state = AppState {
        config: std::sync::Arc::new(config.clone()),
        db: pool.clone(),
        http: reqwest::Client::new(),
    };
    assert!(codex_gateway::upstream::spawn_health_worker(disabled_state).is_none());

    config.health_checks_enabled = true;
    config.health_check_interval_ms = 100;
    let enabled_state = AppState {
        config: std::sync::Arc::new(config),
        db: pool,
        http: reqwest::Client::new(),
    };
    let handle = codex_gateway::upstream::spawn_health_worker(enabled_state);
    assert!(handle.is_some());
    handle.unwrap().abort();
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
async fn runtime_settings_validate_audit_and_precedence() {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let mut config = test_config();
    config.route_strategy = RouteStrategy::Weighted;
    config.runtime_env.route_strategy = Some(RouteStrategy::Weighted);
    let user_id = seed_user_model(&pool, Some("http://127.0.0.1:9")).await;
    let (_, key) = storage::create_api_key(
        &pool,
        &config.app_secret,
        &user_id,
        &CreateApiKey {
            name: "settings-admin".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();
    let app = build_app(AppState {
        config: Arc::new(config),
        db: pool.clone(),
        http: reqwest::Client::new(),
    });

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/settings",
            &key,
            json!({
                "route_strategy": "sticky_by_key",
                "request_log_retention_days": 12,
                "expose_debug_headers": true
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_json(response).await;
    assert_eq!(body["route_strategy"], "weighted");
    assert_eq!(
        body["database"]["settings"]["route_strategy"],
        "sticky_by_key"
    );
    let route_field = body["runtime"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["key"] == "route_strategy")
        .unwrap();
    assert_eq!(route_field["source"], "environment");
    assert_eq!(route_field["environment_value"], "weighted");
    assert_eq!(route_field["database_value"], "sticky_by_key");

    let audit_logs = storage::list_admin_audit_logs(&pool).await.unwrap();
    let settings_audit = audit_logs
        .iter()
        .find(|log| log.action == "update_system_settings")
        .unwrap();
    let metadata = settings_audit.metadata_json.as_deref().unwrap();
    assert!(metadata.contains("route_strategy"));
    assert!(!metadata.contains("test-secret"));
    assert!(!metadata.contains(&key));

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/settings",
            &key,
            json!({ "app_secret": "do-not-store" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_json(response).await;
    assert_eq!(body["error"]["code"], "invalid_setting");

    let response = app
        .oneshot(empty_request("GET", "/api/admin/settings", &key))
        .await
        .unwrap();
    let body = to_json(response).await;
    assert!(body.get("app_secret").is_none());
    assert!(!body.to_string().contains("do-not-store"));
}

#[tokio::test]
async fn admin_settings_oversized_json_is_bounded_and_structured() {
    let (app, key) = test_app(None).await;
    let padding = bytes::Bytes::from(vec![b'a'; JSON_BODY_LIMIT_BYTES + 1]);
    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/admin/settings")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from_stream(async_stream::stream! {
                    yield Ok::<_, Infallible>(bytes::Bytes::from_static(b"{\"route_strategy\":\"priority\",\"padding\":\""));
                    yield Ok::<_, Infallible>(padding);
                    yield Ok::<_, Infallible>(bytes::Bytes::from_static(b"\"}"));
                }))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_json(response).await;
    assert_eq!(body["error"]["code"], "request_body_too_large");
    assert_eq!(body["error"]["type"], "gateway_error");
}

#[tokio::test]
async fn runtime_settings_live_reload_routing_body_headers_retention_and_limits() {
    let upstream = spawn_mock_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/settings",
            &key,
            json!({
                "route_strategy": "weighted",
                "max_request_body_bytes": 40,
                "request_log_retention_days": 1,
                "daily_usage_retention_days": 1,
                "expose_debug_headers": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let oversized = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/responses")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from_stream(async_stream::stream! {
                    yield Ok::<_, Infallible>(bytes::Bytes::from_static(b"{\"model\":\"codex-mini\","));
                    yield Ok::<_, Infallible>(bytes::Bytes::from_static(b"\"input\":[\"this chunk pushes the body past forty bytes\"]}"));
                }))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        to_json(oversized).await["error"]["code"],
        "request_body_too_large"
    );

    app.clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/settings",
            &key,
            json!({ "max_request_body_bytes": 10_000 }),
        ))
        .await
        .unwrap();
    let proxied = app
        .clone()
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(proxied.status(), StatusCode::OK);
    assert_eq!(
        proxied
            .headers()
            .get("x-codex-gateway-route-strategy")
            .and_then(|value| value.to_str().ok()),
        Some("weighted")
    );
    let _ = to_json(proxied).await;
    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert!(
        logs.iter()
            .any(|log| log.route_strategy.as_deref() == Some("weighted"))
    );

    let user_id = logs[0].user_id.clone();
    let api_key_id = logs[0].api_key_id.clone();
    insert_test_log(
        &pool,
        "old-runtime-retention-log",
        &user_id,
        &api_key_id,
        None,
        None,
        (200, "2000-01-01T00:00:00.000Z"),
    )
    .await;
    insert_test_log(
        &pool,
        "new-runtime-retention-log",
        &user_id,
        &api_key_id,
        None,
        None,
        (
            200,
            &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ),
    )
    .await;
    let retention = app
        .clone()
        .oneshot(empty_request("POST", "/api/admin/retention/run", &key))
        .await
        .unwrap();
    assert_eq!(retention.status(), StatusCode::OK);
    let remaining = storage::list_request_logs(&pool, None).await.unwrap();
    assert!(
        remaining
            .iter()
            .all(|log| log.request_id != "old-runtime-retention-log")
    );
    assert!(
        remaining
            .iter()
            .any(|log| log.request_id == "new-runtime-retention-log")
    );

    let limits = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/limits/system",
            &key,
            json!({ "request_quota": 3 }),
        ))
        .await
        .unwrap();
    assert_eq!(limits.status(), StatusCode::OK);
    let settings = app
        .oneshot(empty_request("GET", "/api/admin/settings", &key))
        .await
        .unwrap();
    let settings = to_json(settings).await;
    assert_eq!(settings["default_limit_policy"]["request_quota"], 3);
}

#[tokio::test]
async fn login_issues_scoped_panel_token_without_creating_api_key_session() {
    let (app, _api_key, pool) = test_app_with_pool(None).await;
    let key_count_before: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_keys")
        .fetch_one(&pool)
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "email": "user@example.com", "password": "password" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let login = to_json(response).await;
    let token = login["token"].as_str().unwrap();
    assert!(token.starts_with("cgw_panel_"));
    assert_eq!(login["token_type"], "panel");
    assert!(login.get("plaintext").is_none());
    assert!(login.get("key").is_none());

    let key_count_after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(key_count_after.0, key_count_before.0);

    let response = app
        .clone()
        .oneshot(empty_request("GET", "/api/me", token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_json(response).await["key_prefix"], "panel");

    let response = app
        .oneshot(empty_request("GET", "/v1/models", token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn cors_rejects_untrusted_origins_when_configured() {
    let (app, _) = test_app(None).await;

    let allowed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/me")
                .header(header::ORIGIN, "http://localhost")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        allowed
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap(),
        "http://localhost"
    );

    let rejected = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/me")
                .header(header::ORIGIN, "https://evil.example")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        rejected
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none()
    );
}

#[tokio::test]
async fn upstream_secrets_are_encrypted_and_can_rotate_versions() {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let mut config = test_config();
    let upstream = storage::create_upstream(
        &pool,
        &config.app_secret,
        config.secret_key_version,
        &UpsertUpstream {
            name: "rotating".into(),
            base_url: "http://127.0.0.1:9".into(),
            api_key: "sk-version-one".into(),
            enabled: Some(true),
            priority: Some(1),
            weight: Some(1),
            timeout_ms: timeout_default(),
            max_retries: None,
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
            upstream_mappings: Some(vec![UpsertModelMapping {
                upstream_id: upstream.id.clone(),
                upstream_model_name: "upstream-codex-mini".into(),
                enabled: Some(true),
                priority: Some(1),
                weight: Some(1),
            }]),
        },
    )
    .await
    .unwrap();

    let stored: (String, i64) =
        sqlx::query_as("SELECT api_key_ciphertext, api_key_secret_version FROM upstreams")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored.1, 1);
    assert!(stored.0.starts_with("cgwenc_v1.1."));
    assert!(!stored.0.contains("sk-version-one"));
    let route = codex_gateway::routing::route_candidates(
        &pool,
        &config,
        "codex-mini",
        config.default_request_timeout_ms,
    )
    .await
    .unwrap()
    .pop()
    .unwrap();
    assert_eq!(route.upstream_api_key, "sk-version-one");

    config.secret_key_version = 2;
    storage::update_upstream(
        &pool,
        &config.app_secret,
        config.secret_key_version,
        &upstream.id,
        &codex_gateway::storage::UpdateUpstream {
            name: None,
            base_url: None,
            api_key: Some("sk-version-two".into()),
            enabled: None,
            priority: None,
            weight: None,
            timeout_ms: timeout_missing(),
            max_retries: None,
            health_check_path: None,
        },
    )
    .await
    .unwrap();

    let stored: (String, i64) =
        sqlx::query_as("SELECT api_key_ciphertext, api_key_secret_version FROM upstreams")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored.1, 2);
    assert!(stored.0.starts_with("cgwenc_v1.2."));
    assert!(!stored.0.contains("sk-version-two"));
    let route = codex_gateway::routing::route_candidates(
        &pool,
        &config,
        "codex-mini",
        config.default_request_timeout_ms,
    )
    .await
    .unwrap()
    .pop()
    .unwrap();
    assert_eq!(route.upstream_api_key, "sk-version-two");
}

#[tokio::test]
async fn legacy_plaintext_upstream_rows_are_auto_encrypted_and_still_usable() {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let config = test_config();
    let upstream_id = auth::new_id();
    let now = storage::now_string();
    sqlx::query(
        "INSERT INTO upstreams
         (id, name, base_url, api_key_ciphertext, api_key_secret_version, enabled, priority, weight,
          timeout_ms, max_retries, health_check_path, created_at, updated_at)
         VALUES (?, 'legacy', 'http://127.0.0.1:9', ?, 0, 1, 1, 1, 5000, 1, '/v1/models', ?, ?)",
    )
    .bind(&upstream_id)
    .bind("sk-legacy-plaintext")
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    storage::create_model(
        &pool,
        &UpsertModel {
            public_name: "codex-mini".into(),
            description: None,
            enabled: Some(true),
            visible_to_users: Some(true),
            upstream_mappings: Some(vec![UpsertModelMapping {
                upstream_id: upstream_id.clone(),
                upstream_model_name: "legacy-upstream-model".into(),
                enabled: Some(true),
                priority: Some(1),
                weight: Some(1),
            }]),
        },
    )
    .await
    .unwrap();
    assert!(
        database_text_dump(&pool)
            .await
            .contains("sk-legacy-plaintext")
    );

    let upgraded = storage::upgrade_legacy_upstream_secrets(&pool, &config)
        .await
        .unwrap();
    assert_eq!(upgraded, 1);

    let db_text = database_text_dump(&pool).await;
    assert!(!db_text.contains("sk-legacy-plaintext"));
    let stored: (String, i64) = sqlx::query_as(
        "SELECT api_key_ciphertext, api_key_secret_version FROM upstreams WHERE id = ?",
    )
    .bind(&upstream_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored.1, config.secret_key_version);
    assert!(stored.0.starts_with("cgwenc_v1."));

    let route = codex_gateway::routing::route_candidates(
        &pool,
        &config,
        "codex-mini",
        config.default_request_timeout_ms,
    )
    .await
    .unwrap()
    .pop()
    .unwrap();
    assert_eq!(route.upstream_api_key, "sk-legacy-plaintext");
}

#[tokio::test]
async fn production_like_config_refuses_default_or_weak_secrets() {
    assert!(
        Config::from_lookup(|key| {
            (key == "CODEX_GATEWAY_ENV").then(|| "production".to_string())
        })
        .is_err()
    );

    assert!(
        Config::from_lookup(|key| match key {
            "CODEX_GATEWAY_ENV" => Some("production".to_string()),
            "CODEX_GATEWAY_APP_SECRET" => Some("short".to_string()),
            _ => None,
        })
        .is_err()
    );
}

#[tokio::test]
async fn database_scan_does_not_reveal_raw_secrets_or_payloads() {
    let upstream = spawn_mock_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;

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
                        "input": "raw prompt should not persist",
                        "client_metadata": {
                            "session_id": "scan-session-secret",
                            "cookie": "scan-cookie-secret"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let db_text = database_text_dump(&pool).await;
    for forbidden in [
        "sk-upstream-test",
        &key,
        "password",
        "scan-cookie-secret",
        "raw prompt should not persist",
        "completion",
        "scan-session-secret",
    ] {
        assert!(
            !db_text.contains(forbidden),
            "database text contained forbidden value {forbidden}"
        );
    }
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

    let audit_logs = storage::list_admin_audit_logs(&pool).await.unwrap();
    assert!(audit_logs.len() >= 9);
    assert!(
        audit_logs
            .iter()
            .any(|log| log.action == "reset_user_password"
                && log.resource_type == "user"
                && log.status == "success")
    );
    let audit_dump = audit_logs
        .iter()
        .filter_map(|log| log.metadata_json.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!audit_dump.contains("new-pass-123"));
    assert!(!audit_dump.contains("sk-rotated"));
    assert!(!audit_dump.contains(created_key["plaintext"].as_str().unwrap()));
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
async fn user_self_service_usage_models_and_key_summaries_are_scoped() {
    let (app, admin_key, pool) = test_app_with_pool(None).await;
    let config = test_config();
    let admin_key_row = storage::list_api_keys(&pool)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let admin_user_id = admin_key_row.user_id.clone();
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
    let (user_key_row, user_key) = storage::create_api_key(
        &pool,
        &config.app_secret,
        &user_id,
        &CreateApiKey {
            name: "user-key".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();
    let model_id: String =
        sqlx::query_scalar("SELECT id FROM models WHERE public_name = 'codex-mini'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let upstream_id: String = sqlx::query_scalar("SELECT id FROM upstreams LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let hidden = storage::create_model(
        &pool,
        &UpsertModel {
            public_name: "admin-shadow".into(),
            description: None,
            enabled: Some(true),
            visible_to_users: Some(false),
            upstream_mappings: None,
        },
    )
    .await
    .unwrap();

    insert_test_log(
        &pool,
        "admin-usage",
        &admin_user_id,
        &admin_key_row.id,
        Some(&model_id),
        Some(&upstream_id),
        (200, "2026-07-09T10:00:00.000Z"),
    )
    .await;
    insert_test_log(
        &pool,
        "user-usage",
        &user_id,
        &user_key_row.id,
        Some(&model_id),
        Some(&upstream_id),
        (500, "2026-07-09T11:00:00.000Z"),
    )
    .await;
    storage::upsert_limit_policy(
        &pool,
        "user",
        &admin_user_id,
        &storage::LimitPolicyPatch {
            request_quota: limit_set(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    storage::upsert_limit_policy(
        &pool,
        "user",
        &user_id,
        &storage::LimitPolicyPatch {
            request_quota: limit_set(7),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let models = app
        .clone()
        .oneshot(empty_request("GET", "/api/models", &user_key))
        .await
        .unwrap();
    assert_eq!(models.status(), StatusCode::OK);
    let models = to_json(models).await;
    assert!(
        models
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["public_name"] == "codex-mini")
    );
    assert!(
        !models
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["id"] == hidden.id)
    );

    let scoped_requests = app
        .clone()
        .oneshot(empty_request(
            "GET",
            format!("/api/requests?user_id={admin_user_id}"),
            &user_key,
        ))
        .await
        .unwrap();
    assert_eq!(scoped_requests.status(), StatusCode::OK);
    let scoped_requests = to_json(scoped_requests).await;
    assert_eq!(scoped_requests.as_array().unwrap().len(), 1);
    assert_eq!(scoped_requests[0]["user_id"], user_id);

    let foreign_key_requests = app
        .clone()
        .oneshot(empty_request(
            "GET",
            format!("/api/requests?key_id={}", admin_key_row.id),
            &user_key,
        ))
        .await
        .unwrap();
    assert_eq!(foreign_key_requests.status(), StatusCode::OK);
    assert!(
        to_json(foreign_key_requests)
            .await
            .as_array()
            .unwrap()
            .is_empty()
    );

    let foreign_usage = app
        .clone()
        .oneshot(empty_request(
            "GET",
            format!("/api/usage/daily?key_id={}", admin_key_row.id),
            &user_key,
        ))
        .await
        .unwrap();
    assert_eq!(foreign_usage.status(), StatusCode::OK);
    assert!(to_json(foreign_usage).await.as_array().unwrap().is_empty());

    let foreign_summary = app
        .clone()
        .oneshot(empty_request(
            "GET",
            format!("/api/usage/summary?key_id={}", admin_key_row.id),
            &user_key,
        ))
        .await
        .unwrap();
    assert_eq!(foreign_summary.status(), StatusCode::OK);
    let foreign_summary = to_json(foreign_summary).await;
    assert_eq!(foreign_summary["totals"]["request_count"], 0);
    assert!(
        foreign_summary["recent_failures"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let foreign_key_summary = app
        .clone()
        .oneshot(empty_request(
            "GET",
            format!("/api/api-keys/{}/usage", admin_key_row.id),
            &user_key,
        ))
        .await
        .unwrap();
    assert_eq!(foreign_key_summary.status(), StatusCode::FORBIDDEN);

    let own_key_summary = app
        .clone()
        .oneshot(empty_request(
            "GET",
            format!("/api/api-keys/{}/usage", user_key_row.id),
            &user_key,
        ))
        .await
        .unwrap();
    assert_eq!(own_key_summary.status(), StatusCode::OK);
    let own_key_summary = to_json(own_key_summary).await;
    assert_eq!(own_key_summary["api_key"]["id"], user_key_row.id);
    assert_eq!(own_key_summary["usage"]["totals"]["request_count"], 1);
    assert_eq!(own_key_summary["usage"]["totals"]["error_count"], 1);
    assert_eq!(
        own_key_summary["usage"]["recent_failures"][0]["request_id"],
        "user-usage"
    );

    let limits = app
        .clone()
        .oneshot(empty_request("GET", "/api/limits", &user_key))
        .await
        .unwrap();
    assert_eq!(limits.status(), StatusCode::OK);
    let limits = to_json(limits).await;
    assert_eq!(limits["user"]["subject_id"], user_id);
    assert_eq!(limits["user"]["request_quota"]["limit"], 7);
    assert!(
        limits["api_keys"]
            .as_array()
            .unwrap()
            .iter()
            .all(|state| state["subject_id"] != admin_key_row.id)
    );

    let admin_models = app
        .oneshot(empty_request("GET", "/api/admin/models", &admin_key))
        .await
        .unwrap();
    assert_eq!(admin_models.status(), StatusCode::OK);
    assert!(
        to_json(admin_models)
            .await
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["id"] == hidden.id)
    );
}

#[tokio::test]
async fn admin_usage_apis_can_inspect_global_dimensions() {
    let (app, admin_key, pool) = test_app_with_pool(None).await;
    let config = test_config();
    let admin_key_row = storage::list_api_keys(&pool)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let user_id = storage::ensure_user(
        &pool,
        &CreateUser {
            email: "usage-user@example.com".into(),
            password: "password".into(),
            role: "user".into(),
            display_name: None,
        },
    )
    .await
    .unwrap();
    let (user_key_row, _user_key) = storage::create_api_key(
        &pool,
        &config.app_secret,
        &user_id,
        &CreateApiKey {
            name: "usage-key".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();
    let model_id: String =
        sqlx::query_scalar("SELECT id FROM models WHERE public_name = 'codex-mini'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let upstream_id: String = sqlx::query_scalar("SELECT id FROM upstreams LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    insert_test_log(
        &pool,
        "admin-global-usage",
        &admin_key_row.user_id,
        &admin_key_row.id,
        Some(&model_id),
        Some(&upstream_id),
        (200, "2026-07-09T10:00:00.000Z"),
    )
    .await;
    insert_test_log(
        &pool,
        "user-global-usage",
        &user_id,
        &user_key_row.id,
        Some(&model_id),
        Some(&upstream_id),
        (502, "2026-07-09T11:00:00.000Z"),
    )
    .await;

    let global_summary = app
        .clone()
        .oneshot(empty_request("GET", "/api/admin/usage/summary", &admin_key))
        .await
        .unwrap();
    assert_eq!(global_summary.status(), StatusCode::OK);
    let global_summary = to_json(global_summary).await;
    assert_eq!(global_summary["totals"]["request_count"], 2);
    assert_eq!(global_summary["totals"]["error_count"], 1);

    let key_usage = app
        .clone()
        .oneshot(empty_request(
            "GET",
            format!("/api/admin/usage/daily?key_id={}", user_key_row.id),
            &admin_key,
        ))
        .await
        .unwrap();
    assert_eq!(key_usage.status(), StatusCode::OK);
    let key_usage = to_json(key_usage).await;
    assert_eq!(key_usage.as_array().unwrap().len(), 1);
    assert_eq!(key_usage[0]["api_key_id"], user_key_row.id);

    let model_usage = app
        .clone()
        .oneshot(empty_request(
            "GET",
            format!("/api/admin/usage/daily?model_id={model_id}"),
            &admin_key,
        ))
        .await
        .unwrap();
    assert_eq!(model_usage.status(), StatusCode::OK);
    assert_eq!(to_json(model_usage).await.as_array().unwrap().len(), 2);

    let user_key_summary = app
        .clone()
        .oneshot(empty_request(
            "GET",
            format!("/api/admin/api-keys/{}/usage", user_key_row.id),
            &admin_key,
        ))
        .await
        .unwrap();
    assert_eq!(user_key_summary.status(), StatusCode::OK);
    let user_key_summary = to_json(user_key_summary).await;
    assert_eq!(user_key_summary["usage"]["totals"]["request_count"], 1);
    assert_eq!(user_key_summary["usage"]["totals"]["error_count"], 1);
    assert_eq!(
        user_key_summary["usage"]["recent_failures"][0]["request_id"],
        "user-global-usage"
    );

    let user_requests = app
        .oneshot(empty_request(
            "GET",
            format!("/api/admin/requests?key_id={}", user_key_row.id),
            &admin_key,
        ))
        .await
        .unwrap();
    assert_eq!(user_requests.status(), StatusCode::OK);
    let user_requests = to_json(user_requests).await;
    assert_eq!(user_requests.as_array().unwrap().len(), 1);
    assert_eq!(user_requests[0]["request_id"], "user-global-usage");
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
            route_strategy: None,
            route_decision_json: None,
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

#[tokio::test]
async fn admin_metrics_are_sanitized_and_surface_failing_upstreams() {
    let failing = spawn_status_upstream(StatusCode::SERVICE_UNAVAILABLE).await;
    let (app, key, pool) = test_app_with_pool(Some(&failing)).await;

    let response = app
        .clone()
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
                        "input": "prompt-secret-material",
                        "client_metadata": {
                            "session_id": "session-secret-material"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let metrics_response = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/metrics")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(metrics_response.status(), StatusCode::OK);
    let metrics = to_json(metrics_response).await;
    assert_eq!(metrics["request_count"], 1);
    assert_eq!(metrics["error_count"], 1);
    assert_eq!(
        metrics["upstream_health"][0]["last_health_status"],
        "degraded"
    );
    assert_eq!(metrics["upstream_health"][0]["error_count"], 1);

    let metrics_text = metrics.to_string();
    assert!(!metrics_text.contains("prompt-secret-material"));
    assert!(!metrics_text.contains("session-secret-material"));
    assert!(!metrics_text.contains("sk-upstream-test"));

    let health: (String,) =
        sqlx::query_as("SELECT last_health_status FROM upstreams WHERE name = 'mock'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(health.0, "degraded");
}

#[tokio::test]
async fn duplicate_client_request_ids_do_not_suppress_logs_usage_or_metrics() {
    let upstream = spawn_mock_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;

    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/responses")
                    .header(header::AUTHORIZATION, format!("Bearer {key}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("x-request-id", "duplicate-client-correlation")
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
        assert_eq!(
            response
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok()),
            Some("duplicate-client-correlation")
        );
    }

    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    let duplicate_logs = logs
        .iter()
        .filter(|log| log.request_id == "duplicate-client-correlation")
        .collect::<Vec<_>>();
    assert_eq!(duplicate_logs.len(), 2);

    let usage = storage::list_daily_usage(&pool, None).await.unwrap();
    assert_eq!(usage.iter().map(|row| row.request_count).sum::<i64>(), 2);
    assert_eq!(usage.iter().map(|row| row.total_tokens).sum::<i64>(), 6);

    let metrics_response = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/metrics")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(metrics_response.status(), StatusCode::OK);
    let metrics = to_json(metrics_response).await;
    assert_eq!(metrics["request_count"], 2);
    assert_eq!(metrics["token_usage"]["total_tokens"], 6);
}

#[tokio::test]
async fn request_log_filters_work_for_admin_api() {
    let (app, key, pool) = test_app_with_pool(None).await;
    let user_id: String = sqlx::query_scalar("SELECT id FROM users LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let api_key_id: String = sqlx::query_scalar("SELECT id FROM api_keys LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let model_id: String = sqlx::query_scalar("SELECT id FROM models LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let upstream_id: String = sqlx::query_scalar("SELECT id FROM upstreams LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    insert_test_log(
        &pool,
        "filter-match",
        &user_id,
        &api_key_id,
        Some(&model_id),
        Some(&upstream_id),
        (502, "2026-07-08T12:00:00.000Z"),
    )
    .await;
    insert_test_log(
        &pool,
        "filter-status-miss",
        &user_id,
        &api_key_id,
        Some(&model_id),
        Some(&upstream_id),
        (200, "2026-07-08T13:00:00.000Z"),
    )
    .await;
    insert_test_log(
        &pool,
        "filter-date-miss",
        &user_id,
        &api_key_id,
        Some(&model_id),
        Some(&upstream_id),
        (502, "2026-07-01T12:00:00.000Z"),
    )
    .await;

    let uri = format!(
        "/api/admin/requests?user_id={user_id}&key_id={api_key_id}&model_id={model_id}&upstream_id={upstream_id}&status=502&from=2026-07-08&to=2026-07-08"
    );
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let logs = to_json(response).await;
    assert_eq!(logs.as_array().unwrap().len(), 1);
    assert_eq!(logs[0]["request_id"], "filter-match");
}

#[tokio::test]
async fn request_log_drilldown_filters_apply_error_latency_and_user_scope() {
    let (app, admin_key, pool) = test_app_with_pool(None).await;
    let admin_api_key = storage::list_api_keys(&pool).await.unwrap().remove(0);
    let model_id = storage::list_models(&pool).await.unwrap().remove(0).id;
    let upstream_id = storage::list_upstreams(&pool).await.unwrap().remove(0).id;
    let user_id = storage::ensure_user(
        &pool,
        &CreateUser {
            email: "request-filter-user@example.com".into(),
            password: "password".into(),
            role: "user".into(),
            display_name: None,
        },
    )
    .await
    .unwrap();
    let (user_key, user_plaintext) = storage::create_api_key(
        &pool,
        "test-secret",
        &user_id,
        &CreateApiKey {
            name: "request-filter-user-key".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();

    insert_test_log_with_latency(
        &pool,
        "admin-fast-error",
        &admin_api_key.user_id,
        &admin_api_key.id,
        Some(&model_id),
        Some(&upstream_id),
        (500, 120, "2026-07-10T08:00:00.000Z"),
    )
    .await;
    insert_test_log_with_latency(
        &pool,
        "admin-slow-error",
        &admin_api_key.user_id,
        &admin_api_key.id,
        Some(&model_id),
        Some(&upstream_id),
        (502, 2_500, "2026-07-10T09:00:00.000Z"),
    )
    .await;
    insert_test_log_with_latency(
        &pool,
        "admin-slow-ok",
        &admin_api_key.user_id,
        &admin_api_key.id,
        Some(&model_id),
        Some(&upstream_id),
        (200, 2_750, "2026-07-10T10:00:00.000Z"),
    )
    .await;
    insert_test_log_with_latency(
        &pool,
        "user-slow-error",
        &user_id,
        &user_key.id,
        Some(&model_id),
        Some(&upstream_id),
        (500, 2_800, "2026-07-10T11:00:00.000Z"),
    )
    .await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/requests?status=error&latency_min_ms=1000&latency_max_ms=2600&from=2026-07-10&to=2026-07-10")
                .header(header::AUTHORIZATION, format!("Bearer {admin_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let logs = to_json(response).await;
    let request_ids: Vec<&str> = logs
        .as_array()
        .unwrap()
        .iter()
        .map(|log| log["request_id"].as_str().unwrap())
        .collect();
    assert_eq!(request_ids, vec!["admin-slow-error"]);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/requests?status=error&latency_min_ms=1000&latency_max_ms=3000&from=2026-07-10&to=2026-07-10")
                .header(header::AUTHORIZATION, format!("Bearer {user_plaintext}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let logs = to_json(response).await;
    let request_ids: Vec<&str> = logs
        .as_array()
        .unwrap()
        .iter()
        .map(|log| log["request_id"].as_str().unwrap())
        .collect();
    assert_eq!(request_ids, vec!["user-slow-error"]);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/requests?key_id={}&status=error&latency_min_ms=1000&latency_max_ms=3000&from=2026-07-10&to=2026-07-10",
                    admin_api_key.id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {user_plaintext}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let logs = to_json(response).await;
    assert_eq!(logs.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn retention_policy_is_idempotent() {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let config = test_config();
    let user_id = seed_user_model(&pool, Some("http://127.0.0.1:9")).await;
    let key = storage::create_api_key(
        &pool,
        &config.app_secret,
        &user_id,
        &CreateApiKey {
            name: "retention".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap()
    .0;
    insert_test_log(
        &pool,
        "old-retention-log",
        &user_id,
        &key.id,
        None,
        None,
        (200, "2026-06-01T00:00:00.000Z"),
    )
    .await;
    insert_test_log(
        &pool,
        "new-retention-log",
        &user_id,
        &key.id,
        None,
        None,
        (200, "2026-07-09T00:00:00.000Z"),
    )
    .await;

    let policy = storage::RetentionPolicy {
        request_log_retention_days: 30,
        daily_usage_retention_days: 30,
    };
    let now = chrono::DateTime::parse_from_rfc3339("2026-07-10T00:00:00.000Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let first = storage::apply_retention_at(&pool, &policy, now)
        .await
        .unwrap();
    let second = storage::apply_retention_at(&pool, &policy, now)
        .await
        .unwrap();

    assert_eq!(first.request_logs_deleted, 1);
    assert_eq!(first.daily_usage_deleted, 1);
    assert_eq!(second.request_logs_deleted, 0);
    assert_eq!(second.daily_usage_deleted, 0);
    let remaining_logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(remaining_logs.len(), 1);
    assert_eq!(remaining_logs[0].request_id, "new-retention-log");
    let remaining_usage = storage::list_daily_usage(&pool, None).await.unwrap();
    assert_eq!(remaining_usage.len(), 1);
    assert_eq!(remaining_usage[0].date, "2026-07-09");
}

#[tokio::test]
async fn analytics_endpoint_aggregates_filters_and_remains_sanitized() {
    let (app, admin_key, pool) = test_app_with_pool(None).await;
    let admin_api_key = storage::list_api_keys(&pool).await.unwrap().remove(0);
    let model_id = storage::list_models(&pool).await.unwrap().remove(0).id;
    let upstream_id = storage::list_upstreams(&pool).await.unwrap().remove(0).id;
    let user_id = storage::ensure_user(
        &pool,
        &CreateUser {
            email: "analytics-user@example.com".into(),
            password: "password".into(),
            role: "user".into(),
            display_name: None,
        },
    )
    .await
    .unwrap();
    let (user_key, user_plaintext) = storage::create_api_key(
        &pool,
        "test-secret",
        &user_id,
        &CreateApiKey {
            name: "analytics-user-key".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();

    insert_test_log_with_latency(
        &pool,
        "analytics-admin-ok",
        &admin_api_key.user_id,
        &admin_api_key.id,
        Some(&model_id),
        Some(&upstream_id),
        (200, 125, "2026-07-10T08:00:00.000Z"),
    )
    .await;
    insert_test_log_with_latency(
        &pool,
        "analytics-admin-fail",
        &admin_api_key.user_id,
        &admin_api_key.id,
        Some(&model_id),
        Some(&upstream_id),
        (500, 1_500, "2026-07-10T09:00:00.000Z"),
    )
    .await;
    storage::insert_request_log(
        &pool,
        RequestLogInsert {
            request_id: "analytics-user-secret".into(),
            user_id: user_id.clone(),
            api_key_id: user_key.id.clone(),
            model_id: Some(model_id.clone()),
            upstream_id: Some(upstream_id.clone()),
            method: "POST".into(),
            path: "/responses".into(),
            status_code: Some(502),
            error_code: Some("upstream_error".into()),
            stream: false,
            usage: UsageSnapshot {
                prompt_tokens: 11,
                completion_tokens: 13,
                total_tokens: 24,
                ..UsageSnapshot::default()
            },
            input_chars: 123,
            output_chars: 456,
            latency_ms: 12_000,
            started_at: "2026-07-10T10:00:00.000Z".into(),
            finished_at: "2026-07-10T10:00:01.000Z".into(),
            client_ip_hash: Some("ip-hash-secret".into()),
            user_agent: Some("cookie secret agent".into()),
            client_metadata_sanitized: Some(
                json!({
                    "session_id_hash": "session-hash",
                    "raw_cookie": "cookie-secret",
                    "prompt": "prompt-secret"
                })
                .to_string(),
            ),
            route_strategy: Some("priority".into()),
            route_decision_json: Some(json!({ "api_key": "sk-secret" }).to_string()),
        },
    )
    .await
    .unwrap();

    let admin_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/admin/analytics?model_id={model_id}&upstream_id={upstream_id}&from=2026-07-10T00:00:00Z&to=2026-07-10T23:59:59Z"
                ))
                .header(header::AUTHORIZATION, format!("Bearer {admin_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_response.status(), StatusCode::OK);
    let body = to_json(admin_response).await;
    assert_eq!(body["requests_24h"].as_array().unwrap().len(), 3);
    assert_eq!(body["token_usage_7d"][0]["total_tokens"], 34);
    assert_eq!(body["model_share"][0]["id"], model_id);
    assert_eq!(body["model_share"][0]["request_count"], 3);
    assert_eq!(body["upstream_error_rate"][0]["upstream_id"], upstream_id);
    assert_eq!(body["upstream_error_rate"][0]["error_count"], 2);
    assert_eq!(body["latency_buckets"].as_array().unwrap().len(), 3);

    let admin_key_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/admin/analytics?key_id={}&from=2026-07-10&to=2026-07-10",
                    user_key.id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {admin_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_key_response.status(), StatusCode::OK);
    let admin_key_body = to_json(admin_key_response).await;
    assert_eq!(admin_key_body["model_share"][0]["request_count"], 1);
    assert_eq!(admin_key_body["token_usage_7d"][0]["total_tokens"], 24);
    assert_eq!(admin_key_body["upstream_error_rate"][0]["error_count"], 1);

    let serialized = body.to_string();
    for sensitive in [
        "prompt-secret",
        "cookie-secret",
        "sk-secret",
        "client_metadata_sanitized",
        "client_ip_hash",
        "user_agent",
        "route_decision_json",
    ] {
        assert!(
            !serialized.contains(sensitive),
            "{sensitive} leaked in analytics"
        );
    }

    let user_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/analytics?user_id={}&from=2026-07-10T00:00:00Z&to=2026-07-10T23:59:59Z",
                    admin_api_key.user_id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {user_plaintext}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(user_response.status(), StatusCode::OK);
    let body = to_json(user_response).await;
    assert_eq!(body["model_share"][0]["request_count"], 1);
    assert_eq!(body["token_usage_7d"][0]["total_tokens"], 24);
    assert_eq!(body["user_error_rate"].as_array().unwrap().len(), 0);

    let user_cross_key_response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/analytics?key_id={}&from=2026-07-10&to=2026-07-10",
                    admin_api_key.id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {user_plaintext}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(user_cross_key_response.status(), StatusCode::OK);
    let body = to_json(user_cross_key_response).await;
    assert_eq!(body["model_share"].as_array().unwrap().len(), 0);
    assert_eq!(body["token_usage_7d"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn analytics_endpoint_applies_status_filter() {
    let (app, admin_key, pool) = test_app_with_pool(None).await;
    let admin_api_key = storage::list_api_keys(&pool).await.unwrap().remove(0);
    let model_id = storage::list_models(&pool).await.unwrap().remove(0).id;
    let upstream_id = storage::list_upstreams(&pool).await.unwrap().remove(0).id;

    insert_test_log(
        &pool,
        "analytics-filter-ok",
        &admin_api_key.user_id,
        &admin_api_key.id,
        Some(&model_id),
        Some(&upstream_id),
        (200, "2026-07-10T08:00:00.000Z"),
    )
    .await;
    insert_test_log(
        &pool,
        "analytics-filter-fail",
        &admin_api_key.user_id,
        &admin_api_key.id,
        Some(&model_id),
        Some(&upstream_id),
        (500, "2026-07-10T09:00:00.000Z"),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/analytics?status=500&from=2026-07-10&to=2026-07-10")
                .header(header::AUTHORIZATION, format!("Bearer {admin_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_json(response).await;
    assert_eq!(body["model_share"][0]["request_count"], 1);
    assert_eq!(body["model_share"][0]["error_count"], 1);
    assert_eq!(body["requests_24h"][0]["error_count"], 1);
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

async fn app_with_single_upstream_timeout(
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
    };
    (build_app(state), plaintext, pool)
}

async fn app_with_two_upstreams(first_url: &str, second_url: &str) -> (Router, String, SqlitePool) {
    app_with_two_upstreams_and_retries(first_url, second_url, 1).await
}

async fn app_with_two_upstreams_and_retries(
    first_url: &str,
    second_url: &str,
    first_max_retries: i64,
) -> (Router, String, SqlitePool) {
    app_with_two_upstreams_and_retries_timeout(first_url, second_url, first_max_retries, 5_000)
        .await
}

async fn app_with_two_upstreams_and_retries_timeout(
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
    };
    (build_app(state), plaintext, pool)
}

async fn seed_weighted_model(pool: &SqlitePool, config: &Config) {
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

fn test_config() -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        database_url: "sqlite://:memory:".into(),
        app_secret: "test-secret".into(),
        secret_key_version: 1,
        public_url: "http://localhost".into(),
        cors_allowed_origins: vec!["http://localhost".into()],
        log_level: "info".into(),
        route_strategy: RouteStrategy::Priority,
        default_request_timeout_ms: 120_000,
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

async fn insert_test_log(
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

async fn insert_test_log_with_latency(
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

async fn spawn_mock_upstream() -> String {
    let app = Router::new()
        .route("/responses", post(mock_responses))
        .route("/responses/compact", post(mock_compact))
        .route("/v1/models", get(mock_models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_gateway_server(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), handle)
}

async fn spawn_cancellable_sse_upstream() -> (String, tokio::sync::oneshot::Receiver<()>) {
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

async fn spawn_usage_sse_upstream(
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

async fn spawn_completed_then_stalling_sse_upstream(
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

async fn spawn_counting_upstream(delay: Duration) -> (String, Arc<AtomicUsize>) {
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
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
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

async fn spawn_blocking_counting_upstream() -> (
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

async fn spawn_delayed_upstream(delay: std::time::Duration) -> String {
    let app = Router::new()
        .route(
            "/responses",
            post(move || async move {
                tokio::time::sleep(delay).await;
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

async fn spawn_delayed_health_upstream(delay: std::time::Duration) -> String {
    let app = Router::new()
        .route("/responses", post(mock_responses))
        .route(
            "/v1/models",
            get(move || async move {
                tokio::time::sleep(delay).await;
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

async fn spawn_body_stall_upstream(delay: std::time::Duration) -> String {
    let app = Router::new()
        .route(
            "/responses",
            post(move || async move {
                let body = Body::from_stream(async_stream::stream! {
                    yield Ok::<_, std::convert::Infallible>(bytes::Bytes::from_static(b"{\"partial\":"));
                    tokio::time::sleep(delay).await;
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

async fn spawn_sse_status_upstream(status: StatusCode) -> String {
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

async fn mock_compact(
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

async fn mock_models() -> impl IntoResponse {
    Json(json!({ "object": "list", "data": [] }))
}

async fn to_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn database_text_dump(pool: &SqlitePool) -> String {
    let tables = sqlx::query(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut dump = String::new();
    for table in tables {
        let table_name: String = table.get("name");
        let escaped_table = table_name.replace('"', "\"\"");
        let columns = sqlx::query(&format!("PRAGMA table_info(\"{escaped_table}\")"))
            .fetch_all(pool)
            .await
            .unwrap();
        for column in columns {
            let column_name: String = column.get("name");
            let column_type: String = column.get("type");
            if !column_type.to_ascii_uppercase().contains("TEXT") {
                continue;
            }
            let escaped_column = column_name.replace('"', "\"\"");
            let rows = sqlx::query(&format!(
                "SELECT \"{escaped_column}\" AS value FROM \"{escaped_table}\" WHERE \"{escaped_column}\" IS NOT NULL",
            ))
            .fetch_all(pool)
            .await
            .unwrap();
            for row in rows {
                let value: String = row.get("value");
                dump.push_str(&value);
                dump.push('\n');
            }
        }
    }
    dump
}

async fn wait_for_request_logs(pool: &SqlitePool, expected: usize) -> Vec<storage::RequestLogRow> {
    for _ in 0..50 {
        let logs = storage::list_request_logs(pool, None).await.unwrap();
        if logs.len() >= expected {
            return logs;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    storage::list_request_logs(pool, None).await.unwrap()
}

async fn wait_for_limit_usage_event(pool: &SqlitePool, total_tokens: i64) -> (i64, Option<String>) {
    for _ in 0..50 {
        if let Some(row) = sqlx::query_as::<_, (i64, Option<String>)>(
            "SELECT total_tokens, finalized_at
             FROM limit_usage_events
             WHERE total_tokens = ? AND finalized_at IS NOT NULL
             LIMIT 1",
        )
        .bind(total_tokens)
        .fetch_optional(pool)
        .await
        .unwrap()
        {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    sqlx::query_as(
        "SELECT total_tokens, finalized_at
         FROM limit_usage_events
         ORDER BY created_at DESC
         LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn limit_inflight_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM limit_inflight_requests")
        .fetch_one(pool)
        .await
        .unwrap()
}

fn header_string(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

struct CancelAwareSse {
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

fn proxy_request(uri: impl AsRef<str>, key: &str) -> Request<Body> {
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

fn limit_set(value: i64) -> storage::LimitPatchValue {
    storage::LimitPatchValue::Set(value)
}

fn timeout_default() -> storage::TimeoutPatchValue {
    storage::TimeoutPatchValue::Default
}

fn timeout_missing() -> storage::TimeoutPatchValue {
    storage::TimeoutPatchValue::Missing
}

fn timeout_explicit(value: i64) -> storage::TimeoutPatchValue {
    storage::TimeoutPatchValue::Explicit(value)
}

fn empty_request(method: &'static str, uri: impl AsRef<str>, key: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap()
}
