use std::time::Instant;

use axum::http::{HeaderMap, StatusCode, header};
use bytes::Bytes;
use serde_json::Value;

use crate::{
    AppState,
    http_error::ApiError,
    routing::RouteCandidate,
    upstream::{self, headers as upstream_headers},
    usage::{self, UsageSnapshot},
};

use super::{
    planning::{self, RoutePlan},
    request::PreparedRequest,
    settlement::{self, AttemptCancellationGuard, AttemptLogBase, AttemptRecord, HealthUpdate},
};

pub(super) enum AttemptOutcome {
    Success(AttemptSuccess),
    RetryableFailure(AttemptFailure),
    TerminalError(TerminalAttemptError),
    StreamingHandoff(StreamingAttempt),
}

pub(super) struct AttemptSuccess {
    pub(super) response: UnaryResponse,
    pub(super) record: AttemptRecord,
}

pub(super) struct AttemptFailure {
    pub(super) response: FailureResponse,
    pub(super) record: Option<AttemptRecord>,
    pub(super) health: Option<HealthUpdate>,
}

pub(super) struct TerminalAttemptError {
    pub(super) error: ApiError,
    pub(super) health: Option<HealthUpdate>,
}

pub(super) struct StreamingAttempt {
    pub(super) upstream_response: reqwest::Response,
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) record: AttemptRecord,
    pub(super) upstream_id: String,
}

pub(super) struct UnaryResponse {
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) body: Bytes,
}

pub(super) enum FailureResponse {
    Gateway(ApiError),
    Upstream(UnaryResponse),
}

impl FailureResponse {
    pub(super) fn into_result(self) -> Result<UnaryResponse, ApiError> {
        match self {
            Self::Gateway(error) => Err(error),
            Self::Upstream(response) => Ok(response),
        }
    }
}

pub(super) async fn execute(
    state: &AppState,
    request: &PreparedRequest,
    plan: &RoutePlan,
    route: &RouteCandidate,
    index: usize,
    retries_remaining: i64,
) -> AttemptOutcome {
    let started = Instant::now();
    let base = log_base(
        request,
        plan,
        route,
        index,
        retries_remaining,
        state.clock.clone(),
    );
    let mut attempt_json = request.json.clone();
    if route.upstream_model_name != request.model
        && let Some(object) = attempt_json.as_object_mut()
    {
        object.insert(
            "model".to_string(),
            Value::String(route.upstream_model_name.clone()),
        );
    }
    let upstream_body = match serde_json::to_vec(&attempt_json) {
        Ok(body) => body,
        Err(error) => {
            tracing::error!(?error, "failed to encode upstream request");
            return AttemptOutcome::TerminalError(TerminalAttemptError {
                error: ApiError::gateway(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "gateway request encoding error",
                    "gateway_internal_error",
                ),
                health: None,
            });
        }
    };
    let url = match upstream::join_upstream_url(&route.base_url, request.canonical_path) {
        Ok(url) => url,
        Err(error) => {
            tracing::warn!(?error, base_url = %route.base_url, "invalid upstream URL");
            return pre_attempt_failure(
                route,
                base,
                started,
                "invalid_url",
                "invalid upstream URL",
                "upstream_unavailable",
            );
        }
    };
    let request_headers = match upstream_headers::forward_request_headers(
        &request.headers,
        &route.upstream_api_key,
    ) {
        Ok(headers) => headers,
        Err(error) => {
            tracing::warn!(
                ?error,
                upstream_id = %route.upstream_id,
                "invalid stored upstream authorization header"
            );
            return pre_attempt_failure(
                route,
                base,
                started,
                "invalid_authorization_header",
                "invalid upstream configuration",
                "upstream_unavailable",
            );
        }
    };

    let mut cancellation = AttemptCancellationGuard::new(
        state.db.clone(),
        state.finalizations.clone(),
        base.clone(),
        started,
    );
    let upstream_response = state
        .http
        .request(request.reqwest_method.clone(), url)
        .headers(request_headers)
        .body(upstream_body)
        .timeout(std::time::Duration::from_millis(
            route.timeout_ms.max(1) as u64
        ))
        .send()
        .await;
    let upstream_response = match upstream_response {
        Ok(response) => response,
        Err(error) => {
            cancellation.disarm();
            tracing::warn!(?error, upstream = %route.upstream_name, "upstream request failed");
            let (status, error_code, health) = classify_upstream_error(&error);
            return AttemptOutcome::RetryableFailure(AttemptFailure {
                response: FailureResponse::Gateway(ApiError::gateway(
                    status,
                    "upstream request failed",
                    error_code,
                )),
                record: Some(record(
                    base,
                    route,
                    status,
                    Some(error_code),
                    UsageSnapshot::default(),
                    0,
                    started,
                    Some(health),
                )),
                health: None,
            });
        }
    };

    let status = StatusCode::from_u16(upstream_response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let status_error_code = error_code_for_status(status);
    settlement::persist_response_health(
        &state.db,
        &state.finalizations,
        HealthUpdate {
            upstream_id: route.upstream_id.clone(),
            status: health_for_status(status),
            error_sample: status_error_code,
        },
    )
    .await;
    let response_headers = upstream_headers::forward_response_headers(upstream_response.headers());
    let is_sse = request.stream_requested
        || upstream_response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("text/event-stream"));
    if is_sse {
        cancellation.disarm();
        let mut base = base;
        base.stream = true;
        return AttemptOutcome::StreamingHandoff(StreamingAttempt {
            upstream_response,
            status,
            headers: response_headers,
            record: record(
                base,
                route,
                status,
                status_error_code,
                UsageSnapshot::default(),
                0,
                started,
                None,
            ),
            upstream_id: route.upstream_id.clone(),
        });
    }

    let bytes = match upstream_response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            cancellation.disarm();
            tracing::warn!(?error, "reading upstream body failed");
            let (status, error_code, health) = classify_body_error(&error);
            return AttemptOutcome::RetryableFailure(AttemptFailure {
                response: FailureResponse::Gateway(ApiError::gateway(
                    status,
                    "upstream body error",
                    error_code,
                )),
                record: Some(record(
                    base,
                    route,
                    status,
                    Some(error_code),
                    UsageSnapshot::default(),
                    0,
                    started,
                    Some(health),
                )),
                health: None,
            });
        }
    };
    cancellation.disarm();
    let usage = if status.is_success() {
        parse_unary_usage(&response_headers, &bytes)
    } else {
        UsageSnapshot::default()
    };
    let response = UnaryResponse {
        status,
        headers: response_headers,
        body: bytes.clone(),
    };
    let record = record(
        base,
        route,
        status,
        status_error_code,
        usage,
        bytes.len() as i64,
        started,
        None,
    );
    if is_retryable_status(status) {
        AttemptOutcome::RetryableFailure(AttemptFailure {
            response: FailureResponse::Upstream(response),
            record: Some(record),
            health: None,
        })
    } else {
        AttemptOutcome::Success(AttemptSuccess { response, record })
    }
}

fn log_base(
    request: &PreparedRequest,
    plan: &RoutePlan,
    route: &RouteCandidate,
    index: usize,
    retries_remaining: i64,
    clock: crate::clock::SharedClock,
) -> AttemptLogBase {
    let started_at = clock
        .now()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    AttemptLogBase {
        clock,
        request_id: if index == 0 {
            request.request_id.clone()
        } else {
            format!("{}-{}", request.request_id, index + 1)
        },
        user_id: request.user.user_id.clone(),
        api_key_id: request.user.api_key_id.clone(),
        model_id: Some(route.model_id.clone()),
        upstream_id: Some(route.upstream_id.clone()),
        method: request.method.to_string(),
        path: request.path.clone(),
        stream: request.stream_requested,
        started_at,
        user_agent: request
            .headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
        input_chars: request.body.len() as i64,
        client_metadata_sanitized: request.client_metadata_sanitized.clone(),
        route_strategy: Some(plan.route_strategy.clone()),
        route_decision_json: Some(planning::route_decision_json(
            route,
            index,
            plan.candidates.len(),
            retries_remaining,
            &plan.route_key_hash,
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn record(
    base: AttemptLogBase,
    route: &RouteCandidate,
    status: StatusCode,
    error_code: Option<&'static str>,
    usage: UsageSnapshot,
    output_chars: i64,
    started: Instant,
    health: Option<&'static str>,
) -> AttemptRecord {
    AttemptRecord {
        base,
        status,
        error_code: error_code.map(str::to_string),
        usage,
        output_chars,
        started,
        health: health.map(|status| HealthUpdate {
            upstream_id: route.upstream_id.clone(),
            status,
            error_sample: error_code,
        }),
    }
}

fn pre_attempt_failure(
    route: &RouteCandidate,
    base: AttemptLogBase,
    started: Instant,
    health_error: &'static str,
    message: &'static str,
    error_code: &'static str,
) -> AttemptOutcome {
    AttemptOutcome::RetryableFailure(AttemptFailure {
        response: FailureResponse::Gateway(ApiError::gateway(
            StatusCode::BAD_GATEWAY,
            message,
            error_code,
        )),
        record: Some(AttemptRecord {
            base,
            status: StatusCode::BAD_GATEWAY,
            error_code: Some(error_code.to_string()),
            usage: UsageSnapshot::default(),
            output_chars: 0,
            started,
            health: Some(HealthUpdate {
                upstream_id: route.upstream_id.clone(),
                status: "degraded",
                error_sample: Some(health_error),
            }),
        }),
        health: None,
    })
}

fn parse_unary_usage(headers: &HeaderMap, bytes: &[u8]) -> UsageSnapshot {
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("json"));
    if !is_json {
        return UsageSnapshot::default();
    }
    serde_json::from_slice::<Value>(bytes)
        .map(|value| usage::extract_usage_from_json(&value))
        .unwrap_or_default()
}

fn classify_upstream_error(error: &reqwest::Error) -> (StatusCode, &'static str, &'static str) {
    if error.is_timeout() {
        (StatusCode::GATEWAY_TIMEOUT, "upstream_timeout", "down")
    } else if error.is_connect() {
        (StatusCode::BAD_GATEWAY, "upstream_error", "down")
    } else {
        (StatusCode::BAD_GATEWAY, "upstream_error", "degraded")
    }
}

fn classify_body_error(error: &reqwest::Error) -> (StatusCode, &'static str, &'static str) {
    if error.is_timeout() {
        (StatusCode::GATEWAY_TIMEOUT, "upstream_timeout", "down")
    } else {
        (StatusCode::BAD_GATEWAY, "upstream_error", "degraded")
    }
}

fn is_retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT
    )
}

fn error_code_for_status(status: StatusCode) -> Option<&'static str> {
    if status == StatusCode::GATEWAY_TIMEOUT {
        Some("upstream_timeout")
    } else if status.is_client_error() || status.is_server_error() {
        Some("upstream_error")
    } else {
        None
    }
}

fn health_for_status(status: StatusCode) -> &'static str {
    if status.is_success() {
        "healthy"
    } else if is_retryable_status(status) {
        "degraded"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, sync::Arc, time::Duration};

    use axum::{Router, body::Body, routing::post};
    use bytes::Bytes;

    use crate::{
        FinalizationLifecycle,
        auth::AuthenticatedUser,
        config::{Config, RouteStrategy, RuntimeConfig},
        storage::{self, CreateApiKey, CreateUser, TimeoutPatchValue, UpsertUpstream},
    };

    use super::*;
    use crate::proxy::{
        planning::RoutePlan, request::PreparedRequest, settlement::AdmissionSettlement,
    };

    #[tokio::test]
    async fn response_headers_update_health_before_stalled_unary_body_settles() {
        let upstream_app = Router::new().route(
            "/responses",
            post(|| async {
                let body = Body::from_stream(async_stream::stream! {
                    std::future::pending::<()>().await;
                    yield Ok::<Bytes, Infallible>(Bytes::new());
                });
                ([(header::CONTENT_TYPE, "application/json")], body)
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_url = format!("http://{}", listener.local_addr().unwrap());
        let upstream_server = tokio::spawn(async move {
            axum::serve(listener, upstream_app).await.unwrap();
        });

        let pool = storage::connect_and_migrate("sqlite://:memory:")
            .await
            .unwrap();
        let config = test_config();
        let user_id = storage::ensure_user(
            &pool,
            &CreateUser {
                email: "header-health@example.com".into(),
                password: "password".into(),
                role: "user".into(),
                display_name: None,
            },
        )
        .await
        .unwrap();
        let (api_key, _) = storage::create_api_key(
            &pool,
            &config.app_secret,
            &user_id,
            &CreateApiKey {
                name: "header-health".into(),
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
                name: "header-health".into(),
                base_url: upstream_url.clone(),
                api_key: "sk-upstream".into(),
                enabled: Some(true),
                priority: Some(1),
                weight: Some(1),
                timeout_ms: TimeoutPatchValue::Explicit(5_000),
                max_retries: Some(0),
                health_check_path: None,
            },
        )
        .await
        .unwrap();
        let (lifecycle, finalizations) = FinalizationLifecycle::new();
        let state = AppState {
            config: Arc::new(config),
            db: pool.clone(),
            http: reqwest::Client::new(),
            finalizations: finalizations.clone(),
            clock: crate::clock::system_clock(),
        };
        let request = PreparedRequest {
            request_id: "header-health-request".into(),
            user: AuthenticatedUser {
                user_id: user_id.clone(),
                api_key_id: api_key.id.clone(),
                key_prefix: api_key.key_prefix.clone(),
                email: "header-health@example.com".into(),
                role: "user".into(),
            },
            runtime: RuntimeConfig {
                route_strategy: RouteStrategy::Priority,
                default_request_timeout_ms: 5_000,
                max_request_body_bytes: 1024,
                request_log_retention_days: 90,
                daily_usage_retention_days: 730,
                expose_debug_headers: false,
            },
            method: axum::http::Method::POST,
            reqwest_method: reqwest::Method::POST,
            headers: HeaderMap::new(),
            path: "/responses".into(),
            canonical_path: "/responses",
            body: Bytes::from_static(br#"{"model":"codex-mini"}"#),
            json: serde_json::json!({"model": "codex-mini"}),
            model: "codex-mini".into(),
            stream_requested: false,
            client_metadata_sanitized: None,
            can_retry: true,
        };
        let route = RouteCandidate {
            model_id: "model-id".into(),
            public_name: "codex-mini".into(),
            upstream_model_id: "mapping-id".into(),
            upstream_model_name: "upstream-model".into(),
            upstream_model_priority: 1,
            upstream_model_weight: 1,
            upstream_id: upstream.id.clone(),
            upstream_name: upstream.name.clone(),
            base_url: upstream_url,
            upstream_api_key: "sk-upstream".into(),
            upstream_api_key_secret_version: 1,
            upstream_priority: 1,
            upstream_weight: 1,
            timeout_ms: 5_000,
            timeout_ms_is_explicit: 1,
            max_retries: 0,
        };
        let plan = RoutePlan {
            candidates: vec![route],
            route_strategy: "priority".into(),
            route_key_hash: "route-hash".into(),
        };
        let admission = storage::admit_limited_request(&pool, &user_id, &api_key.id)
            .await
            .unwrap();
        let settlement = AdmissionSettlement::new(
            pool.clone(),
            finalizations.clone(),
            state.clock.clone(),
            admission,
        );
        let attempt = tokio::spawn(async move {
            execute(&state, &request, &plan, &plan.candidates[0], 0, 0).await
        });

        tokio::time::timeout(
            Duration::from_secs(2),
            lifecycle.wait_for_completed_tasks(1),
        )
        .await
        .unwrap();
        let stored_upstream = storage::get_upstream(&pool, &upstream.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_upstream.last_health_status, "healthy");
        assert!(
            storage::list_request_logs(&pool, None)
                .await
                .unwrap()
                .is_empty()
        );
        let limit_state = storage::user_limit_state(&pool, &user_id, Some(&api_key.id))
            .await
            .unwrap();
        assert_eq!(limit_state.user.concurrency.in_flight, 1);

        attempt.abort();
        let cancellation = match attempt.await {
            Err(error) => error,
            Ok(_) => panic!("stalled attempt unexpectedly completed"),
        };
        assert!(cancellation.is_cancelled());
        drop(settlement);
        drop(finalizations);
        tokio::time::timeout(Duration::from_secs(2), lifecycle.drain())
            .await
            .unwrap();
        let limit_state = storage::user_limit_state(&pool, &user_id, Some(&api_key.id))
            .await
            .unwrap();
        assert_eq!(limit_state.user.concurrency.in_flight, 0);
        upstream_server.abort();
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
            default_request_timeout_ms: 5_000,
            max_request_body_bytes: 1024,
            health_checks_enabled: false,
            health_check_interval_ms: 30_000,
            request_log_retention_days: 90,
            daily_usage_retention_days: 730,
            retention_run_on_startup: false,
            expose_debug_headers: false,
            admin_email: None,
            admin_password: None,
            bootstrap_admin_key: None,
            runtime_env: Default::default(),
        }
    }
}
