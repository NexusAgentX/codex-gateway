mod support;

use support::*;

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
async fn login_issues_scoped_panel_token_without_creating_api_key_session() {
    let (app, _api_key, pool) = test_app_with_pool(None).await;
    let key_count_before = storage::list_api_keys(&pool).await.unwrap().len();

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

    let key_count_after = storage::list_api_keys(&pool).await.unwrap().len();
    assert_eq!(key_count_after, key_count_before);

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
async fn panel_token_expiry_uses_injected_clock() {
    let now = DateTime::parse_from_rfc3339("2026-07-12T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let clock = TestClock::new(now);
    let (app, _, _) = TestAppBuilder::new().clock(clock.clone()).build().await;

    let login = app
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
    let token = assert_status_json(login, StatusCode::OK).await["token"]
        .as_str()
        .unwrap()
        .to_string();

    assert_eq!(
        app.clone()
            .oneshot(empty_request("GET", "/api/me", &token))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    clock.advance(ChronoDuration::hours(13));
    let expired = app
        .oneshot(empty_request("GET", "/api/me", &token))
        .await
        .unwrap();
    let body = assert_status_json(expired, StatusCode::FORBIDDEN).await;
    assert_eq!(body["error"]["code"], "expired_api_key");
}

#[tokio::test]
async fn api_key_last_used_at_uses_injected_clock() {
    let now = DateTime::parse_from_rfc3339("2042-03-04T05:06:07Z")
        .unwrap()
        .with_timezone(&Utc);
    let clock = TestClock::new(now);
    let (app, api_key, pool) = TestAppBuilder::new().clock(clock.clone()).build().await;
    let key_id = storage::list_api_keys(&pool).await.unwrap().remove(0).id;

    let response = app
        .clone()
        .oneshot(empty_request("GET", "/v1/models", &api_key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        storage::get_api_key(&pool, &key_id)
            .await
            .unwrap()
            .unwrap()
            .last_used_at
            .as_deref(),
        Some("2042-03-04T05:06:07.000Z")
    );

    clock.advance(ChronoDuration::hours(2));
    let login = app
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
    let panel_token = assert_status_json(login, StatusCode::OK).await["token"]
        .as_str()
        .unwrap()
        .to_string();
    let response = app
        .clone()
        .oneshot(empty_request("GET", "/api/me", &panel_token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        storage::get_api_key(&pool, &key_id)
            .await
            .unwrap()
            .unwrap()
            .last_used_at
            .as_deref(),
        Some("2042-03-04T05:06:07.000Z")
    );

    let response = app
        .oneshot(empty_request("GET", "/api/me", &api_key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        storage::get_api_key(&pool, &key_id)
            .await
            .unwrap()
            .unwrap()
            .last_used_at
            .as_deref(),
        Some("2042-03-04T07:06:07.000Z")
    );
}

#[tokio::test]
async fn invalid_api_key_expiration_fails_closed_without_leaking_storage_value() {
    let (app, api_key, pool) = test_app_with_pool(None).await;
    let key_id = storage::list_api_keys(&pool).await.unwrap().remove(0).id;
    let corrupt_expiration = "not-a-timestamp-secret";
    sqlx::query("UPDATE api_keys SET expires_at = ? WHERE id = ?")
        .bind(corrupt_expiration)
        .bind(key_id)
        .execute(&pool)
        .await
        .unwrap();

    let response = app
        .oneshot(empty_request("GET", "/v1/models", &api_key))
        .await
        .unwrap();
    let body = assert_status_json(response, StatusCode::FORBIDDEN).await;
    assert_eq!(body["error"]["code"], "expired_api_key");
    assert!(!body.to_string().contains(corrupt_expiration));
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

    let upstream_id = storage::list_upstreams(&pool).await.unwrap().remove(0).id;
    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/upstreams/{upstream_id}"),
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
            format!("/api/admin/upstreams/{upstream_id}/disable"),
            &admin_key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_json(response).await["enabled"], false);

    let model_id = storage::list_models(&pool).await.unwrap().remove(0).id;
    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/models/{model_id}"),
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
    assert_eq!(updated_model["visible_to_users"], false);

    let mapping_id = storage::list_upstream_models_for_model(&pool, &model_id)
        .await
        .unwrap()
        .remove(0)
        .id;
    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            format!("/api/admin/model-mappings/{mapping_id}"),
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
            format!("/api/admin/model-mappings/{mapping_id}/disable"),
            &admin_key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_json(response).await["enabled"], false);

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
    let admin_id = storage::list_users(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|user| user.role == "admin")
        .unwrap()
        .id;
    let admin_key_id = storage::list_api_keys_for_user(&pool, &admin_id)
        .await
        .unwrap()
        .remove(0)
        .id;

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
            format!("/api/api-keys/{admin_key_id}/revoke"),
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
    let model_id = storage::list_models(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|model| model.public_name == "codex-mini")
        .unwrap()
        .id;
    let upstream_id = storage::list_upstreams(&pool).await.unwrap().remove(0).id;
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
    let model_id = storage::list_models(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|model| model.public_name == "codex-mini")
        .unwrap()
        .id;
    let upstream_id = storage::list_upstreams(&pool).await.unwrap().remove(0).id;
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
