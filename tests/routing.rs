mod support;

use support::*;

#[tokio::test]
async fn unhealthy_candidate_is_skipped_without_attempt_log() {
    let (unhealthy_url, unhealthy_calls) = spawn_counting_upstream().await;
    let (healthy_url, healthy_calls) = spawn_counting_upstream().await;
    let (app, key, pool) = app_with_two_upstreams(&unhealthy_url, &healthy_url).await;
    let upstreams = storage::list_upstreams(&pool).await.unwrap();
    let first_id = upstreams
        .iter()
        .find(|item| item.name == "first")
        .unwrap()
        .id
        .clone();
    let second_id = upstreams
        .iter()
        .find(|item| item.name == "second")
        .unwrap()
        .id
        .clone();
    storage::record_upstream_health(&pool, &first_id, "down", Some("upstream_timeout"))
        .await
        .unwrap();

    let response = app
        .oneshot(proxy_request("/responses", &key))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(unhealthy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(healthy_calls.load(Ordering::SeqCst), 1);
    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].upstream_id.as_deref(), Some(second_id.as_str()));
    assert_limit_settlement(&pool, 1, 3).await;
}

#[tokio::test]
async fn runtime_default_timeout_live_reloads_for_existing_defaulted_upstream() {
    let (slow, _calls, entered, release) = spawn_blocking_counting_upstream().await;
    let (app, key, pool) = app_with_single_upstream_timeout(&slow, None).await;

    let first_request = tokio::spawn({
        let app = app.clone();
        let key = key.clone();
        async move {
            app.oneshot(proxy_request("/responses", &key))
                .await
                .unwrap()
        }
    });
    entered.notified().await;
    release.notify_one();
    let response = first_request.await.unwrap();
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
    assert_eq!(created["timeout_ms"], default_request_timeout_ms());
    assert_eq!(created["timeout_ms_is_explicit"], false);

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
    assert_eq!(listed["timeout_ms_is_explicit"], false);

    let stored = storage::list_upstreams(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.name == "api-default-timeout")
        .unwrap();
    assert_eq!(stored.timeout_ms_is_explicit, 0);
}

#[tokio::test]
async fn admin_patch_preserves_default_timeout_mode_when_omitted() {
    let slow = spawn_stalling_upstream().await;
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
    assert_eq!(updated["timeout_ms_is_explicit"], false);

    let stored = storage::get_upstream(&pool, &upstream.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.timeout_ms_is_explicit, 0);
}

#[tokio::test]
async fn explicit_upstream_timeout_ignores_runtime_default_changes() {
    let (slow, _calls, entered, release) = spawn_blocking_counting_upstream().await;
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

    let request = tokio::spawn(async move {
        app.oneshot(proxy_request("/responses", &key))
            .await
            .unwrap()
    });
    entered.notified().await;
    release.notify_one();
    let response = request.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_json(response).await["model_seen"], "counted");
}

#[tokio::test]
async fn admin_patch_can_reset_explicit_timeout_to_runtime_default() {
    let slow = spawn_stalling_upstream().await;
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
    assert_eq!(updated["timeout_ms_is_explicit"], false);

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
async fn admin_health_check_updates_upstream_status() {
    let upstream = spawn_mock_upstream().await;
    let (app, key, pool) = test_app_with_pool(Some(&upstream)).await;
    let upstream_id = storage::list_upstreams(&pool).await.unwrap().remove(0).id;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/admin/upstreams/{upstream_id}/health"))
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let health = storage::get_upstream(&pool, &upstream_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(health.last_health_status, "healthy");
}

#[tokio::test]
async fn defaulted_health_checks_use_live_runtime_timeout() {
    let delayed_health = spawn_stalling_health_upstream().await;
    let (app, key, pool) = app_with_single_upstream_timeout(&delayed_health, None).await;
    let upstream_id = storage::list_upstreams(&pool).await.unwrap().remove(0).id;

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
            format!("/api/admin/upstreams/{upstream_id}/health"),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_json(response).await["health"], "down");
    let health = storage::get_upstream(&pool, &upstream_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(health.last_health_status, "down");
    assert!(health.recent_error_samples.contains("upstream_timeout"));

    storage::record_upstream_health(&pool, &upstream_id, "healthy", None)
        .await
        .unwrap();
    let config = test_config();
    let checked = codex_gateway::upstream::check_all_enabled_upstreams(&AppState {
        config: Arc::new(config),
        db: pool.clone(),
        http: reqwest::Client::new(),
        finalizations: FinalizationTracker::default(),
        clock: codex_gateway::clock::system_clock(),
    })
    .await
    .unwrap();
    assert_eq!(checked, 1);
    let health = storage::get_upstream(&pool, &upstream_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(health.last_health_status, "down");
    assert!(health.recent_error_samples.contains("upstream_timeout"));
}

#[tokio::test]
async fn explicit_health_timeout_ignores_runtime_default_changes() {
    let (delayed_health, entered, release) = spawn_blocking_health_upstream().await;
    let (app, key, pool) = app_with_single_upstream_timeout(&delayed_health, Some(500)).await;
    let upstream_id = storage::list_upstreams(&pool).await.unwrap().remove(0).id;

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

    let health_path = format!("/api/admin/upstreams/{upstream_id}/health");
    let health_request = tokio::spawn({
        let app = app.clone();
        let key = key.clone();
        async move {
            app.oneshot(empty_request("POST", health_path, &key))
                .await
                .unwrap()
        }
    });
    entered.notified().await;
    release.notify_one();
    let response = health_request.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_json(response).await["health"], "healthy");

    storage::record_upstream_health(&pool, &upstream_id, "down", Some("reset"))
        .await
        .unwrap();
    let state = AppState {
        config: Arc::new(test_config()),
        db: pool.clone(),
        http: reqwest::Client::new(),
        finalizations: FinalizationTracker::default(),
        clock: codex_gateway::clock::system_clock(),
    };
    let check = tokio::spawn(async move {
        codex_gateway::upstream::check_all_enabled_upstreams(&state)
            .await
            .unwrap()
    });
    entered.notified().await;
    release.notify_one();
    let checked = check.await.unwrap();
    assert_eq!(checked, 1);
    let health = storage::get_upstream(&pool, &upstream_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(health.last_health_status, "healthy");
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
        finalizations: FinalizationTracker::default(),
        clock: codex_gateway::clock::system_clock(),
    };
    assert!(codex_gateway::upstream::spawn_health_worker(disabled_state).is_none());

    config.health_checks_enabled = true;
    config.health_check_interval_ms = 100;
    let enabled_state = AppState {
        config: std::sync::Arc::new(config),
        db: pool,
        http: reqwest::Client::new(),
        finalizations: FinalizationTracker::default(),
        clock: codex_gateway::clock::system_clock(),
    };
    let handle = codex_gateway::upstream::spawn_health_worker(enabled_state);
    assert!(handle.is_some());
    handle.unwrap().abort();
}
