mod support;

use support::*;

#[tokio::test]
async fn request_and_token_quotas_enforce_and_reset_by_window() {
    let upstream = spawn_mock_upstream().await;
    let clock = TestClock::new(Utc::now());
    let (app, key, pool) = TestAppBuilder::new()
        .upstream(&upstream)
        .clock(clock.clone())
        .build()
        .await;

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

    clock.advance(ChronoDuration::seconds(2));
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

    clock.advance(ChronoDuration::seconds(2));
    let response = app
        .clone()
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn api_key_usage_endpoints_share_injected_limit_clock() {
    let upstream = spawn_mock_upstream().await;
    let now = DateTime::parse_from_rfc3339("2026-07-12T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let clock = TestClock::new(now);
    let (app, key, pool) = TestAppBuilder::new()
        .upstream(&upstream)
        .clock(clock.clone())
        .build()
        .await;
    let (_, key_id) = seeded_user_and_key_ids(&pool).await;
    storage::upsert_limit_policy(
        &pool,
        "system",
        "",
        &storage::LimitPolicyPatch {
            request_quota: limit_set(10),
            request_window_seconds: Some(1),
            token_quota: limit_set(10),
            token_window_seconds: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let proxied = app
        .clone()
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();
    assert_eq!(proxied.status(), StatusCode::OK);

    assert_usage_endpoint_windows(&app, &key, &key_id, 1, 3, 1).await;
    clock.advance(ChronoDuration::seconds(2));
    assert_usage_endpoint_windows(&app, &key, &key_id, 0, 0, 1).await;
}

async fn assert_usage_endpoint_windows(
    app: &Router,
    key: &str,
    key_id: &str,
    expected_requests: i64,
    expected_tokens: i64,
    expected_daily_requests: i64,
) {
    let limits = app
        .clone()
        .oneshot(empty_request("GET", "/api/limits", key))
        .await
        .unwrap();
    let limits = assert_status_json(limits, StatusCode::OK).await;
    assert_eq!(
        limits["current_key"]["request_quota"]["used"],
        expected_requests
    );
    assert_eq!(
        limits["current_key"]["token_budget"]["used"],
        expected_tokens
    );

    for path in [
        format!("/api/api-keys/{key_id}/usage"),
        format!("/api/admin/api-keys/{key_id}/usage"),
    ] {
        let response = app
            .clone()
            .oneshot(empty_request("GET", path, key))
            .await
            .unwrap();
        let body = assert_status_json(response, StatusCode::OK).await;
        assert_eq!(body["limits"]["request_quota"]["used"], expected_requests);
        assert_eq!(body["limits"]["token_budget"]["used"], expected_tokens);
        assert_eq!(
            body["usage"]["totals"]["request_count"],
            expected_daily_requests
        );
    }
}

#[tokio::test]
async fn limit_policy_patch_preserves_omitted_fields_and_null_clears_limits() {
    let (app, admin_key, pool) = test_app_with_pool(None).await;
    let (user_id, key_id) = seeded_user_and_key_ids(&pool).await;

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
    let (upstream, upstream_calls) = spawn_counting_upstream().await;
    let (app, admin_key, pool) = test_app_with_pool(Some(&upstream)).await;
    let (user_id, key_id) = seeded_user_and_key_ids(&pool).await;

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
    let (upstream, _upstream_calls) = spawn_counting_upstream().await;
    let clock = TestClock::new(Utc::now());
    let (app, key, pool) = TestAppBuilder::new()
        .upstream(&upstream)
        .clock(clock.clone())
        .build()
        .await;
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

    assert_no_api_key_limit_policies(&pool).await;

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

    assert_no_api_key_limit_policies(&pool).await;

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

    clock.advance(ChronoDuration::seconds(2));
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
    let (user_id, key_id) = seeded_user_and_key_ids(&pool).await;

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
    let (upstream, upstream_calls) = spawn_counting_upstream().await;
    let (app, key_one, pool) = test_app_with_pool(Some(&upstream)).await;
    let config = test_config();
    let (user_id, _) = seeded_user_and_key_ids(&pool).await;
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

    assert_no_api_key_limit_policies(&pool).await;

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
    let (upstream, upstream_calls) = spawn_counting_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    let (user_id, key_id) = seeded_user_and_key_ids(&pool).await;

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
    let (upstream, upstream_calls) = spawn_counting_upstream().await;
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
    assert_limit_settlement(&pool, 1, 3).await;
}

#[tokio::test]
async fn pre_admission_rejection_creates_no_attempt_log_or_settlement() {
    let (upstream, upstream_calls) = spawn_counting_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/responses")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"input": []}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(to_json(response).await["error"]["code"], "model_not_found");
    assert_eq!(upstream_calls.load(Ordering::SeqCst), 0);
    assert!(
        storage::list_request_logs(&pool, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert_limit_settlement(&pool, 0, 0).await;
}

#[tokio::test]
async fn quota_limited_and_disabled_principals_cannot_bypass_routes_or_panel_tokens() {
    let (upstream, upstream_calls) = spawn_counting_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    let (user_id, key_id) = seeded_user_and_key_ids(&pool).await;

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
