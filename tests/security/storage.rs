use crate::support::*;

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
    assert_eq!(
        codex_gateway::secrets::decrypt_upstream_api_key(&config.app_secret, stored.1, &stored.0)
            .unwrap(),
        "sk-version-one"
    );

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
    assert_eq!(
        codex_gateway::secrets::decrypt_upstream_api_key(&config.app_secret, stored.1, &stored.0)
            .unwrap(),
        "sk-version-two"
    );
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

    assert_eq!(
        codex_gateway::secrets::decrypt_upstream_api_key(&config.app_secret, stored.1, &stored.0)
            .unwrap(),
        "sk-legacy-plaintext"
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
async fn admin_audit_failure_rolls_back_business_mutation() {
    let (app, admin_key, pool) = test_app_with_pool(None).await;
    sqlx::query(
        "CREATE TRIGGER fail_admin_audit
         BEFORE INSERT ON admin_audit_logs
         BEGIN
             SELECT RAISE(FAIL, 'injected admin audit failure');
         END",
    )
    .execute(&pool)
    .await
    .unwrap();

    let response = app
        .oneshot(json_request(
            "POST",
            "/api/admin/users",
            &admin_key,
            json!({
                "email": "audit-rollback@example.com",
                "password": "rollback-password",
                "role": "user"
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        to_json(response).await,
        json!({
            "error": {
                "message": "gateway storage error",
                "type": "gateway_error",
                "code": "gateway_internal_error",
                "details": null
            }
        })
    );
    let user_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE email = 'audit-rollback@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let audit_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_audit_logs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(user_count, 0);
    assert_eq!(audit_count, 0);
}

#[tokio::test]
async fn health_audit_failure_returns_compatible_error_and_rolls_back_health_state() {
    let upstream_url = spawn_mock_upstream().await;
    let (app, admin_key, pool) = test_app_with_pool(Some(&upstream_url)).await;
    let upstream_id: String = sqlx::query_scalar("SELECT id FROM upstreams LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let before: (String, Option<String>, Option<String>, String, String) = sqlx::query_as(
        "SELECT last_health_status, last_health_checked_at, health_status_changed_at,
                recent_error_samples, updated_at
         FROM upstreams WHERE id = ?",
    )
    .bind(&upstream_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_health_audit
         BEFORE INSERT ON admin_audit_logs
         WHEN NEW.action = 'check_upstream_health'
         BEGIN
             SELECT RAISE(FAIL, 'injected health audit failure');
         END",
    )
    .execute(&pool)
    .await
    .unwrap();

    let response = app
        .oneshot(empty_request(
            "POST",
            format!("/api/admin/upstreams/{upstream_id}/health"),
            &admin_key,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        to_json(response).await,
        json!({
            "error": {
                "message": "upstream health check failed",
                "type": "gateway_error",
                "code": "upstream_unavailable",
                "details": null
            }
        })
    );
    let after: (String, Option<String>, Option<String>, String, String) = sqlx::query_as(
        "SELECT last_health_status, last_health_checked_at, health_status_changed_at,
                recent_error_samples, updated_at
         FROM upstreams WHERE id = ?",
    )
    .bind(&upstream_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'check_upstream_health'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(after, before);
    assert_eq!(audit_count, 0);
}

#[tokio::test]
async fn concurrent_read_before_write_admin_mutations_all_commit_with_one_audit_each() {
    const REQUEST_COUNT: usize = 100;

    let temp_dir = tempfile::tempdir().unwrap();
    let database_url = format!("sqlite://{}", temp_dir.path().join("gateway.db").display());
    let pool = storage::connect_and_migrate(&database_url).await.unwrap();
    let mut held_connections = Vec::new();
    for _ in 0..5 {
        held_connections.push(pool.acquire().await.unwrap());
    }
    assert_eq!(pool.size(), 5);
    drop(held_connections);

    let mut config = test_config();
    config.database_url = database_url;
    let admin_id = storage::ensure_user(
        &pool,
        &CreateUser {
            email: "concurrency-admin@example.com".into(),
            password: "password".into(),
            role: "admin".into(),
            display_name: Some("Concurrency Admin".into()),
        },
    )
    .await
    .unwrap();
    let target_id = storage::ensure_user(
        &pool,
        &CreateUser {
            email: "concurrency-target@example.com".into(),
            password: "password".into(),
            role: "user".into(),
            display_name: Some("Before".into()),
        },
    )
    .await
    .unwrap();
    let admin_token = auth::generate_panel_token(&config.app_secret, &admin_id);
    let app = build_app(AppState {
        config: Arc::new(config),
        db: pool.clone(),
        http: reqwest::Client::new(),
        finalizations: FinalizationTracker::default(),
        clock: codex_gateway::clock::system_clock(),
    });
    let barrier = Arc::new(tokio::sync::Barrier::new(REQUEST_COUNT + 1));
    let mut requests = Vec::with_capacity(REQUEST_COUNT);
    for index in 0..REQUEST_COUNT {
        let app = app.clone();
        let barrier = barrier.clone();
        let admin_token = admin_token.clone();
        let target_id = target_id.clone();
        requests.push(tokio::spawn(async move {
            barrier.wait().await;
            app.oneshot(json_request(
                "PATCH",
                format!("/api/admin/users/{target_id}"),
                &admin_token,
                json!({ "display_name": format!("Concurrent {index}") }),
            ))
            .await
            .unwrap()
        }));
    }
    barrier.wait().await;

    for request in requests {
        let response = request.await.unwrap();
        if response.status() != StatusCode::OK {
            let status = response.status();
            let body = to_json(response).await;
            panic!("concurrent admin mutation returned {status}: {body}");
        }
    }

    let updated = storage::get_user(&pool, &target_id).await.unwrap().unwrap();
    let final_index = updated
        .display_name
        .as_deref()
        .and_then(|name| name.strip_prefix("Concurrent "))
        .and_then(|index| index.parse::<usize>().ok());
    assert!(final_index.is_some_and(|index| index < REQUEST_COUNT));
    assert_eq!(updated.role, "user");
    assert_eq!(updated.status, "active");

    let audit_counts: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COUNT(DISTINCT id)
         FROM admin_audit_logs
         WHERE action = 'update_user' AND resource_type = 'user' AND resource_id = ?",
    )
    .bind(&target_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit_counts, (REQUEST_COUNT as i64, REQUEST_COUNT as i64));
    let metadata: Vec<String> = sqlx::query_scalar(
        "SELECT metadata_json FROM admin_audit_logs
         WHERE action = 'update_user' AND resource_id = ?",
    )
    .bind(&target_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(metadata.iter().all(|metadata| {
        serde_json::from_str::<Value>(metadata).is_ok_and(|metadata| {
            metadata
                == json!({
                    "role_changed": false,
                    "status_changed": false,
                    "display_name_changed": true
                })
        })
    }));
}

#[tokio::test]
async fn cancelled_managed_immediate_transaction_rolls_back_and_reuses_single_connection() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}",
        temp_dir.path().join("cancelled-audit.db").display()
    );
    let options = database_url
        .parse::<SqliteConnectOptions>()
        .unwrap()
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    assert_eq!(pool.size(), 1);
    let actor_id = storage::ensure_user(
        &pool,
        &CreateUser {
            email: "cancellation-admin@example.com".into(),
            password: "password".into(),
            role: "admin".into(),
            display_name: Some("Cancellation Admin".into()),
        },
    )
    .await
    .unwrap();
    sqlx::query("CREATE TEMP TABLE cancelled_audit_connection_marker (id INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    let cancelled_id = "cancelled-business-row".to_string();
    let cancelled_email = "cancelled-mutation@example.com".to_string();
    let cancelled_actor_id = actor_id.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let cancellation_gate = Arc::new(tokio::sync::Notify::new());
    let cancelled_pool = pool.clone();
    let cancelled_task = tokio::spawn({
        let cancellation_gate = cancellation_gate.clone();
        async move {
            storage::with_admin_audit::<_, sqlx::Error, _>(&cancelled_pool, move |conn| {
                Box::pin(async move {
                    let now = storage::now_string();
                    sqlx::query(
                        "INSERT INTO users
                             (id, email, password_hash, role, status, created_at, updated_at)
                             VALUES (?, ?, 'unused', 'user', 'active', ?, ?)",
                    )
                    .bind(&cancelled_id)
                    .bind(&cancelled_email)
                    .bind(&now)
                    .bind(&now)
                    .execute(&mut *conn)
                    .await?;
                    started_tx.send(()).unwrap();
                    cancellation_gate.notified().await;
                    Ok((
                        (),
                        storage::AdminAuditInsert {
                            actor_user_id: cancelled_actor_id,
                            actor_email: "cancellation-admin@example.com".into(),
                            action: "cancelled_mutation",
                            resource_type: "user",
                            resource_id: Some(cancelled_id),
                            status: "success",
                            metadata_json: Some("{}".into()),
                        },
                    ))
                })
            })
            .await
        }
    });

    started_rx.await.unwrap();
    cancelled_task.abort();
    assert!(cancelled_task.await.unwrap_err().is_cancelled());
    let connection_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_temp_master
         WHERE type = 'table' AND name = 'cancelled_audit_connection_marker'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(connection_marker_count, 0);

    let recovered_id = "recovered-business-row".to_string();
    let recovered_id_for_query = recovered_id.clone();
    let recovered_actor_id = actor_id.clone();
    storage::with_admin_audit::<_, sqlx::Error, _>(&pool, move |conn| {
        Box::pin(async move {
            let now = storage::now_string();
            sqlx::query(
                "INSERT INTO users
                 (id, email, password_hash, role, status, created_at, updated_at)
                 VALUES (?, 'recovered-mutation@example.com', 'unused', 'user', 'active', ?, ?)",
            )
            .bind(&recovered_id)
            .bind(&now)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            Ok((
                (),
                storage::AdminAuditInsert {
                    actor_user_id: recovered_actor_id,
                    actor_email: "cancellation-admin@example.com".into(),
                    action: "recovered_mutation",
                    resource_type: "user",
                    resource_id: Some(recovered_id),
                    status: "success",
                    metadata_json: Some("{}".into()),
                },
            ))
        })
    })
    .await
    .unwrap();

    let cancelled_business_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = 'cancelled-business-row'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let recovered_business_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = ?")
            .bind(&recovered_id_for_query)
            .fetch_one(&pool)
            .await
            .unwrap();
    let cancelled_audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'cancelled_mutation'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let recovered_audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_audit_logs
         WHERE action = 'recovered_mutation' AND resource_id = ?",
    )
    .bind(&recovered_id_for_query)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(cancelled_business_count, 0);
    assert_eq!(cancelled_audit_count, 0);
    assert_eq!(recovered_business_count, 1);
    assert_eq!(recovered_audit_count, 1);
}

#[tokio::test]
async fn duplicate_mapping_rolls_back_model_and_all_mappings() {
    let (app, admin_key, pool) = test_app_with_pool(None).await;
    let upstream_id: String = sqlx::query_scalar("SELECT id FROM upstreams LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    let mapping = json!({
        "upstream_id": upstream_id,
        "upstream_model_name": "duplicate-upstream-model",
        "enabled": true,
        "priority": 1,
        "weight": 1
    });
    let response = app
        .oneshot(json_request(
            "POST",
            "/api/admin/models",
            &admin_key,
            json!({
                "public_name": "duplicate-mapping-model",
                "description": "must roll back",
                "enabled": true,
                "visible_to_users": true,
                "upstream_mappings": [mapping.clone(), mapping]
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        to_json(response).await["error"]["code"],
        "gateway_internal_error"
    );
    let model_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models WHERE public_name = ?")
        .bind("duplicate-mapping-model")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mapping_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_models WHERE upstream_model_name = ?")
            .bind("duplicate-upstream-model")
            .fetch_one(&pool)
            .await
            .unwrap();
    let audit_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'create_model'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(model_count, 0);
    assert_eq!(mapping_count, 0);
    assert_eq!(audit_count, 0);
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
async fn limit_finalization_uses_injected_clock_after_success() {
    let initial = DateTime::parse_from_rfc3339("2044-01-02T03:04:05Z")
        .unwrap()
        .with_timezone(&Utc);
    let clock = TestClock::new(initial);
    let (upstream, _, entered, release) = spawn_blocking_counting_upstream().await;
    let (app, key, pool, _) = TestAppBuilder::new()
        .upstream(upstream)
        .clock(clock.clone())
        .build_tracked()
        .await;
    let request = tokio::spawn(async move {
        app.oneshot(proxy_request("/responses", &key))
            .await
            .unwrap()
    });

    entered.notified().await;
    clock.advance(ChronoDuration::minutes(7));
    release.notify_one();
    assert_eq!(request.await.unwrap().status(), StatusCode::OK);

    let (created_at, finalized_at): (String, Option<String>) =
        sqlx::query_as("SELECT created_at, finalized_at FROM limit_usage_events LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(created_at, "2044-01-02T03:04:05.000Z");
    assert_eq!(finalized_at.as_deref(), Some("2044-01-02T03:11:05.000Z"));
    assert!(created_at <= finalized_at.unwrap());

    let (started_at, finished_at): (String, Option<String>) =
        sqlx::query_as("SELECT started_at, finished_at FROM request_logs LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(started_at, "2044-01-02T03:04:05.000Z");
    assert_eq!(finished_at.as_deref(), Some("2044-01-02T03:11:05.000Z"));
}

#[tokio::test]
async fn limit_finalization_uses_injected_clock_after_cancellation() {
    let initial = DateTime::parse_from_rfc3339("2045-06-07T08:09:10Z")
        .unwrap()
        .with_timezone(&Utc);
    let clock = TestClock::new(initial);
    let (upstream, _, entered, release) = spawn_blocking_counting_upstream().await;
    let (app, key, pool, lifecycle) = TestAppBuilder::new()
        .upstream(upstream)
        .clock(clock.clone())
        .build_tracked()
        .await;
    let request = tokio::spawn(async move {
        app.oneshot(proxy_request("/responses", &key))
            .await
            .unwrap()
    });

    entered.notified().await;
    clock.advance(ChronoDuration::minutes(9));
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    release.notify_one();
    await_finalizations(&lifecycle, 2).await;

    let (created_at, finalized_at): (String, Option<String>) =
        sqlx::query_as("SELECT created_at, finalized_at FROM limit_usage_events LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(created_at, "2045-06-07T08:09:10.000Z");
    assert_eq!(finalized_at.as_deref(), Some("2045-06-07T08:18:10.000Z"));
    assert!(created_at <= finalized_at.unwrap());

    let (started_at, finished_at, status_code): (String, Option<String>, Option<i64>) =
        sqlx::query_as("SELECT started_at, finished_at, status_code FROM request_logs LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(started_at, "2045-06-07T08:09:10.000Z");
    assert_eq!(finished_at.as_deref(), Some("2045-06-07T08:18:10.000Z"));
    assert_eq!(status_code, Some(499));
}

mod admin_identity {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use codex_gateway::{
        AppState, FinalizationTracker, JSON_BODY_LIMIT_BYTES, auth, build_app,
        config::{Config, RouteStrategy},
        storage::{self, CreateApiKey, CreateUser},
    };
    use hmac::{Hmac, Mac};
    use serde_json::{Value, json};
    use sha2::Sha256;
    use sqlx::SqlitePool;
    use tower::ServiceExt;

    const JSON_CONTENT_TYPE: &str = "application/json";
    const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

    struct AdminFixture {
        app: Router,
        pool: SqlitePool,
        admin_id: String,
        admin_key_id: String,
        admin_key: String,
        admin_panel: String,
        user_key_id: String,
        user_key: String,
        user_panel: String,
        expired_admin_key_id: String,
        expired_admin_key: String,
        expired_admin_panel: String,
        disabled_admin_key_id: String,
        disabled_admin_key: String,
        disabled_user_key_id: String,
        disabled_user_key: String,
        disabled_user_panel: String,
    }

    impl AdminFixture {
        fn seeded_api_keys(&self) -> [(&'static str, &str); 5] {
            [
                ("admin", &self.admin_key_id),
                ("user", &self.user_key_id),
                ("expired", &self.expired_admin_key_id),
                ("disabled-key", &self.disabled_admin_key_id),
                ("disabled-user", &self.disabled_user_key_id),
            ]
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum CredentialKind {
        AdminPanel,
        AdminKey,
        Missing,
        InvalidPanel,
        InvalidKey,
        UserPanel,
        UserKey,
        ExpiredPanel,
        ExpiredKey,
        DisabledKey,
        DisabledUserPanel,
        DisabledUserKey,
    }

    impl CredentialKind {
        fn name(self) -> &'static str {
            match self {
                Self::AdminPanel => "admin-panel",
                Self::AdminKey => "admin-key",
                Self::Missing => "missing",
                Self::InvalidPanel => "invalid-panel",
                Self::InvalidKey => "invalid-key",
                Self::UserPanel => "user-panel",
                Self::UserKey => "user-key",
                Self::ExpiredPanel => "expired-panel",
                Self::ExpiredKey => "expired-key",
                Self::DisabledKey => "disabled-key",
                Self::DisabledUserPanel => "disabled-user-panel",
                Self::DisabledUserKey => "disabled-user-key",
            }
        }

        fn value(self, fixture: &AdminFixture) -> Option<&str> {
            match self {
                Self::AdminPanel => Some(&fixture.admin_panel),
                Self::AdminKey => Some(&fixture.admin_key),
                Self::Missing => None,
                Self::InvalidPanel => Some("cgw_panel_invalid.invalid"),
                Self::InvalidKey => Some("cgk_live_unknown_secret"),
                Self::UserPanel => Some(&fixture.user_panel),
                Self::UserKey => Some(&fixture.user_key),
                Self::ExpiredPanel => Some(&fixture.expired_admin_panel),
                Self::ExpiredKey => Some(&fixture.expired_admin_key),
                Self::DisabledKey => Some(&fixture.disabled_admin_key),
                Self::DisabledUserPanel => Some(&fixture.disabled_user_panel),
                Self::DisabledUserKey => Some(&fixture.disabled_user_key),
            }
        }

        fn is_admin(self) -> bool {
            matches!(self, Self::AdminPanel | Self::AdminKey)
        }

        fn active_api_key_id(self, fixture: &AdminFixture) -> Option<&str> {
            match self {
                Self::AdminKey => Some(&fixture.admin_key_id),
                Self::UserKey => Some(&fixture.user_key_id),
                _ => None,
            }
        }

        fn is_api_key_case(self) -> bool {
            matches!(
                self,
                Self::AdminKey
                    | Self::InvalidKey
                    | Self::UserKey
                    | Self::ExpiredKey
                    | Self::DisabledKey
                    | Self::DisabledUserKey
            )
        }

        fn auth_error(self) -> ExpectedResponse {
            match self {
                Self::Missing | Self::InvalidPanel | Self::InvalidKey => gateway_error(
                    StatusCode::UNAUTHORIZED,
                    "invalid_api_key",
                    "invalid API key",
                ),
                Self::UserPanel | Self::UserKey => {
                    gateway_error(StatusCode::FORBIDDEN, "forbidden", "admin role required")
                }
                Self::ExpiredPanel | Self::ExpiredKey => {
                    gateway_error(StatusCode::FORBIDDEN, "expired_api_key", "expired API key")
                }
                Self::DisabledKey => gateway_error(
                    StatusCode::FORBIDDEN,
                    "disabled_api_key",
                    "disabled API key",
                ),
                Self::DisabledUserPanel | Self::DisabledUserKey => {
                    gateway_error(StatusCode::FORBIDDEN, "disabled_user", "disabled user")
                }
                Self::AdminPanel | Self::AdminKey => panic!("administrator has no auth error"),
            }
        }
    }

    const CREDENTIALS: [CredentialKind; 12] = [
        CredentialKind::AdminPanel,
        CredentialKind::AdminKey,
        CredentialKind::Missing,
        CredentialKind::InvalidPanel,
        CredentialKind::InvalidKey,
        CredentialKind::UserPanel,
        CredentialKind::UserKey,
        CredentialKind::ExpiredPanel,
        CredentialKind::ExpiredKey,
        CredentialKind::DisabledKey,
        CredentialKind::DisabledUserPanel,
        CredentialKind::DisabledUserKey,
    ];

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum BodyKind {
        Valid,
        Malformed,
        WrongContentType,
        MissingContentType,
        Empty,
        Oversized,
    }

    impl BodyKind {
        fn name(self) -> &'static str {
            match self {
                Self::Valid => "valid",
                Self::Malformed => "malformed",
                Self::WrongContentType => "wrong-content-type",
                Self::MissingContentType => "missing-content-type",
                Self::Empty => "empty",
                Self::Oversized => "oversized",
            }
        }

        fn request_parts(self, valid_json: String) -> (Option<&'static str>, Body) {
            match self {
                Self::Valid => (Some(JSON_CONTENT_TYPE), Body::from(valid_json)),
                Self::Malformed => (Some(JSON_CONTENT_TYPE), Body::from("{")),
                Self::WrongContentType => (Some("text/plain"), Body::from(valid_json)),
                Self::MissingContentType => (None, Body::from(valid_json)),
                Self::Empty => (Some(JSON_CONTENT_TYPE), Body::empty()),
                Self::Oversized => (
                    Some(JSON_CONTENT_TYPE),
                    Body::from(format!(
                        r#"{{"padding":"{}"}}"#,
                        "x".repeat(JSON_BODY_LIMIT_BYTES)
                    )),
                ),
            }
        }

        fn standard_rejection(self) -> Option<ExpectedResponse> {
            match self {
                Self::Valid => None,
                Self::Malformed => Some(text_error(
                    StatusCode::BAD_REQUEST,
                    "Failed to parse the request body as JSON: EOF while parsing an object at line 1 column 1",
                )),
                Self::WrongContentType | Self::MissingContentType => Some(text_error(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "Expected request with `Content-Type: application/json`",
                )),
                Self::Empty => Some(text_error(
                    StatusCode::BAD_REQUEST,
                    "Failed to parse the request body as JSON: EOF while parsing a value at line 1 column 0",
                )),
                Self::Oversized => Some(text_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Failed to buffer the request body: length limit exceeded",
                )),
            }
        }

        fn settings_rejection(self) -> ExpectedResponse {
            match self {
                Self::Malformed | Self::Empty => gateway_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "request body must be JSON",
                ),
                Self::WrongContentType | Self::MissingContentType => gateway_error(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "invalid_request",
                    "request body must be JSON",
                ),
                Self::Oversized => gateway_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "request_body_too_large",
                    "request body exceeds configured maximum",
                ),
                Self::Valid => panic!("valid settings body is not a rejection case"),
            }
        }
    }

    const STANDARD_BODIES: [BodyKind; 6] = [
        BodyKind::Valid,
        BodyKind::Malformed,
        BodyKind::WrongContentType,
        BodyKind::MissingContentType,
        BodyKind::Empty,
        BodyKind::Oversized,
    ];

    const REJECTED_BODIES: [BodyKind; 5] = [
        BodyKind::Malformed,
        BodyKind::WrongContentType,
        BodyKind::MissingContentType,
        BodyKind::Empty,
        BodyKind::Oversized,
    ];

    #[derive(Debug)]
    struct ExpectedResponse {
        status: StatusCode,
        content_type: &'static str,
        body: ExpectedBody,
    }

    #[derive(Debug)]
    enum ExpectedBody {
        Json(Value),
        Text(&'static str),
    }

    struct CapturedResponse {
        status: StatusCode,
        content_type: String,
        body: Vec<u8>,
    }

    struct LastUsedSentinel {
        key_id: String,
        value: String,
    }

    #[tokio::test]
    async fn administrator_json_write_matrix_preserves_responses_and_side_effects() {
        let fixture = admin_fixture().await;
        let initial_users = table_count(&fixture.pool, "users").await;
        let initial_audits = table_count(&fixture.pool, "admin_audit_logs").await;
        let mut successful_writes = 0_i64;

        for credential in CREDENTIALS {
            for body_kind in STANDARD_BODIES {
                let case = format!("{} / {}", credential.name(), body_kind.name());
                let email = format!(
                    "matrix-{}-{}@example.com",
                    credential.name(),
                    body_kind.name()
                );
                let valid_json = json!({
                    "email": email,
                    "password": "password",
                    "role": "user"
                })
                .to_string();
                let (content_type, body) = body_kind.request_parts(valid_json);
                let users_before = table_count(&fixture.pool, "users").await;
                let audits_before = table_count(&fixture.pool, "admin_audit_logs").await;
                let sentinels = reset_last_used_at_for_credential(
                    &fixture,
                    credential,
                    &format!("standard:{}:{}", credential.name(), body_kind.name()),
                )
                .await;
                let response = fixture
                    .app
                    .clone()
                    .oneshot(request(
                        "POST",
                        "/api/admin/users",
                        credential.value(&fixture),
                        content_type,
                        body,
                    ))
                    .await
                    .unwrap();
                let response = capture(response).await;
                if let Some(sentinels) = &sentinels {
                    let touched_key_id = (body_kind == BodyKind::Valid)
                        .then(|| credential.active_api_key_id(&fixture))
                        .flatten();
                    assert_last_used_at(&fixture.pool, sentinels, touched_key_id, &case).await;
                }

                if body_kind == BodyKind::Valid && credential.is_admin() {
                    assert_eq!(response.status, StatusCode::OK, "{case}");
                    assert_eq!(response.content_type, JSON_CONTENT_TYPE, "{case}");
                    let response_json = response.json(&case);
                    let id = response_json["id"]
                        .as_str()
                        .unwrap_or_else(|| panic!("{case}: missing user id"));
                    assert_eq!(response_json, json!({ "id": id }), "{case}");
                    assert_eq!(
                        table_count(&fixture.pool, "users").await,
                        users_before + 1,
                        "{case}"
                    );
                    assert_eq!(
                        table_count(&fixture.pool, "admin_audit_logs").await,
                        audits_before + 1,
                        "{case}"
                    );
                    assert_created_user_and_audit(&fixture, id, &email).await;
                    successful_writes += 1;
                } else {
                    let expected = body_kind
                        .standard_rejection()
                        .unwrap_or_else(|| credential.auth_error());
                    assert_response(&response, &expected, &case);
                    assert_eq!(
                        table_count(&fixture.pool, "users").await,
                        users_before,
                        "{case}: rejected request mutated users"
                    );
                    assert_eq!(
                        table_count(&fixture.pool, "admin_audit_logs").await,
                        audits_before,
                        "{case}: rejected request created an audit"
                    );
                    assert!(!user_email_exists(&fixture.pool, &email).await, "{case}");
                }
            }
        }

        assert_eq!(successful_writes, 2);
        assert_eq!(table_count(&fixture.pool, "users").await, initial_users + 2);
        assert_eq!(
            table_count(&fixture.pool, "admin_audit_logs").await,
            initial_audits + 2
        );
    }

    #[tokio::test]
    async fn settings_json_matrix_preserves_auth_first_custom_errors_without_side_effects() {
        let fixture = admin_fixture().await;
        let initial_settings = settings_snapshot(&fixture.pool).await;
        let initial_audits = table_count(&fixture.pool, "admin_audit_logs").await;

        for credential in CREDENTIALS {
            for body_kind in REJECTED_BODIES {
                let case = format!("{} / {}", credential.name(), body_kind.name());
                let (content_type, body) = body_kind.request_parts("{}".to_string());
                let sentinels = reset_last_used_at_for_credential(
                    &fixture,
                    credential,
                    &format!("settings:{}:{}", credential.name(), body_kind.name()),
                )
                .await;
                let response = fixture
                    .app
                    .clone()
                    .oneshot(request(
                        "PATCH",
                        "/api/admin/settings",
                        credential.value(&fixture),
                        content_type,
                        body,
                    ))
                    .await
                    .unwrap();
                let response = capture(response).await;
                if let Some(sentinels) = &sentinels {
                    assert_last_used_at(
                        &fixture.pool,
                        sentinels,
                        credential.active_api_key_id(&fixture),
                        &case,
                    )
                    .await;
                }
                let expected = if credential.is_admin() {
                    body_kind.settings_rejection()
                } else {
                    credential.auth_error()
                };
                assert_response(&response, &expected, &case);
                assert_eq!(
                    settings_snapshot(&fixture.pool).await,
                    initial_settings,
                    "{case}"
                );
                assert_eq!(
                    table_count(&fixture.pool, "admin_audit_logs").await,
                    initial_audits,
                    "{case}: rejected settings request created an audit"
                );
            }
        }
    }

    #[tokio::test]
    async fn path_and_query_rejections_precede_body_and_authorization() {
        let fixture = admin_fixture().await;
        let credentials = [
            CredentialKind::AdminPanel,
            CredentialKind::AdminKey,
            CredentialKind::InvalidKey,
            CredentialKind::UserKey,
        ];
        let path_bodies = [BodyKind::Valid, BodyKind::Malformed];
        let expected_path = text_error(
            StatusCode::BAD_REQUEST,
            "Invalid URL: Invalid UTF-8 in `id`",
        );
        let expected_query = text_error(
            StatusCode::BAD_REQUEST,
            "Failed to deserialize query string: limit: invalid digit found in string",
        );

        for credential in credentials {
            for body_kind in path_bodies {
                let case = format!("path / {} / {}", credential.name(), body_kind.name());
                let (content_type, body) =
                    body_kind.request_parts(json!({ "role": "user" }).to_string());
                let users_before = table_count(&fixture.pool, "users").await;
                let audits_before = table_count(&fixture.pool, "admin_audit_logs").await;
                let sentinels = reset_last_used_at_for_credential(
                    &fixture,
                    credential,
                    &format!("path:{}:{}", credential.name(), body_kind.name()),
                )
                .await;
                let response = fixture
                    .app
                    .clone()
                    .oneshot(request(
                        "PATCH",
                        "/api/admin/users/%FF",
                        credential.value(&fixture),
                        content_type,
                        body,
                    ))
                    .await
                    .unwrap();
                let response = capture(response).await;
                if let Some(sentinels) = &sentinels {
                    assert_last_used_at(&fixture.pool, sentinels, None, &case).await;
                }
                assert_response(&response, &expected_path, &case);
                assert_eq!(
                    table_count(&fixture.pool, "users").await,
                    users_before,
                    "{case}"
                );
                assert_eq!(
                    table_count(&fixture.pool, "admin_audit_logs").await,
                    audits_before,
                    "{case}"
                );
            }

            let case = format!("query / {}", credential.name());
            let audits_before = table_count(&fixture.pool, "admin_audit_logs").await;
            let sentinels = reset_last_used_at_for_credential(
                &fixture,
                credential,
                &format!("query:{}", credential.name()),
            )
            .await;
            let response = fixture
                .app
                .clone()
                .oneshot(request(
                    "GET",
                    "/api/admin/requests?limit=oops",
                    credential.value(&fixture),
                    None,
                    Body::empty(),
                ))
                .await
                .unwrap();
            let response = capture(response).await;
            if let Some(sentinels) = &sentinels {
                assert_last_used_at(&fixture.pool, sentinels, None, &case).await;
            }
            assert_response(&response, &expected_query, &case);
            assert_eq!(
                table_count(&fixture.pool, "admin_audit_logs").await,
                audits_before,
                "{case}"
            );
        }
    }

    #[tokio::test]
    async fn administrator_extractor_accepts_both_sources_on_domain_routes() {
        let fixture = admin_fixture().await;

        for credential in [CredentialKind::AdminKey, CredentialKind::AdminPanel] {
            for path in [
                "/api/admin/users",
                "/api/admin/api-keys",
                "/api/admin/upstreams",
                "/api/admin/models",
                "/api/admin/requests",
                "/api/admin/limits",
                "/api/admin/settings",
            ] {
                let case = format!("{} / {path}", credential.name());
                let response = fixture
                    .app
                    .clone()
                    .oneshot(request(
                        "GET",
                        path,
                        credential.value(&fixture),
                        None,
                        Body::empty(),
                    ))
                    .await
                    .unwrap();
                let response = capture(response).await;
                assert_eq!(response.status, StatusCode::OK, "{case}");
                assert_eq!(response.content_type, JSON_CONTENT_TYPE, "{case}");
                let _: Value = response.json(&case);
            }
        }
    }

    #[tokio::test]
    async fn ordinary_panel_and_key_remain_valid_for_self_service() {
        let fixture = admin_fixture().await;

        for credential in [CredentialKind::UserPanel, CredentialKind::UserKey] {
            let case = credential.name();
            let response = fixture
                .app
                .clone()
                .oneshot(request(
                    "GET",
                    "/api/me",
                    credential.value(&fixture),
                    None,
                    Body::empty(),
                ))
                .await
                .unwrap();
            let response = capture(response).await;
            assert_eq!(response.status, StatusCode::OK, "{case}");
            assert_eq!(response.content_type, JSON_CONTENT_TYPE, "{case}");
            let body = response.json(case);
            assert_eq!(body["email"], "user@example.com", "{case}");
            assert_eq!(body["role"], "user", "{case}");
        }
    }

    impl CapturedResponse {
        fn json(&self, case: &str) -> Value {
            serde_json::from_slice(&self.body)
                .unwrap_or_else(|error| panic!("{case}: invalid JSON response: {error}"))
        }
    }

    async fn capture(response: axum::response::Response) -> CapturedResponse {
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("<missing>")
            .to_string();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        CapturedResponse {
            status,
            content_type,
            body,
        }
    }

    fn assert_response(response: &CapturedResponse, expected: &ExpectedResponse, case: &str) {
        assert_eq!(response.status, expected.status, "{case}");
        assert_eq!(response.content_type, expected.content_type, "{case}");
        match &expected.body {
            ExpectedBody::Json(expected) => assert_eq!(response.json(case), *expected, "{case}"),
            ExpectedBody::Text(expected) => {
                assert_eq!(response.body.as_slice(), expected.as_bytes(), "{case}")
            }
        }
    }

    fn gateway_error(
        status: StatusCode,
        code: &'static str,
        message: &'static str,
    ) -> ExpectedResponse {
        ExpectedResponse {
            status,
            content_type: JSON_CONTENT_TYPE,
            body: ExpectedBody::Json(json!({
                "error": {
                    "message": message,
                    "type": "gateway_error",
                    "code": code,
                    "details": null
                }
            })),
        }
    }

    fn text_error(status: StatusCode, body: &'static str) -> ExpectedResponse {
        ExpectedResponse {
            status,
            content_type: TEXT_CONTENT_TYPE,
            body: ExpectedBody::Text(body),
        }
    }

    fn request(
        method: &'static str,
        uri: &str,
        credential: Option<&str>,
        content_type: Option<&str>,
        body: Body,
    ) -> Request<Body> {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(credential) = credential {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {credential}"));
        }
        if let Some(content_type) = content_type {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }
        builder.body(body).unwrap()
    }

    async fn assert_created_user_and_audit(fixture: &AdminFixture, id: &str, email: &str) {
        let user = storage::get_user(&fixture.pool, id)
            .await
            .unwrap()
            .expect("created user exists");
        assert_eq!(user.email, email);
        assert_eq!(user.role, "user");
        assert_eq!(user.status, "active");

        let logs = storage::list_admin_audit_logs(&fixture.pool).await.unwrap();
        let audit = logs
            .iter()
            .find(|audit| audit.resource_id.as_deref() == Some(id))
            .expect("created user audit exists");
        assert_eq!(audit.actor_user_id, fixture.admin_id);
        assert_eq!(audit.actor_email, "admin@example.com");
        assert_eq!(audit.action, "create_user");
        assert_eq!(audit.resource_type, "user");
        assert_eq!(audit.status, "success");
        assert_eq!(
            serde_json::from_str::<Value>(audit.metadata_json.as_deref().unwrap()).unwrap(),
            json!({ "email": email, "role": "user" })
        );
    }

    async fn table_count(pool: &SqlitePool, table: &'static str) -> i64 {
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn user_email_exists(pool: &SqlitePool, email: &str) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE email = ?")
            .bind(email)
            .fetch_one(pool)
            .await
            .unwrap()
            != 0
    }

    async fn settings_snapshot(pool: &SqlitePool) -> Value {
        serde_json::to_value(storage::get_system_config(pool).await.unwrap()).unwrap()
    }

    async fn reset_last_used_at_for_credential(
        fixture: &AdminFixture,
        credential: CredentialKind,
        case: &str,
    ) -> Option<Vec<LastUsedSentinel>> {
        if !credential.is_api_key_case() {
            return None;
        }
        Some(reset_last_used_at(fixture, case).await)
    }

    async fn reset_last_used_at(fixture: &AdminFixture, case: &str) -> Vec<LastUsedSentinel> {
        let mut sentinels = Vec::new();
        for (label, key_id) in fixture.seeded_api_keys() {
            let value = format!("sentinel:{case}:{label}");
            let updated = sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE id = ?")
                .bind(&value)
                .bind(key_id)
                .execute(&fixture.pool)
                .await
                .unwrap();
            assert_eq!(
                updated.rows_affected(),
                1,
                "{case}: missing seeded {label} key"
            );
            sentinels.push(LastUsedSentinel {
                key_id: key_id.to_string(),
                value,
            });
        }
        sentinels
    }

    async fn assert_last_used_at(
        pool: &SqlitePool,
        sentinels: &[LastUsedSentinel],
        touched_key_id: Option<&str>,
        case: &str,
    ) {
        let mut touched = 0;
        for sentinel in sentinels {
            let actual: Option<String> =
                sqlx::query_scalar("SELECT last_used_at FROM api_keys WHERE id = ?")
                    .bind(&sentinel.key_id)
                    .fetch_one(pool)
                    .await
                    .unwrap();
            if touched_key_id == Some(sentinel.key_id.as_str()) {
                let actual =
                    actual.unwrap_or_else(|| panic!("{case}: touched key timestamp is null"));
                assert_ne!(
                    actual, sentinel.value,
                    "{case}: authentication did not touch key"
                );
                chrono::DateTime::parse_from_rfc3339(&actual)
                    .unwrap_or_else(|error| panic!("{case}: invalid touched timestamp: {error}"));
                touched += 1;
            } else {
                assert_eq!(
                    actual.as_deref(),
                    Some(sentinel.value.as_str()),
                    "{case}: unexpected key authentication side effect"
                );
            }
        }
        assert_eq!(touched, usize::from(touched_key_id.is_some()), "{case}");
    }

    async fn admin_fixture() -> AdminFixture {
        let pool = storage::connect_and_migrate("sqlite://:memory:")
            .await
            .unwrap();
        let config = test_config();
        let admin_id = create_user(&pool, "admin@example.com", "admin").await;
        let user_id = create_user(&pool, "user@example.com", "user").await;
        let disabled_user_id = create_user(&pool, "disabled@example.com", "admin").await;

        let (admin_key_id, admin_key) = create_key(&pool, &config, &admin_id, "admin", None).await;
        let (user_key_id, user_key) = create_key(&pool, &config, &user_id, "user", None).await;
        let (expired_admin_key_id, expired_admin_key) = create_key(
            &pool,
            &config,
            &admin_id,
            "expired",
            Some("2020-01-01T00:00:00Z"),
        )
        .await;
        let (disabled_key_row, disabled_admin_key) = storage::create_api_key(
            &pool,
            &config.app_secret,
            &admin_id,
            &CreateApiKey {
                name: "disabled".into(),
                expires_at: None,
            },
        )
        .await
        .unwrap();
        storage::set_api_key_status(&pool, &disabled_key_row.id, "disabled")
            .await
            .unwrap();
        let disabled_admin_key_id = disabled_key_row.id;
        let (disabled_user_key_id, disabled_user_key) =
            create_key(&pool, &config, &disabled_user_id, "disabled-user", None).await;

        let admin_panel = auth::generate_panel_token(&config.app_secret, &admin_id);
        let user_panel = auth::generate_panel_token(&config.app_secret, &user_id);
        let expired_admin_panel = expired_panel_token(&config.app_secret, &admin_id);
        let disabled_user_panel = auth::generate_panel_token(&config.app_secret, &disabled_user_id);
        storage::update_user(
            &pool,
            &disabled_user_id,
            &storage::UpdateUser {
                role: None,
                status: Some("disabled".into()),
                display_name: None,
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
        AdminFixture {
            app: build_app(state),
            pool,
            admin_id,
            admin_key_id,
            admin_key,
            admin_panel,
            user_key_id,
            user_key,
            user_panel,
            expired_admin_key_id,
            expired_admin_key,
            expired_admin_panel,
            disabled_admin_key_id,
            disabled_admin_key,
            disabled_user_key_id,
            disabled_user_key,
            disabled_user_panel,
        }
    }

    async fn create_user(pool: &SqlitePool, email: &str, role: &str) -> String {
        storage::ensure_user(
            pool,
            &CreateUser {
                email: email.into(),
                password: "password".into(),
                role: role.into(),
                display_name: None,
            },
        )
        .await
        .unwrap()
    }

    async fn create_key(
        pool: &SqlitePool,
        config: &Config,
        user_id: &str,
        name: &str,
        expires_at: Option<&str>,
    ) -> (String, String) {
        let (key, plaintext) = storage::create_api_key(
            pool,
            &config.app_secret,
            user_id,
            &CreateApiKey {
                name: name.into(),
                expires_at: expires_at.map(str::to_string),
            },
        )
        .await
        .unwrap();
        (key.id, plaintext)
    }

    fn expired_panel_token(app_secret: &str, user_id: &str) -> String {
        let payload = json!({
            "scope": "panel",
            "user_id": user_id,
            "session_id": "expired-session",
            "exp": 0
        });
        let payload_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
        let mut mac = Hmac::<Sha256>::new_from_slice(app_secret.as_bytes()).unwrap();
        mac.update(b"codex-gateway panel token v1");
        mac.update(payload_b64.as_bytes());
        format!(
            "cgw_panel_{payload_b64}.{}",
            hex::encode(mac.finalize().into_bytes())
        )
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
            default_request_timeout_ms: codex_gateway::config::default_request_timeout_ms(),
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
}
