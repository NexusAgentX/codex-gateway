mod support;

use support::*;

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
async fn runtime_settings_http_contract_covers_every_descriptor() {
    let (app, key) = test_app(None).await;

    for descriptor in RUNTIME_CONFIG_DESCRIPTORS {
        let valid_value = match descriptor.key {
            RuntimeConfigKey::RouteStrategy => json!("weighted"),
            RuntimeConfigKey::DefaultRequestTimeoutMs => json!(321),
            RuntimeConfigKey::MaxRequestBodyBytes => json!(654),
            RuntimeConfigKey::RequestLogRetentionDays => json!(0),
            RuntimeConfigKey::DailyUsageRetentionDays => json!(42),
            RuntimeConfigKey::ExposeDebugHeaders => json!(true),
        };
        let invalid_value = match descriptor.key {
            RuntimeConfigKey::RouteStrategy => json!("random"),
            RuntimeConfigKey::DefaultRequestTimeoutMs | RuntimeConfigKey::MaxRequestBodyBytes => {
                json!(0)
            }
            RuntimeConfigKey::RequestLogRetentionDays
            | RuntimeConfigKey::DailyUsageRetentionDays => json!(-1),
            RuntimeConfigKey::ExposeDebugHeaders => json!("true"),
        };

        let response = app
            .clone()
            .oneshot(json_request(
                "PATCH",
                "/api/admin/settings",
                &key,
                json!({ descriptor.field_name: valid_value }),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{} set",
            descriptor.field_name
        );
        let body = to_json(response).await;
        let field = body["runtime"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["key"] == descriptor.field_name)
            .unwrap();
        assert_eq!(field["source"], "database");
        assert_eq!(field["value"], valid_value);
        assert_eq!(field["database_value"], valid_value);
        assert_eq!(field["default_value"], descriptor.default_value.to_json());
        assert_eq!(field["label"], descriptor.display.label);
        assert_eq!(field["value_type"], descriptor.value_type.as_str());
        assert_eq!(field["validation"], descriptor.validation_json());
        assert_eq!(
            field["environment_variable"],
            descriptor.environment_variable
        );
        assert_eq!(field["unit"], json!(descriptor.display.unit));
        assert_eq!(field["editable"], descriptor.editable);
        assert_eq!(field["live_reload"], descriptor.live_reload);
        assert_eq!(field["requires_restart"], descriptor.requires_restart);

        let response = app
            .clone()
            .oneshot(json_request(
                "PATCH",
                "/api/admin/settings",
                &key,
                json!({ descriptor.field_name: null }),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{} clear",
            descriptor.field_name
        );
        let body = to_json(response).await;
        let field = body["runtime"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["key"] == descriptor.field_name)
            .unwrap();
        assert_eq!(field["source"], "default");
        assert_eq!(field["database_value"], Value::Null);
        assert_eq!(field["value"], descriptor.default_value.to_json());

        let response = app
            .clone()
            .oneshot(json_request(
                "PATCH",
                "/api/admin/settings",
                &key,
                json!({ descriptor.field_name: invalid_value }),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{} invalid value",
            descriptor.field_name
        );
        assert_eq!(to_json(response).await["error"]["code"], "invalid_request");
    }
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
        finalizations: FinalizationTracker::default(),
        clock: codex_gateway::clock::system_clock(),
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
    let now = DateTime::parse_from_rfc3339("2026-07-12T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let clock = TestClock::new(now);
    let (app, key, pool) = TestAppBuilder::new()
        .upstream(&upstream)
        .clock(clock.clone())
        .build()
        .await;

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
            &clock
                .now()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
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
