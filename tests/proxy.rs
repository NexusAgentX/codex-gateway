mod support;

use support::*;

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
    assert_limit_settlement(&pool, 1, 3).await;
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
    assert!(logs.iter().all(|log| {
        !format!("{log:?}").contains("compact-secret")
            && !format!("{log:?}").contains("must-not-forward")
    }));
    assert_limit_settlement(&pool, 2, 18).await;
}

#[tokio::test]
async fn non_stream_client_disconnect_settles_once_and_logs_real_attempt_once() {
    let (upstream, upstream_calls, upstream_entered, release_upstream) =
        spawn_blocking_counting_upstream().await;
    let (app, key, pool, lifecycle) = tracked_test_app_with_pool(Some(&upstream)).await;
    let request = tokio::spawn(async move {
        app.oneshot(proxy_request("/responses", &key))
            .await
            .unwrap()
    });

    upstream_entered.notified().await;
    assert_eq!(upstream_calls.load(Ordering::SeqCst), 1);
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    release_upstream.notify_one();

    await_finalizations(&lifecycle, 2).await;
    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].status_code, Some(499));
    assert_eq!(logs[0].error_code.as_deref(), Some("client_disconnected"));
    assert_limit_settlement(&pool, 1, 0).await;
}

#[test]
fn graceful_shutdown_drains_non_stream_and_stream_drop_finalization() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}",
        temp_dir.path().join("shutdown-finalization.db").display()
    );
    let runtime_database_url = database_url.clone();
    let report = std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let non_stream_entered = Arc::new(tokio::sync::Notify::new());
            let release_non_stream = Arc::new(tokio::sync::Notify::new());
            let upstream_app = Router::new().route(
                "/responses",
                post({
                    let non_stream_entered = non_stream_entered.clone();
                    let release_non_stream = release_non_stream.clone();
                    move |Json(body): Json<Value>| {
                        let non_stream_entered = non_stream_entered.clone();
                        let release_non_stream = release_non_stream.clone();
                        async move {
                            if body["stream"].as_bool().unwrap_or(false) {
                                let body = Body::from_stream(async_stream::stream! {
                                    yield Ok::<_, Infallible>(bytes::Bytes::from_static(
                                        b"data: {\"type\":\"response.created\"}\n\n",
                                    ));
                                    std::future::pending::<()>().await;
                                });
                                return ([(header::CONTENT_TYPE, "text/event-stream")], body)
                                    .into_response();
                            }
                            non_stream_entered.notify_one();
                            release_non_stream.notified().await;
                            Json(json!({
                                "model_seen": "shutdown-model",
                                "usage": {
                                    "input_tokens": 1,
                                    "output_tokens": 2,
                                    "total_tokens": 3
                                }
                            }))
                            .into_response()
                        }
                    }
                }),
            );
            let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let upstream_url = format!("http://{}", upstream_listener.local_addr().unwrap());
            let upstream_server = tokio::spawn(async move {
                axum::serve(upstream_listener, upstream_app).await.unwrap();
            });

            let pool = storage::connect_and_migrate(&runtime_database_url)
                .await
                .unwrap();
            let config = test_config();
            let user_id = seed_user_model(&pool, Some(&upstream_url)).await;
            let (_, key) = storage::create_api_key(
                &pool,
                &config.app_secret,
                &user_id,
                &CreateApiKey {
                    name: "shutdown-finalization".into(),
                    expires_at: None,
                },
            )
            .await
            .unwrap();
            let (lifecycle, finalizations) = FinalizationLifecycle::new();
            let app = build_app(AppState {
                config: Arc::new(config),
                db: pool.clone(),
                http: reqwest::Client::new(),
                finalizations,
                clock: codex_gateway::clock::system_clock(),
            });
            let gateway_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
            let gateway_server = tokio::spawn({
                let app = app.clone();
                async move {
                    axum::serve(gateway_listener, app)
                        .with_graceful_shutdown(async {
                            let _ = shutdown_rx.await;
                        })
                        .await
                        .unwrap();
                }
            });

            let non_stream = tokio::spawn({
                let app = app.clone();
                let key = key.clone();
                async move {
                    app.oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/responses")
                            .header(header::AUTHORIZATION, format!("Bearer {key}"))
                            .header(header::CONTENT_TYPE, "application/json")
                            .header("x-request-id", "shutdown-non-stream")
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
                    .unwrap()
                }
            });
            non_stream_entered.notified().await;
            non_stream.abort();
            let non_stream_cancelled = match non_stream.await {
                Err(error) => error,
                Ok(_) => panic!("blocked non-stream request unexpectedly completed"),
            };
            assert!(non_stream_cancelled.is_cancelled());
            release_non_stream.notify_one();

            let stream_response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/responses")
                        .header(header::AUTHORIZATION, format!("Bearer {key}"))
                        .header(header::CONTENT_TYPE, "application/json")
                        .header("x-request-id", "shutdown-stream")
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
            assert_eq!(stream_response.status(), StatusCode::OK);
            drop(stream_response);

            shutdown_tx.send(()).unwrap();
            gateway_server.await.unwrap();
            drop(app);
            let report = lifecycle.drain().await;
            pool.close().await;
            upstream_server.abort();
            report
        })
    })
    .join()
    .unwrap();

    assert_eq!(report.registered_tasks, 4);
    assert_eq!(report.completed_tasks, 4);
    assert_eq!(report.panicked_tasks, 0);
    assert_eq!(report.attempt_persistence_tasks, 1);
    assert_eq!(report.admission_finalization_tasks, 1);
    assert_eq!(report.stream_finalization_tasks, 1);
    assert_eq!(report.upstream_health_tasks, 1);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let pool = storage::connect_and_migrate(&database_url).await.unwrap();
        let logs = storage::list_request_logs(&pool, None).await.unwrap();
        assert_eq!(logs.len(), 2);
        assert!(logs.iter().all(|log| {
            log.status_code == Some(499) && log.error_code.as_deref() == Some("client_disconnected")
        }));
        assert!(
            logs.iter()
                .any(|log| log.request_id == "shutdown-non-stream")
        );
        assert!(logs.iter().any(|log| log.request_id == "shutdown-stream"));
        assert_limit_settlement(&pool, 2, 0).await;
        pool.close().await;
    });
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
    assert_limit_settlement(&pool, 1, 3).await;
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
    let slow = spawn_stalling_upstream().await;
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

    let first = storage::list_upstreams(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|upstream| upstream.name == "first")
        .unwrap();
    assert_eq!(first.last_health_status, "down");
    assert!(first.recent_error_samples.contains("upstream_timeout"));
    assert_limit_settlement(&pool, 1, 3).await;
}

#[tokio::test]
async fn body_read_timeout_retries_next_eligible_upstream() {
    let stalled = spawn_body_stall_upstream().await;
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

    let first = storage::list_upstreams(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|upstream| upstream.name == "first")
        .unwrap();
    assert_eq!(first.last_health_status, "down");
    assert!(first.recent_error_samples.contains("upstream_timeout"));
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
    assert_limit_settlement(&pool, 1, 0).await;
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
    let (app, key, pool, lifecycle) = tracked_test_app_with_pool(Some(&upstream)).await;
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

    await_finalizations(&lifecycle, 2).await;
    let logs = storage::list_request_logs(&pool, None).await.unwrap();
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
    assert_limit_settlement(&pool, 1, 24).await;

    gateway_handle.abort();
}

#[tokio::test]
async fn streaming_response_finalizes_when_client_drops_after_completed_event() {
    let initial = DateTime::parse_from_rfc3339("2046-02-03T04:05:06Z")
        .unwrap()
        .with_timezone(&Utc);
    let clock = TestClock::new(initial);
    let upstream = spawn_completed_then_stalling_sse_upstream(17, 19, 36).await;
    let (app, key, pool, lifecycle) = TestAppBuilder::new()
        .upstream(upstream)
        .clock(clock)
        .build_tracked()
        .await;
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

    await_finalizations(&lifecycle, 2).await;
    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].status_code, Some(200));
    assert_eq!(logs[0].error_code, None);
    assert_eq!(logs[0].stream, 1);
    assert_eq!(logs[0].usage_source, "upstream");
    assert_eq!(logs[0].prompt_tokens, 17);
    assert_eq!(logs[0].completion_tokens, 19);
    assert_eq!(logs[0].total_tokens, 36);
    assert_eq!(logs[0].started_at, "2046-02-03T04:05:06.000Z");
    assert_eq!(
        logs[0].finished_at.as_deref(),
        Some("2046-02-03T04:05:06.000Z")
    );

    let usage = storage::list_daily_usage(&pool, None).await.unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].request_count, 1);
    assert_eq!(usage[0].stream_count, 1);
    assert_eq!(usage[0].prompt_tokens, 17);
    assert_eq!(usage[0].completion_tokens, 19);
    assert_eq!(usage[0].total_tokens, 36);
    assert_limit_settlement(&pool, 1, 36).await;

    gateway_handle.abort();
}

#[tokio::test]
async fn sse_client_disconnect_finalizes_log_and_cancels_upstream() {
    let initial = DateTime::parse_from_rfc3339("2047-08-09T10:11:12Z")
        .unwrap()
        .with_timezone(&Utc);
    let clock = TestClock::new(initial);
    let (upstream, upstream_dropped) = spawn_cancellable_sse_upstream().await;
    let (app, key, pool, lifecycle) = TestAppBuilder::new()
        .upstream(upstream)
        .clock(clock.clone())
        .build_tracked()
        .await;
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
    clock.advance(ChronoDuration::minutes(11));
    drop(stream);

    tokio::time::timeout(Duration::from_secs(2), upstream_dropped)
        .await
        .unwrap()
        .unwrap();
    await_finalizations(&lifecycle, 2).await;
    let logs = storage::list_request_logs(&pool, None).await.unwrap();
    assert_eq!(logs[0].status_code, Some(499));
    assert_eq!(logs[0].error_code.as_deref(), Some("client_disconnected"));
    assert_eq!(logs[0].started_at, "2047-08-09T10:11:12.000Z");
    assert_eq!(
        logs[0].finished_at.as_deref(),
        Some("2047-08-09T10:22:12.000Z")
    );
    assert_eq!(logs[0].stream, 1);
    assert_eq!(logs[0].usage_source, "unknown");
    assert!(logs[0].finished_at.is_some());
    assert!(logs[0].output_chars > 0);
    assert_limit_settlement(&pool, 1, 0).await;

    assert!(!format!("{:?}", logs[0]).contains("stream-secret should not persist"));
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
