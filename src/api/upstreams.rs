use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch, post},
};
use serde::{Deserialize, Deserializer};
use serde_json::json;

use crate::{AppState, storage};

use super::{
    ApiError,
    auth::{Administrator, AdministratorJson, admin_audit},
    contracts::UpstreamResponse,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum TimeoutPatchRequest {
    #[default]
    Missing,
    Default,
    Explicit(i64),
}

impl TimeoutPatchRequest {
    fn explicit_value(&self) -> Option<i64> {
        match self {
            Self::Explicit(value) => Some(*value),
            Self::Missing | Self::Default => None,
        }
    }

    fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

impl From<TimeoutPatchRequest> for storage::TimeoutPatchValue {
    fn from(value: TimeoutPatchRequest) -> Self {
        match value {
            TimeoutPatchRequest::Missing => Self::Missing,
            TimeoutPatchRequest::Default => Self::Default,
            TimeoutPatchRequest::Explicit(value) => Self::Explicit(value),
        }
    }
}

impl<'de> Deserialize<'de> for TimeoutPatchRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Null => Ok(Self::Default),
            serde_json::Value::Number(number) => number
                .as_i64()
                .map(Self::Explicit)
                .ok_or_else(|| serde::de::Error::custom("timeout_ms must be an integer")),
            serde_json::Value::Object(object) => {
                let mode = object
                    .get("mode")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| serde::de::Error::custom("timeout mode is required"))?;
                match mode {
                    "default" | "inherit" => Ok(Self::Default),
                    "explicit" => object
                        .get("value")
                        .and_then(serde_json::Value::as_i64)
                        .map(Self::Explicit)
                        .ok_or_else(|| {
                            serde::de::Error::custom("explicit timeout mode requires integer value")
                        }),
                    _ => Err(serde::de::Error::custom(
                        "timeout mode must be default, inherit, or explicit",
                    )),
                }
            }
            _ => Err(serde::de::Error::custom(
                "timeout_ms must be an integer, null, or mode object",
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct UpsertUpstreamRequest {
    name: String,
    base_url: String,
    api_key: String,
    enabled: Option<bool>,
    priority: Option<i64>,
    weight: Option<i64>,
    #[serde(default)]
    timeout_ms: TimeoutPatchRequest,
    max_retries: Option<i64>,
    health_check_path: Option<String>,
}

impl From<UpsertUpstreamRequest> for storage::UpsertUpstream {
    fn from(value: UpsertUpstreamRequest) -> Self {
        Self {
            name: value.name,
            base_url: value.base_url,
            api_key: value.api_key,
            enabled: value.enabled,
            priority: value.priority,
            weight: value.weight,
            timeout_ms: value.timeout_ms.into(),
            max_retries: value.max_retries,
            health_check_path: value.health_check_path,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct UpdateUpstreamRequest {
    name: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
    enabled: Option<bool>,
    priority: Option<i64>,
    weight: Option<i64>,
    #[serde(default)]
    timeout_ms: TimeoutPatchRequest,
    max_retries: Option<i64>,
    health_check_path: Option<String>,
}

impl From<UpdateUpstreamRequest> for storage::UpdateUpstream {
    fn from(value: UpdateUpstreamRequest) -> Self {
        Self {
            name: value.name,
            base_url: value.base_url,
            api_key: value.api_key,
            enabled: value.enabled,
            priority: value.priority,
            weight: value.weight,
            timeout_ms: value.timeout_ms.into(),
            max_retries: value.max_retries,
            health_check_path: value.health_check_path,
        }
    }
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/admin/upstreams",
            get(admin_upstreams).post(admin_create_upstream),
        )
        .route("/api/admin/upstreams/{id}", patch(admin_update_upstream))
        .route(
            "/api/admin/upstreams/{id}/disable",
            post(admin_disable_upstream),
        )
        .route(
            "/api/admin/upstreams/{id}/health",
            post(admin_check_upstream_health),
        )
}

async fn admin_upstreams(
    State(state): State<AppState>,
    Administrator(_admin): Administrator,
) -> Result<Json<Vec<UpstreamResponse>>, ApiError> {
    let default_timeout = storage::runtime_config(&state.db, &state.config)
        .await?
        .effective
        .default_request_timeout_ms;
    let upstreams = storage::list_upstreams(&state.db)
        .await?
        .into_iter()
        .map(|upstream| UpstreamResponse::try_from_record(upstream, default_timeout))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(upstreams))
}

async fn admin_create_upstream(
    State(state): State<AppState>,
    AdministratorJson(admin, input): AdministratorJson<UpsertUpstreamRequest>,
) -> Result<Json<UpstreamResponse>, ApiError> {
    validate_upsert_upstream(&input)?;
    let input: storage::UpsertUpstream = input.into();
    let default_timeout = storage::runtime_config(&state.db, &state.config)
        .await?
        .effective
        .default_request_timeout_ms;
    let app_secret = state.config.app_secret.clone();
    let secret_key_version = state.config.secret_key_version;
    let upstream = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let upstream =
                storage::create_upstream_conn(conn, &app_secret, secret_key_version, &input)
                    .await?;
            let audit = admin_audit(
                &admin,
                "create_upstream",
                "upstream",
                Some(upstream.id.clone()),
                json!({
                    "name": upstream.name,
                    "base_url": upstream.base_url,
                    "secret_version": upstream.api_key_secret_version
                }),
            );
            Ok((upstream, audit))
        })
    })
    .await?;
    Ok(Json(UpstreamResponse::try_from_record(
        upstream,
        default_timeout,
    )?))
}

async fn admin_update_upstream(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AdministratorJson(admin, input): AdministratorJson<UpdateUpstreamRequest>,
) -> Result<Json<UpstreamResponse>, ApiError> {
    validate_update_upstream(&input)?;
    let input: storage::UpdateUpstream = input.into();
    let default_timeout = storage::runtime_config(&state.db, &state.config)
        .await?
        .effective
        .default_request_timeout_ms;
    let app_secret = state.config.app_secret.clone();
    let secret_key_version = state.config.secret_key_version;
    let upstream = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let upstream =
                storage::update_upstream_conn(conn, &app_secret, secret_key_version, &id, &input)
                    .await?
                    .ok_or_else(|| {
                        ApiError::gateway(StatusCode::NOT_FOUND, "upstream not found", "not_found")
                    })?;
            let audit = admin_audit(
                &admin,
                "update_upstream",
                "upstream",
                Some(id),
                json!({
                    "name_changed": input.name.is_some(),
                    "base_url_changed": input.base_url.is_some(),
                    "api_key_rotated": input.api_key.is_some(),
                    "enabled_changed": input.enabled.is_some(),
                    "priority_changed": input.priority.is_some(),
                    "weight_changed": input.weight.is_some(),
                    "timeout_ms_changed": !input.timeout_ms.is_missing(),
                    "max_retries_changed": input.max_retries.is_some(),
                    "health_check_path_changed": input.health_check_path.is_some(),
                    "secret_version": upstream.api_key_secret_version
                }),
            );
            Ok((upstream, audit))
        })
    })
    .await?;
    Ok(Json(UpstreamResponse::try_from_record(
        upstream,
        default_timeout,
    )?))
}

async fn admin_disable_upstream(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Administrator(admin): Administrator,
) -> Result<Json<UpstreamResponse>, ApiError> {
    let default_timeout = storage::runtime_config(&state.db, &state.config)
        .await?
        .effective
        .default_request_timeout_ms;
    let input = storage::UpdateUpstream {
        name: None,
        base_url: None,
        api_key: None,
        enabled: Some(false),
        priority: None,
        weight: None,
        timeout_ms: storage::TimeoutPatchValue::Missing,
        max_retries: None,
        health_check_path: None,
    };
    let app_secret = state.config.app_secret.clone();
    let secret_key_version = state.config.secret_key_version;
    let upstream = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let upstream =
                storage::update_upstream_conn(conn, &app_secret, secret_key_version, &id, &input)
                    .await?
                    .ok_or_else(|| {
                        ApiError::gateway(StatusCode::NOT_FOUND, "upstream not found", "not_found")
                    })?;
            let audit = admin_audit(
                &admin,
                "disable_upstream",
                "upstream",
                Some(id),
                json!({ "enabled": false }),
            );
            Ok((upstream, audit))
        })
    })
    .await?;
    Ok(Json(UpstreamResponse::try_from_record(
        upstream,
        default_timeout,
    )?))
}

async fn admin_check_upstream_health(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Administrator(admin): Administrator,
) -> Result<Json<serde_json::Value>, ApiError> {
    let upstream = storage::get_upstream(&state.db, &id)
        .await?
        .ok_or_else(|| {
            ApiError::gateway(StatusCode::NOT_FOUND, "upstream not found", "not_found")
        })?;
    let default_timeout = storage::runtime_config(&state.db, &state.config)
        .await?
        .effective
        .default_request_timeout_ms;
    let upstream = crate::upstream::upstream_with_effective_timeout(upstream, default_timeout);
    let (status, error_sample) =
        crate::upstream::probe_upstream_health(&state.http, &state.config.app_secret, &upstream)
            .await
            .map_err(|error| {
                tracing::warn!(?error, upstream_id = %id, "upstream health check failed");
                ApiError::gateway(
                    StatusCode::BAD_GATEWAY,
                    "upstream health check failed",
                    "upstream_unavailable",
                )
            })?;
    let response_id = id.clone();
    let status = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            storage::record_upstream_health_conn(conn, &id, &status, error_sample).await?;
            let audit = admin_audit(
                &admin,
                "check_upstream_health",
                "upstream",
                Some(id),
                json!({ "health": status }),
            );
            Ok((status, audit))
        })
    })
    .await
    .map_err(|error| {
        tracing::warn!(?error, upstream_id = %response_id, "upstream health check failed");
        ApiError::gateway(
            StatusCode::BAD_GATEWAY,
            "upstream health check failed",
            "upstream_unavailable",
        )
    })?;
    let id = response_id;
    Ok(Json(json!({ "id": id, "health": status })))
}

fn validate_upsert_upstream(input: &UpsertUpstreamRequest) -> Result<(), ApiError> {
    validate_required("name", &input.name)?;
    validate_url(&input.base_url)?;
    validate_upstream_api_key(&input.api_key)?;
    validate_route_numbers(
        input.priority,
        input.weight,
        input.timeout_ms.explicit_value(),
        input.max_retries,
    )?;
    validate_health_path(input.health_check_path.as_deref())?;
    Ok(())
}

fn validate_update_upstream(input: &UpdateUpstreamRequest) -> Result<(), ApiError> {
    if input.name.is_none()
        && input.base_url.is_none()
        && input.api_key.is_none()
        && input.enabled.is_none()
        && input.priority.is_none()
        && input.weight.is_none()
        && input.timeout_ms.is_missing()
        && input.max_retries.is_none()
        && input.health_check_path.is_none()
    {
        return Err(ApiError::bad_request(
            "no upstream fields supplied",
            "invalid_request",
        ));
    }
    if let Some(name) = &input.name {
        validate_required("name", name)?;
    }
    if let Some(base_url) = &input.base_url {
        validate_url(base_url)?;
    }
    if let Some(api_key) = &input.api_key {
        validate_upstream_api_key(api_key)?;
    }
    validate_route_numbers(
        input.priority,
        input.weight,
        input.timeout_ms.explicit_value(),
        input.max_retries,
    )?;
    validate_health_path(input.health_check_path.as_deref())?;
    Ok(())
}

fn validate_required(field: &str, value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::bad_request(
            format!("{field} must not be empty"),
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_url(value: &str) -> Result<(), ApiError> {
    validate_required("base_url", value)?;
    let parsed = url::Url::parse(value)
        .map_err(|_| ApiError::bad_request("base_url must be a valid URL", "invalid_request"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ApiError::bad_request(
            "base_url must use http or https",
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_upstream_api_key(value: &str) -> Result<(), ApiError> {
    validate_required("api_key", value)?;
    crate::upstream::headers::authorization_header(value).map_err(|_| {
        ApiError::bad_request(
            "api_key cannot be used in an Authorization header",
            "invalid_request",
        )
    })?;
    Ok(())
}

fn validate_route_numbers(
    priority: Option<i64>,
    weight: Option<i64>,
    timeout_ms: Option<i64>,
    max_retries: Option<i64>,
) -> Result<(), ApiError> {
    if priority.is_some_and(|value| value < 0) {
        return Err(ApiError::bad_request(
            "priority must be zero or greater",
            "invalid_request",
        ));
    }
    if weight.is_some_and(|value| value < 1) {
        return Err(ApiError::bad_request(
            "weight must be at least 1",
            "invalid_request",
        ));
    }
    if timeout_ms.is_some_and(|value| value < 1) {
        return Err(ApiError::bad_request(
            "timeout_ms must be at least 1",
            "invalid_request",
        ));
    }
    if max_retries.is_some_and(|value| value < 0) {
        return Err(ApiError::bad_request(
            "max_retries must be zero or greater",
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_health_path(value: Option<&str>) -> Result<(), ApiError> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_required("health_check_path", value)?;
    if !value.starts_with('/') {
        return Err(ApiError::bad_request(
            "health_check_path must start with /",
            "invalid_request",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn timeout_patch_request_preserves_absent_null_value_and_mode_forms() {
        let missing: UpdateUpstreamRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(missing.timeout_ms, TimeoutPatchRequest::Missing);

        for (json, expected) in [
            (json!(null), TimeoutPatchRequest::Default),
            (json!(25_000), TimeoutPatchRequest::Explicit(25_000)),
            (json!({ "mode": "default" }), TimeoutPatchRequest::Default),
            (json!({ "mode": "inherit" }), TimeoutPatchRequest::Default),
            (
                json!({ "mode": "explicit", "value": 30_000 }),
                TimeoutPatchRequest::Explicit(30_000),
            ),
        ] {
            let request: UpdateUpstreamRequest =
                serde_json::from_value(json!({ "timeout_ms": json })).unwrap();
            assert_eq!(request.timeout_ms, expected);
        }
    }

    #[test]
    fn timeout_patch_request_rejects_invalid_forms_with_compatible_messages() {
        for (json, expected) in [
            (json!(1.5), "timeout_ms must be an integer"),
            (
                json!("120000"),
                "timeout_ms must be an integer, null, or mode object",
            ),
            (json!({}), "timeout mode is required"),
            (
                json!({ "mode": "explicit" }),
                "explicit timeout mode requires integer value",
            ),
            (
                json!({ "mode": "other" }),
                "timeout mode must be default, inherit, or explicit",
            ),
        ] {
            let error =
                serde_json::from_value::<UpdateUpstreamRequest>(json!({ "timeout_ms": json }))
                    .unwrap_err();
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn timeout_patch_request_converts_explicitly_to_storage_command() {
        for (request, expected) in [
            (
                TimeoutPatchRequest::Missing,
                storage::TimeoutPatchValue::Missing,
            ),
            (
                TimeoutPatchRequest::Default,
                storage::TimeoutPatchValue::Default,
            ),
            (
                TimeoutPatchRequest::Explicit(42),
                storage::TimeoutPatchValue::Explicit(42),
            ),
        ] {
            let actual: storage::TimeoutPatchValue = request.into();
            assert_eq!(actual, expected);
        }
    }
}
