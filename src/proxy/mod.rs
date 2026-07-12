use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Instant,
};

use async_stream::stream;
use axum::{
    Json,
    body::Body,
    extract::{Extension, OriginalUri, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use serde_json::{Map, Value, json};

use crate::{
    AppState, RequestId,
    api::{self, ApiError},
    auth, request_id_header,
    routing::{self, RoutingError},
    storage::{self, RequestLogInsert},
    upstream,
    usage::{self, SseUsageScanner, UsageSnapshot},
};

pub async fn models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    api::authenticate_api_key(&state, &headers).await?;
    let models = storage::list_visible_models(&state.db).await?;
    let data: Vec<Value> = models
        .into_iter()
        .map(|model| {
            json!({
                "id": model.public_name,
                "display_name": model.public_name,
                "object": "model",
                "type": "model",
                "created_at": model.created_at
            })
        })
        .collect();
    Ok(Json(json!({ "object": "list", "data": data })))
}

pub async fn proxy_responses(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    OriginalUri(uri): OriginalUri,
    request: Request,
) -> Result<Response, ApiError> {
    let runtime = storage::runtime_config(&state.db, &state.config).await?;
    let (parts, body) = request.into_parts();
    let method = parts.method;
    let headers = parts.headers;
    let user = api::authenticate_api_key(&state, &headers).await?;
    let path = uri.path().to_string();
    let canonical_path = upstream::canonical_proxy_path(&path).ok_or_else(|| {
        ApiError::gateway(StatusCode::NOT_FOUND, "unsupported proxy path", "not_found")
    })?;
    let body = read_limited_body(body, runtime.effective.max_request_body_bytes).await?;

    let request_json: Value = serde_json::from_slice(&body).map_err(|_| {
        ApiError::gateway(
            StatusCode::BAD_REQUEST,
            "request body must be JSON",
            "invalid_request",
        )
    })?;
    let model = request_json
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ApiError::gateway(StatusCode::BAD_REQUEST, "missing model", "model_not_found")
        })?
        .to_string();
    let stream_requested = request_json
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && canonical_path == "/responses";
    let client_metadata_sanitized = sanitize_client_metadata(
        request_json.get("client_metadata"),
        &state.config.app_secret,
    );

    let candidates = routing::route_candidates(
        &state.db,
        &state.config,
        &model,
        runtime.effective.default_request_timeout_ms,
    )
    .await
    .map_err(|error| route_error(&model, RoutingError::Storage(error)))?;
    if candidates.is_empty() {
        let model_exists = routing::model_exists(&state.db, &model)
            .await
            .map_err(|error| route_error(&model, RoutingError::Storage(error)))?;
        return Err(if model_exists {
            route_error(&model, RoutingError::UpstreamUnavailable)
        } else {
            route_error(&model, RoutingError::ModelNotFound)
        });
    }

    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes()).map_err(|_| {
        ApiError::gateway(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unsupported HTTP method",
            "gateway_internal_error",
        )
    })?;

    let can_retry = !stream_requested;
    let route_seed = uuid::Uuid::new_v4().to_string();
    let route_key = routing_key(
        runtime.effective.route_strategy,
        &model,
        &request_json,
        &user.api_key_id,
        &route_seed,
    );
    let route_key_hash = auth::hash_api_key(&state.config.app_secret, &route_key);
    let route_strategy = route_strategy_name(runtime.effective.route_strategy).to_string();
    let candidates =
        routing::order_candidates(&candidates, runtime.effective.route_strategy, &route_key);
    let candidate_count = candidates.len();
    let mut retries_remaining = candidates
        .first()
        .map(|candidate| candidate.max_retries.max(0))
        .unwrap_or_default();
    let limit_admission =
        storage::admit_limited_request(&state.db, &user.user_id, &user.api_key_id).await?;
    let mut limit_admission = Some(limit_admission);

    for (index, route) in candidates.into_iter().enumerate() {
        if index > 0 {
            if retries_remaining <= 0 {
                break;
            }
            retries_remaining -= 1;
        }

        let attempt_started_at = storage::now_string();
        let attempt_started = Instant::now();
        let has_next = index + 1 < candidate_count;
        let can_retry_after_attempt = can_retry && has_next && retries_remaining > 0;
        let mut attempt_json = request_json.clone();
        if route.upstream_model_name != model
            && let Some(object) = attempt_json.as_object_mut()
        {
            object.insert(
                "model".to_string(),
                Value::String(route.upstream_model_name.clone()),
            );
        }

        let attempt_request_id = if index == 0 {
            request_id.0.clone()
        } else {
            format!("{}-{}", request_id.0, index + 1)
        };
        let log_base = LogBase {
            request_id: attempt_request_id,
            user_id: user.user_id.clone(),
            api_key_id: user.api_key_id.clone(),
            model_id: Some(route.model_id.clone()),
            upstream_id: Some(route.upstream_id.clone()),
            method: method.to_string(),
            path: path.clone(),
            stream: stream_requested,
            started_at: attempt_started_at,
            user_agent: headers
                .get(header::USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
            input_chars: body.len() as i64,
            client_metadata_sanitized: client_metadata_sanitized.clone(),
            route_strategy: Some(route_strategy.clone()),
            route_decision_json: Some(route_decision_json(
                &route,
                index,
                candidate_count,
                retries_remaining,
                &route_key_hash,
            )),
        };

        let upstream_body = match serde_json::to_vec(&attempt_json) {
            Ok(body) => body,
            Err(error) => {
                tracing::error!(?error, "failed to encode upstream request");
                let status = StatusCode::INTERNAL_SERVER_ERROR;
                log_attempt(
                    &state,
                    log_base,
                    status,
                    Some("gateway_internal_error".to_string()),
                    UsageSnapshot::default(),
                    0,
                    attempt_started,
                )
                .await;
                finalize_limit(&state, limit_admission.take(), UsageSnapshot::default()).await;
                return Err(ApiError::gateway(
                    status,
                    "gateway request encoding error",
                    "gateway_internal_error",
                ));
            }
        };

        let url = match upstream::join_upstream_url(&route.base_url, canonical_path) {
            Ok(url) => url,
            Err(error) => {
                tracing::warn!(?error, base_url = %route.base_url, "invalid upstream URL");
                let status = StatusCode::BAD_GATEWAY;
                let _ = storage::record_upstream_health(
                    &state.db,
                    &route.upstream_id,
                    "degraded",
                    Some("invalid_url"),
                )
                .await;
                log_attempt(
                    &state,
                    log_base,
                    status,
                    Some("upstream_unavailable".to_string()),
                    UsageSnapshot::default(),
                    0,
                    attempt_started,
                )
                .await;
                if can_retry_after_attempt {
                    continue;
                }
                finalize_limit(&state, limit_admission.take(), UsageSnapshot::default()).await;
                return Err(ApiError::gateway(
                    status,
                    "invalid upstream URL",
                    "upstream_unavailable",
                ));
            }
        };

        let request_headers = match forward_request_headers(&headers, &route.upstream_api_key) {
            Ok(headers) => headers,
            Err(error) => {
                tracing::warn!(
                    ?error,
                    upstream_id = %route.upstream_id,
                    "invalid stored upstream authorization header"
                );
                let status = StatusCode::BAD_GATEWAY;
                let _ = storage::record_upstream_health(
                    &state.db,
                    &route.upstream_id,
                    "degraded",
                    Some("invalid_authorization_header"),
                )
                .await;
                log_attempt(
                    &state,
                    log_base,
                    status,
                    Some("upstream_unavailable".to_string()),
                    UsageSnapshot::default(),
                    0,
                    attempt_started,
                )
                .await;
                if can_retry_after_attempt {
                    continue;
                }
                finalize_limit(&state, limit_admission.take(), UsageSnapshot::default()).await;
                return Err(ApiError::gateway(
                    status,
                    "invalid upstream configuration",
                    "upstream_unavailable",
                ));
            }
        };

        let upstream_response = state
            .http
            .request(reqwest_method.clone(), url)
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
                tracing::warn!(?error, upstream = %route.upstream_name, "upstream request failed");
                let (status, error_code, health) = classify_upstream_error(&error);
                let _ = storage::record_upstream_health(
                    &state.db,
                    &route.upstream_id,
                    health,
                    Some(error_code),
                )
                .await;
                log_attempt(
                    &state,
                    log_base,
                    status,
                    Some(error_code.to_string()),
                    UsageSnapshot::default(),
                    0,
                    attempt_started,
                )
                .await;
                if can_retry_after_attempt {
                    continue;
                }
                finalize_limit(&state, limit_admission.take(), UsageSnapshot::default()).await;
                return Err(ApiError::gateway(
                    status,
                    "upstream request failed",
                    error_code,
                ));
            }
        };

        let status = StatusCode::from_u16(upstream_response.status().as_u16())
            .unwrap_or(StatusCode::BAD_GATEWAY);
        let status_error_code = error_code_for_status(status);
        let _ = storage::record_upstream_health(
            &state.db,
            &route.upstream_id,
            health_for_status(status),
            status_error_code,
        )
        .await;
        let response_headers = forward_response_headers(upstream_response.headers());
        let is_sse = stream_requested
            || upstream_response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.contains("text/event-stream"));

        if is_sse {
            let mut log_base = log_base;
            log_base.stream = true;
            let mut response_headers = response_headers;
            set_debug_headers_for_headers(
                &mut response_headers,
                runtime.effective.expose_debug_headers,
                &route_strategy,
                &route.upstream_id,
            );
            return Ok(streaming_response(
                state,
                upstream_response,
                status,
                response_headers,
                log_base,
                attempt_started,
                limit_admission.take(),
            ));
        }

        let bytes = match upstream_response.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(?error, "reading upstream body failed");
                let status = if error.is_timeout() {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                };
                let error_code = if error.is_timeout() {
                    "upstream_timeout"
                } else {
                    "upstream_error"
                };
                let health = if error.is_timeout() {
                    "down"
                } else {
                    "degraded"
                };
                let _ = storage::record_upstream_health(
                    &state.db,
                    &route.upstream_id,
                    health,
                    Some(error_code),
                )
                .await;
                log_attempt(
                    &state,
                    log_base,
                    status,
                    Some(error_code.to_string()),
                    UsageSnapshot::default(),
                    0,
                    attempt_started,
                )
                .await;
                if can_retry_after_attempt {
                    continue;
                }
                finalize_limit(&state, limit_admission.take(), UsageSnapshot::default()).await;
                return Err(ApiError::gateway(status, "upstream body error", error_code));
            }
        };

        let error_code = status_error_code.map(str::to_string);
        let usage = if status.is_success() {
            parse_unary_usage(&response_headers, &bytes)
        } else {
            UsageSnapshot::default()
        };
        log_attempt(
            &state,
            log_base,
            status,
            error_code,
            usage.clone(),
            bytes.len() as i64,
            attempt_started,
        )
        .await;

        if can_retry_after_attempt && is_retryable_status(status) {
            continue;
        }
        finalize_limit(&state, limit_admission.take(), usage).await;
        let mut response = (status, response_headers, Body::from(bytes)).into_response();
        set_response_request_id(&mut response, &request_id.0);
        set_debug_headers(
            &mut response,
            runtime.effective.expose_debug_headers,
            &route_strategy,
            &route.upstream_id,
        );
        return Ok(response);
    }

    finalize_limit(&state, limit_admission.take(), UsageSnapshot::default()).await;
    Err(ApiError::gateway(
        StatusCode::BAD_GATEWAY,
        format!("No healthy upstream available for model {model}"),
        "upstream_unavailable",
    ))
}

#[derive(Clone)]
struct LogBase {
    request_id: String,
    user_id: String,
    api_key_id: String,
    model_id: Option<String>,
    upstream_id: Option<String>,
    method: String,
    path: String,
    stream: bool,
    started_at: String,
    user_agent: Option<String>,
    input_chars: i64,
    client_metadata_sanitized: Option<String>,
    route_strategy: Option<String>,
    route_decision_json: Option<String>,
}

fn streaming_response(
    state: AppState,
    upstream_response: reqwest::Response,
    status: StatusCode,
    mut response_headers: HeaderMap,
    log_base: LogBase,
    started: Instant,
    limit_admission: Option<storage::LimitAdmission>,
) -> Response {
    set_headers_request_id(&mut response_headers, &log_base.request_id);
    let db = state.db.clone();
    let mut upstream_stream = upstream_response.bytes_stream();
    let stream_state = Arc::new(Mutex::new(StreamingLogState::default()));
    let log_guard = StreamingLogGuard {
        db: db.clone(),
        base: log_base.clone(),
        status,
        started,
        state: stream_state.clone(),
        limit_admission,
    };
    let body_stream = stream! {
        let log_guard = log_guard;

        while let Some(item) = upstream_stream.next().await {
            match item {
                Ok(chunk) => {
                    let completed = {
                        let mut state = stream_state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                        state.output_chars += chunk.len() as i64;
                        state.scanner.push(&chunk);
                        state.scanner.completed()
                    };
                    if completed
                        && let Some(completion) = log_guard.finish(None)
                    {
                        persist_streaming_log(
                            &db,
                            log_base.clone(),
                            completion,
                            started,
                            log_guard.limit_admission.clone(),
                            "completed",
                        ).await;
                    }
                    yield Ok::<Bytes, Infallible>(chunk);
                }
                Err(error) => {
                    tracing::warn!(?error, "upstream SSE stream failed");
                    let mut state = stream_state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    state.error_code = Some("upstream_error".to_string());
                    break;
                }
            }
        }

        if let Some(completion) = log_guard.finish(None) {
            persist_streaming_log(
                &db,
                log_base,
                completion,
                started,
                log_guard.limit_admission.clone(),
                "eof",
            ).await;
        }
    };

    (status, response_headers, Body::from_stream(body_stream)).into_response()
}

#[derive(Default)]
struct StreamingLogState {
    scanner: SseUsageScanner,
    output_chars: i64,
    error_code: Option<String>,
    finalized: bool,
}

struct StreamingLogCompletion {
    status: StatusCode,
    error_code: Option<String>,
    usage: UsageSnapshot,
    output_chars: i64,
}

struct StreamingLogGuard {
    db: sqlx::SqlitePool,
    base: LogBase,
    status: StatusCode,
    started: Instant,
    state: Arc<Mutex<StreamingLogState>>,
    limit_admission: Option<storage::LimitAdmission>,
}

impl StreamingLogGuard {
    fn finish(&self, forced_error_code: Option<&'static str>) -> Option<StreamingLogCompletion> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.finalized {
            return None;
        }
        state.finalized = true;
        let error_code = forced_error_code
            .map(str::to_string)
            .or_else(|| state.error_code.clone());
        let status = stream_log_status(self.status, error_code.as_deref());
        Some(StreamingLogCompletion {
            status,
            error_code,
            usage: state.scanner.snapshot(),
            output_chars: state.output_chars,
        })
    }
}

impl Drop for StreamingLogGuard {
    fn drop(&mut self) {
        let Some(completion) = self.finish(Some("client_disconnected")) else {
            return;
        };
        let db = self.db.clone();
        let admission = self.limit_admission.clone();
        let total_tokens = completion.usage.total_tokens;
        let log = build_log(
            self.base.clone(),
            completion.status,
            completion.error_code,
            completion.usage,
            completion.output_chars,
            self.started,
        );
        let request_id = log.request_id.clone();
        tokio::spawn(async move {
            persist_streaming_log_with_request_id(
                &db,
                log,
                request_id,
                admission,
                total_tokens,
                "disconnected",
            )
            .await;
        });
    }
}

async fn persist_streaming_log(
    db: &sqlx::SqlitePool,
    log_base: LogBase,
    completion: StreamingLogCompletion,
    started: Instant,
    admission: Option<storage::LimitAdmission>,
    reason: &'static str,
) {
    let total_tokens = completion.usage.total_tokens;
    let log = build_log(
        log_base,
        completion.status,
        completion.error_code,
        completion.usage,
        completion.output_chars,
        started,
    );
    let request_id = log.request_id.clone();
    persist_streaming_log_with_request_id(db, log, request_id, admission, total_tokens, reason)
        .await;
}

async fn persist_streaming_log_with_request_id(
    db: &sqlx::SqlitePool,
    log: RequestLogInsert,
    request_id: String,
    admission: Option<storage::LimitAdmission>,
    total_tokens: i64,
    reason: &'static str,
) {
    if let Err(error) = storage::insert_request_log(db, log).await {
        tracing::warn!(?error, %reason, "failed to write streaming request log");
    } else {
        tracing::debug!(%request_id, %reason, "streaming request log written");
    }
    if let Some(admission) = admission
        && let Err(error) = storage::finalize_limit_admission(db, &admission, total_tokens).await
    {
        tracing::warn!(?error, %reason, "failed to finalize streaming limit admission");
    }
}

fn stream_log_status(status: StatusCode, error_code: Option<&str>) -> StatusCode {
    match error_code {
        Some("client_disconnected") => {
            StatusCode::from_u16(499).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
        }
        Some("upstream_error") if status.is_success() => StatusCode::BAD_GATEWAY,
        _ => status,
    }
}

async fn log_attempt(
    state: &AppState,
    log_base: LogBase,
    status: StatusCode,
    error_code: Option<String>,
    usage: UsageSnapshot,
    output_chars: i64,
    started: Instant,
) {
    let log = build_log(log_base, status, error_code, usage, output_chars, started);
    let request_id = log.request_id.clone();
    if let Err(error) = storage::insert_request_log(&state.db, log).await {
        tracing::warn!(?error, "failed to write request log");
    } else {
        tracing::debug!(%request_id, status = status.as_u16(), "request log written");
    }
}

async fn finalize_limit(
    state: &AppState,
    admission: Option<storage::LimitAdmission>,
    usage: UsageSnapshot,
) {
    let Some(admission) = admission else {
        return;
    };
    if let Err(error) =
        storage::finalize_limit_admission(&state.db, &admission, usage.total_tokens).await
    {
        tracing::warn!(?error, "failed to finalize limit admission");
    }
}

fn set_response_request_id(response: &mut Response, request_id: &str) {
    set_headers_request_id(response.headers_mut(), request_id);
}

fn set_headers_request_id(headers: &mut HeaderMap, request_id: &str) {
    if let Ok(value) = HeaderValue::from_str(request_id) {
        headers.insert(request_id_header(), value);
    }
}

fn set_debug_headers(
    response: &mut Response,
    expose_debug_headers: bool,
    route_strategy: &str,
    upstream_id: &str,
) {
    set_debug_headers_for_headers(
        response.headers_mut(),
        expose_debug_headers,
        route_strategy,
        upstream_id,
    );
}

fn set_debug_headers_for_headers(
    headers: &mut HeaderMap,
    expose_debug_headers: bool,
    route_strategy: &str,
    upstream_id: &str,
) {
    if !expose_debug_headers {
        return;
    }
    if let Ok(value) = HeaderValue::from_str(route_strategy) {
        headers.insert(
            HeaderName::from_static("x-codex-gateway-route-strategy"),
            value,
        );
    }
    if let Ok(value) = HeaderValue::from_str(upstream_id) {
        headers.insert(
            HeaderName::from_static("x-codex-gateway-upstream-id"),
            value,
        );
    }
}

async fn read_limited_body(body: Body, limit: i64) -> Result<Bytes, ApiError> {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut stream = body.into_data_stream();
    let mut buffered = BytesMut::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            tracing::warn!(?error, "request body read failed");
            ApiError::gateway(
                StatusCode::BAD_REQUEST,
                "failed to read request body",
                "invalid_request",
            )
        })?;
        if buffered.len().saturating_add(chunk.len()) > limit {
            return Err(ApiError::gateway(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request body exceeds configured maximum",
                "request_body_too_large",
            ));
        }
        buffered.extend_from_slice(&chunk);
    }
    Ok(buffered.freeze())
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

fn build_log(
    base: LogBase,
    status: StatusCode,
    error_code: Option<String>,
    usage: UsageSnapshot,
    output_chars: i64,
    started: Instant,
) -> RequestLogInsert {
    RequestLogInsert {
        request_id: base.request_id,
        user_id: base.user_id,
        api_key_id: base.api_key_id,
        model_id: base.model_id,
        upstream_id: base.upstream_id,
        method: base.method,
        path: base.path,
        status_code: Some(i64::from(status.as_u16())),
        error_code,
        stream: base.stream,
        usage,
        input_chars: base.input_chars,
        output_chars,
        latency_ms: started.elapsed().as_millis() as i64,
        started_at: base.started_at,
        finished_at: storage::now_string(),
        client_ip_hash: None,
        user_agent: base.user_agent,
        client_metadata_sanitized: base.client_metadata_sanitized,
        route_strategy: base.route_strategy,
        route_decision_json: base.route_decision_json,
    }
}

fn routing_key(
    strategy: crate::config::RouteStrategy,
    model: &str,
    request_json: &Value,
    api_key_id: &str,
    request_seed: &str,
) -> String {
    match strategy {
        crate::config::RouteStrategy::Priority => model.to_string(),
        crate::config::RouteStrategy::Weighted => format!("{model}:{request_seed}"),
        crate::config::RouteStrategy::StickyByKey => {
            let sticky_value = request_json
                .get("client_metadata")
                .and_then(Value::as_object)
                .and_then(|metadata| {
                    ["session_id", "thread_id", "turn_id"]
                        .into_iter()
                        .find_map(|key| metadata.get(key).and_then(Value::as_str))
                })
                .filter(|value| !value.is_empty())
                .unwrap_or(api_key_id);
            format!("{model}:{sticky_value}")
        }
    }
}

fn route_strategy_name(strategy: crate::config::RouteStrategy) -> &'static str {
    match strategy {
        crate::config::RouteStrategy::Priority => "priority",
        crate::config::RouteStrategy::Weighted => "weighted",
        crate::config::RouteStrategy::StickyByKey => "sticky_by_key",
    }
}

fn route_decision_json(
    route: &routing::RouteCandidate,
    index: usize,
    candidate_count: usize,
    retries_remaining: i64,
    route_key_hash: &str,
) -> String {
    json!({
        "attempt": index + 1,
        "candidate_count": candidate_count,
        "route_key_hash": route_key_hash,
        "model_id": route.model_id,
        "upstream_id": route.upstream_id,
        "upstream_model_id": route.upstream_model_id,
        "upstream_model_name": route.upstream_model_name,
        "upstream_priority": route.upstream_priority,
        "upstream_model_priority": route.upstream_model_priority,
        "upstream_weight": route.upstream_weight,
        "upstream_model_weight": route.upstream_model_weight,
        "max_retries": route.max_retries.max(0),
        "retries_remaining_after_this_attempt": retries_remaining
    })
    .to_string()
}

fn route_error(model: &str, error: RoutingError) -> ApiError {
    match error {
        RoutingError::ModelNotFound => ApiError::gateway(
            StatusCode::NOT_FOUND,
            format!("Model {model} is not configured"),
            "model_not_found",
        ),
        RoutingError::UpstreamUnavailable => ApiError::gateway(
            StatusCode::BAD_GATEWAY,
            format!("No healthy upstream available for model {model}"),
            "upstream_unavailable",
        ),
        RoutingError::Storage(error) => {
            tracing::error!(?error, "routing storage error");
            ApiError::gateway(
                StatusCode::INTERNAL_SERVER_ERROR,
                "gateway storage error",
                "gateway_internal_error",
            )
        }
    }
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

pub fn sanitize_client_metadata(value: Option<&Value>, app_secret: &str) -> Option<String> {
    let object = value?.as_object()?;
    let mut field_names: Vec<String> = object.keys().cloned().collect();
    field_names.sort();

    let mut sanitized = Map::new();
    sanitized.insert(
        "field_names".to_string(),
        Value::Array(field_names.into_iter().map(Value::String).collect()),
    );

    for key in [
        "session_id",
        "thread_id",
        "turn_id",
        "x-codex-installation-id",
    ] {
        if let Some(raw) = object.get(key).and_then(Value::as_str)
            && !raw.is_empty()
        {
            sanitized.insert(
                format!("{key}_hash"),
                Value::String(crate::auth::hash_api_key(app_secret, raw)),
            );
        }
    }

    Some(Value::Object(sanitized).to_string())
}

pub fn forward_request_headers(
    incoming: &HeaderMap,
    upstream_api_key: &str,
) -> Result<HeaderMap, http::header::InvalidHeaderValue> {
    let mut out = HeaderMap::new();
    for (name, value) in incoming {
        if should_forward_request_header(name) {
            out.insert(name.clone(), value.clone());
        }
    }
    out.insert(
        header::AUTHORIZATION,
        upstream_authorization_header(upstream_api_key)?,
    );
    out.insert(
        HeaderName::from_static("x-codex-gateway"),
        HeaderValue::from_static("codex-gateway/0.1"),
    );
    Ok(out)
}

pub fn upstream_authorization_header(
    upstream_api_key: &str,
) -> Result<HeaderValue, http::header::InvalidHeaderValue> {
    HeaderValue::from_str(&format!("Bearer {upstream_api_key}"))
}

pub fn forward_response_headers(incoming: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in incoming {
        if should_forward_response_header(name) {
            out.insert(name.clone(), value.clone());
        }
    }
    out
}

fn should_forward_request_header(name: &HeaderName) -> bool {
    let name = name.as_str().to_ascii_lowercase();
    if is_hop_by_hop(&name) || is_sensitive_header(&name) {
        return false;
    }
    if matches!(name.as_str(), "host" | "content-length") {
        return false;
    }
    matches!(
        name.as_str(),
        "accept" | "content-type" | "user-agent" | "traceparent" | "tracestate"
    ) || name.starts_with("x-codex-")
        || name.starts_with("x-openai-")
        || name.starts_with("x-responsesapi-")
        || name.starts_with("openai-")
}

fn should_forward_response_header(name: &HeaderName) -> bool {
    let name = name.as_str().to_ascii_lowercase();
    if is_hop_by_hop(&name) || is_sensitive_header(&name) {
        return false;
    }
    !matches!(name.as_str(), "server" | "x-powered-by" | "content-length")
}

pub fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn is_sensitive_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "www-authenticate"
            | "set-cookie"
            | "cookie"
            | "x-api-key"
            | "x-api-key-id"
            | "x-upstream-api-key"
            | "x-openai-api-key"
            | "openai-api-key"
            | "api-key"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sensitive_request_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer cgk_live_a_b"),
        );
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/event-stream"),
        );
        headers.insert(
            HeaderName::from_static("connection"),
            HeaderValue::from_static("close"),
        );
        headers.insert(
            HeaderName::from_static("x-codex-turn-state"),
            HeaderValue::from_static("state"),
        );
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("secret"),
        );
        headers.insert(
            HeaderName::from_static("cookie"),
            HeaderValue::from_static("secret"),
        );

        let forwarded = forward_request_headers(&headers, "sk-upstream").unwrap();
        assert_eq!(
            forwarded.get(header::AUTHORIZATION).unwrap(),
            "Bearer sk-upstream"
        );
        assert_eq!(forwarded.get(header::ACCEPT).unwrap(), "text/event-stream");
        assert_eq!(forwarded.get("x-codex-turn-state").unwrap(), "state");
        assert!(!forwarded.contains_key("connection"));
        assert!(!forwarded.contains_key("cookie"));
        assert!(!forwarded.contains_key("x-api-key"));
    }

    #[test]
    fn rejects_invalid_upstream_authorization_without_panicking() {
        let headers = HeaderMap::new();
        assert!(forward_request_headers(&headers, "sk-good").is_ok());
        assert!(forward_request_headers(&headers, "sk-bad\r\nx-leak: yes").is_err());
    }

    #[test]
    fn strips_sensitive_response_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        headers.insert(
            HeaderName::from_static("transfer-encoding"),
            HeaderValue::from_static("chunked"),
        );
        headers.insert(
            HeaderName::from_static("server"),
            HeaderValue::from_static("upstream"),
        );
        headers.insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer upstream"),
        );
        headers.insert(header::SET_COOKIE, HeaderValue::from_static("sid=secret"));
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("secret"),
        );

        let forwarded = forward_response_headers(&headers);
        assert_eq!(
            forwarded.get(header::CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );
        assert!(!forwarded.contains_key("transfer-encoding"));
        assert!(!forwarded.contains_key("server"));
        assert!(!forwarded.contains_key(header::WWW_AUTHENTICATE));
        assert!(!forwarded.contains_key(header::SET_COOKIE));
        assert!(!forwarded.contains_key("x-api-key"));
    }

    #[test]
    fn sanitizes_client_metadata_without_raw_values() {
        let metadata = json!({
            "session_id": "sess-secret",
            "thread_id": "thread-secret",
            "x-codex-turn-metadata": "raw secret",
            "other": {"nested": "ignored"}
        });
        let sanitized = sanitize_client_metadata(Some(&metadata), "app-secret").unwrap();
        assert!(sanitized.contains("field_names"));
        assert!(sanitized.contains("session_id_hash"));
        assert!(sanitized.contains("thread_id_hash"));
        assert!(!sanitized.contains("sess-secret"));
        assert!(!sanitized.contains("thread-secret"));
        assert!(!sanitized.contains("raw secret"));
    }
}
