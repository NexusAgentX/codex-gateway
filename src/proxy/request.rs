use axum::{
    body::Body,
    extract::Request,
    http::{HeaderMap, Method, StatusCode, Uri},
};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use serde_json::{Map, Value};

use crate::{
    AppState, RequestId, auth, config::RuntimeConfig, http_error::ApiError, storage, upstream,
};

pub(super) struct PreparedRequest {
    pub(super) request_id: String,
    pub(super) user: auth::AuthenticatedUser,
    pub(super) runtime: RuntimeConfig,
    pub(super) method: Method,
    pub(super) reqwest_method: reqwest::Method,
    pub(super) headers: HeaderMap,
    pub(super) path: String,
    pub(super) canonical_path: &'static str,
    pub(super) body: Bytes,
    pub(super) json: Value,
    pub(super) model: String,
    pub(super) stream_requested: bool,
    pub(super) client_metadata_sanitized: Option<String>,
    pub(super) can_retry: bool,
}

pub(super) async fn prepare(
    state: &AppState,
    request_id: RequestId,
    uri: Uri,
    request: Request,
) -> Result<PreparedRequest, ApiError> {
    let runtime = storage::runtime_config(&state.db, &state.config)
        .await?
        .effective;
    let (parts, body) = request.into_parts();
    let method = parts.method;
    let headers = parts.headers;
    let user = super::authenticate_api_key(state, &headers).await?;
    let path = uri.path().to_string();
    let canonical_path = upstream::canonical_proxy_path(&path).ok_or_else(|| {
        ApiError::gateway(StatusCode::NOT_FOUND, "unsupported proxy path", "not_found")
    })?;
    let body = read_limited_body(body, runtime.max_request_body_bytes).await?;
    let json: Value = serde_json::from_slice(&body).map_err(|_| {
        ApiError::gateway(
            StatusCode::BAD_REQUEST,
            "request body must be JSON",
            "invalid_request",
        )
    })?;
    let model = json
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ApiError::gateway(StatusCode::BAD_REQUEST, "missing model", "model_not_found")
        })?
        .to_string();
    let stream_requested = json.get("stream").and_then(Value::as_bool).unwrap_or(false)
        && canonical_path == "/responses";
    let client_metadata_sanitized =
        sanitize_client_metadata(json.get("client_metadata"), &state.config.app_secret);
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes()).map_err(|_| {
        ApiError::gateway(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unsupported HTTP method",
            "gateway_internal_error",
        )
    })?;

    Ok(PreparedRequest {
        request_id: request_id.0,
        user,
        runtime,
        method,
        reqwest_method,
        headers,
        path,
        canonical_path,
        body,
        json,
        model,
        stream_requested,
        client_metadata_sanitized,
        can_retry: !stream_requested,
    })
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

pub(super) fn sanitize_client_metadata(value: Option<&Value>, app_secret: &str) -> Option<String> {
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
