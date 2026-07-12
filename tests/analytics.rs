mod support;

use support::*;

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

    let health = storage::list_upstreams(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|upstream| upstream.name == "mock")
        .unwrap();
    assert_eq!(health.last_health_status, "degraded");
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
    let api_key = storage::list_api_keys(&pool).await.unwrap().remove(0);
    let user_id = api_key.user_id;
    let api_key_id = api_key.id;
    let model_id = storage::list_models(&pool).await.unwrap().remove(0).id;
    let upstream_id = storage::list_upstreams(&pool).await.unwrap().remove(0).id;

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
