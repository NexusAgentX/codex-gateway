use std::{convert::Infallible, time::Instant};

use async_stream::stream;
use axum::{
    Json,
    body::Body,
    extract::{OriginalUri, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures_util::StreamExt;
use serde_json::{Map, Value, json};

use crate::{
    AppState,
    api::{self, ApiError},
    routing::{self, RoutingError},
    storage::{self, RequestLogInsert},
    upstream,
    usage::{self, SseUsageScanner, UsageSnapshot},
};

pub async fn models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    api::authenticate(&state, &headers).await?;
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
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let user = api::authenticate(&state, &headers).await?;
    let path = uri.path().to_string();
    let canonical_path = upstream::canonical_proxy_path(&path).ok_or_else(|| {
        ApiError::gateway(StatusCode::NOT_FOUND, "unsupported proxy path", "not_found")
    })?;

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

    let candidates = routing::route_candidates(&state.db, &model)
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
    let last_index = candidates.len().saturating_sub(1);
    for (index, route) in candidates.into_iter().enumerate() {
        let attempt_started_at = storage::now_string();
        let attempt_started = Instant::now();
        let has_next = index < last_index;
        let mut attempt_json = request_json.clone();
        if route.upstream_model_name != model
            && let Some(object) = attempt_json.as_object_mut()
        {
            object.insert(
                "model".to_string(),
                Value::String(route.upstream_model_name.clone()),
            );
        }

        let log_base = LogBase {
            request_id: uuid::Uuid::new_v4().to_string(),
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
                if can_retry && has_next {
                    continue;
                }
                return Err(ApiError::gateway(
                    status,
                    "invalid upstream URL",
                    "upstream_unavailable",
                ));
            }
        };

        let upstream_response = state
            .http
            .request(reqwest_method.clone(), url)
            .headers(forward_request_headers(&headers, &route.upstream_api_key))
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
                let _ =
                    storage::update_upstream_health(&state.db, &route.upstream_id, health).await;
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
                if can_retry && has_next {
                    continue;
                }
                return Err(ApiError::gateway(
                    status,
                    "upstream request failed",
                    error_code,
                ));
            }
        };

        let status = StatusCode::from_u16(upstream_response.status().as_u16())
            .unwrap_or(StatusCode::BAD_GATEWAY);
        let _ = storage::update_upstream_health(
            &state.db,
            &route.upstream_id,
            health_for_status(status),
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
            return Ok(streaming_response(
                state,
                upstream_response,
                status,
                response_headers,
                log_base,
                attempt_started,
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
                let _ = storage::update_upstream_health(&state.db, &route.upstream_id, "degraded")
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
                if can_retry && has_next {
                    continue;
                }
                return Err(ApiError::gateway(status, "upstream body error", error_code));
            }
        };

        let error_code = error_code_for_status(status).map(str::to_string);
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
            usage,
            bytes.len() as i64,
            attempt_started,
        )
        .await;

        if can_retry && has_next && is_retryable_status(status) {
            continue;
        }
        return Ok((status, response_headers, Body::from(bytes)).into_response());
    }

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
}

fn streaming_response(
    state: AppState,
    upstream_response: reqwest::Response,
    status: StatusCode,
    response_headers: HeaderMap,
    log_base: LogBase,
    started: Instant,
) -> Response {
    let db = state.db.clone();
    let mut upstream_stream = upstream_response.bytes_stream();
    let body_stream = stream! {
        let mut scanner = SseUsageScanner::default();
        let mut output_chars = 0_i64;
        let mut error_code: Option<String> = None;

        while let Some(item) = upstream_stream.next().await {
            match item {
                Ok(chunk) => {
                    output_chars += chunk.len() as i64;
                    scanner.push(&chunk);
                    yield Ok::<Bytes, Infallible>(chunk);
                }
                Err(error) => {
                    tracing::warn!(?error, "upstream SSE stream failed");
                    error_code = Some("upstream_error".to_string());
                    break;
                }
            }
        }

        let log = build_log(log_base, status, error_code, scanner.snapshot(), output_chars, started);
        if let Err(error) = storage::insert_request_log(&db, log).await {
            tracing::warn!(?error, "failed to write streaming request log");
        }
    };

    (status, response_headers, Body::from_stream(body_stream)).into_response()
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
    if let Err(error) = storage::insert_request_log(&state.db, log).await {
        tracing::warn!(?error, "failed to write request log");
    }
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
    }
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

pub fn forward_request_headers(incoming: &HeaderMap, upstream_api_key: &str) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in incoming {
        if should_forward_request_header(name) {
            out.insert(name.clone(), value.clone());
        }
    }
    out.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {upstream_api_key}"))
            .expect("upstream API key must be header-compatible"),
    );
    out.insert(
        HeaderName::from_static("x-codex-gateway"),
        HeaderValue::from_static("codex-gateway/0.1"),
    );
    out
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

        let forwarded = forward_request_headers(&headers, "sk-upstream");
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
