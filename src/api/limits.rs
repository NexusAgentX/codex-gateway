use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch},
};
use serde::{Deserialize, Deserializer};
use serde_json::json;

use crate::{AppState, storage};

use super::{
    ApiError,
    auth::{Administrator, AdministratorJson, admin_audit, authenticate},
    contracts::{AdminLimitResponse, LimitPolicyResponse, LimitSubjectResponse, UserLimitResponse},
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum LimitPatchRequest {
    #[default]
    Missing,
    Inherit,
    Clear,
    Set(i64),
}

impl From<LimitPatchRequest> for storage::LimitPatchValue {
    fn from(value: LimitPatchRequest) -> Self {
        match value {
            LimitPatchRequest::Missing => Self::Missing,
            LimitPatchRequest::Inherit => Self::Inherit,
            LimitPatchRequest::Clear => Self::Clear,
            LimitPatchRequest::Set(value) => Self::Set(value),
        }
    }
}

impl<'de> Deserialize<'de> for LimitPatchRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Null => Ok(Self::Clear),
            serde_json::Value::Number(number) => number
                .as_i64()
                .map(Self::Set)
                .ok_or_else(|| serde::de::Error::custom("limit value must be an integer")),
            serde_json::Value::Object(object) => {
                let mode = object
                    .get("mode")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| serde::de::Error::custom("limit mode is required"))?;
                match mode {
                    "inherit" => Ok(Self::Inherit),
                    "unlimited" => Ok(Self::Clear),
                    "limited" => object
                        .get("value")
                        .and_then(serde_json::Value::as_i64)
                        .map(Self::Set)
                        .ok_or_else(|| {
                            serde::de::Error::custom("limited mode requires integer value")
                        }),
                    _ => Err(serde::de::Error::custom(
                        "limit mode must be inherit, limited, or unlimited",
                    )),
                }
            }
            _ => Err(serde::de::Error::custom(
                "limit value must be an integer, null, or mode object",
            )),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct LimitPolicyPatchRequest {
    #[serde(default)]
    request_quota: LimitPatchRequest,
    request_window_seconds: Option<i64>,
    #[serde(default)]
    token_quota: LimitPatchRequest,
    token_window_seconds: Option<i64>,
    #[serde(default)]
    rate_limit_requests: LimitPatchRequest,
    rate_limit_window_seconds: Option<i64>,
    #[serde(default)]
    concurrency_limit: LimitPatchRequest,
}

impl From<LimitPolicyPatchRequest> for storage::LimitPolicyPatch {
    fn from(value: LimitPolicyPatchRequest) -> Self {
        Self {
            request_quota: value.request_quota.into(),
            request_window_seconds: value.request_window_seconds,
            token_quota: value.token_quota.into(),
            token_window_seconds: value.token_window_seconds,
            rate_limit_requests: value.rate_limit_requests.into(),
            rate_limit_window_seconds: value.rate_limit_window_seconds,
            concurrency_limit: value.concurrency_limit.into(),
        }
    }
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/limits", get(my_limits))
        .route(
            "/api/admin/users/{id}/limits",
            get(admin_user_limits).patch(admin_update_user_limits),
        )
        .route(
            "/api/admin/api-keys/{id}/limits",
            get(admin_api_key_limits).patch(admin_update_api_key_limits),
        )
        .route("/api/admin/limits", get(admin_limits))
        .route(
            "/api/admin/limits/system",
            patch(admin_update_system_limits),
        )
}

async fn my_limits(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<UserLimitResponse>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    let current_key_id =
        (!user.api_key_id.starts_with("panel:")).then_some(user.api_key_id.as_str());
    Ok(Json(
        storage::user_limit_state(&state.db, &user.user_id, current_key_id)
            .await?
            .into(),
    ))
}

async fn admin_user_limits(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Administrator(_admin): Administrator,
) -> Result<Json<UserLimitResponse>, ApiError> {
    storage::get_user(&state.db, &id)
        .await?
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "user not found", "not_found"))?;
    Ok(Json(
        storage::user_limit_state(&state.db, &id, None)
            .await?
            .into(),
    ))
}

async fn admin_update_user_limits(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AdministratorJson(admin, input): AdministratorJson<LimitPolicyPatchRequest>,
) -> Result<Json<UserLimitResponse>, ApiError> {
    validate_limit_policy(&input)?;
    let input: storage::LimitPolicyPatch = input.into();
    storage::get_user(&state.db, &id)
        .await?
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "user not found", "not_found"))?;
    let user_id = id.clone();
    storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let policy = storage::upsert_limit_policy_conn(conn, "user", &user_id, &input).await?;
            let audit = admin_audit(
                &admin,
                "update_user_limits",
                "limit_policy",
                Some(user_id),
                json!({ "scope": "user", "policy": policy }),
            );
            Ok(((), audit))
        })
    })
    .await?;
    Ok(Json(
        storage::user_limit_state(&state.db, &id, None)
            .await?
            .into(),
    ))
}

async fn admin_api_key_limits(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Administrator(_admin): Administrator,
) -> Result<Json<LimitSubjectResponse>, ApiError> {
    let key = storage::get_api_key(&state.db, &id).await?.ok_or_else(|| {
        ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found")
    })?;
    let state = storage::user_limit_state(&state.db, &key.user_id, Some(&id)).await?;
    state
        .current_key
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found"))
        .map(Into::into)
        .map(Json)
}

async fn admin_update_api_key_limits(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AdministratorJson(admin, input): AdministratorJson<LimitPolicyPatchRequest>,
) -> Result<Json<LimitSubjectResponse>, ApiError> {
    validate_limit_policy(&input)?;
    let input: storage::LimitPolicyPatch = input.into();
    let key = storage::get_api_key(&state.db, &id).await?.ok_or_else(|| {
        ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found")
    })?;
    let key_id = id.clone();
    let user_id = key.user_id.clone();
    storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let policy =
                storage::upsert_limit_policy_conn(conn, "api_key", &key_id, &input).await?;
            let audit = admin_audit(
                &admin,
                "update_api_key_limits",
                "limit_policy",
                Some(key_id),
                json!({ "scope": "api_key", "user_id": user_id, "policy": policy }),
            );
            Ok(((), audit))
        })
    })
    .await?;
    let state = storage::user_limit_state(&state.db, &key.user_id, Some(&id)).await?;
    state
        .current_key
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found"))
        .map(Into::into)
        .map(Json)
}

async fn admin_limits(
    State(state): State<AppState>,
    Administrator(_admin): Administrator,
) -> Result<Json<AdminLimitResponse>, ApiError> {
    Ok(Json(storage::admin_limit_state(&state.db).await?.into()))
}

async fn admin_update_system_limits(
    State(state): State<AppState>,
    AdministratorJson(admin, input): AdministratorJson<LimitPolicyPatchRequest>,
) -> Result<Json<LimitPolicyResponse>, ApiError> {
    validate_limit_policy(&input)?;
    let input: storage::LimitPolicyPatch = input.into();
    let policy = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let policy = storage::upsert_limit_policy_conn(conn, "system", "", &input).await?;
            let audit = admin_audit(
                &admin,
                "update_system_limits",
                "limit_policy",
                None,
                json!({ "scope": "system", "policy": policy }),
            );
            Ok((policy, audit))
        })
    })
    .await?;
    Ok(Json(policy.into()))
}

fn validate_limit_policy(input: &LimitPolicyPatchRequest) -> Result<(), ApiError> {
    for (name, value) in [
        ("request_quota", &input.request_quota),
        ("token_quota", &input.token_quota),
        ("rate_limit_requests", &input.rate_limit_requests),
        ("concurrency_limit", &input.concurrency_limit),
    ] {
        if matches!(value, LimitPatchRequest::Set(value) if *value < 0) {
            return Err(ApiError::bad_request(
                format!("{name} must be zero or greater"),
                "invalid_request",
            ));
        }
    }
    for (name, value) in [
        ("request_window_seconds", input.request_window_seconds),
        ("token_window_seconds", input.token_window_seconds),
        ("rate_limit_window_seconds", input.rate_limit_window_seconds),
    ] {
        if value.is_some_and(|value| value <= 0) {
            return Err(ApiError::bad_request(
                format!("{name} must be at least 1"),
                "invalid_request",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn limit_patch_request_preserves_absent_null_value_and_mode_forms() {
        let missing: LimitPolicyPatchRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(missing.request_quota, LimitPatchRequest::Missing);

        for (json, expected) in [
            (json!(null), LimitPatchRequest::Clear),
            (json!(100), LimitPatchRequest::Set(100)),
            (json!({ "mode": "inherit" }), LimitPatchRequest::Inherit),
            (json!({ "mode": "unlimited" }), LimitPatchRequest::Clear),
            (
                json!({ "mode": "limited", "value": 200 }),
                LimitPatchRequest::Set(200),
            ),
        ] {
            let request: LimitPolicyPatchRequest =
                serde_json::from_value(json!({ "request_quota": json })).unwrap();
            assert_eq!(request.request_quota, expected);
        }
    }

    #[test]
    fn limit_patch_request_rejects_invalid_forms_with_compatible_messages() {
        for (json, expected) in [
            (json!(1.5), "limit value must be an integer"),
            (
                json!("100"),
                "limit value must be an integer, null, or mode object",
            ),
            (json!({}), "limit mode is required"),
            (
                json!({ "mode": "limited" }),
                "limited mode requires integer value",
            ),
            (
                json!({ "mode": "other" }),
                "limit mode must be inherit, limited, or unlimited",
            ),
        ] {
            let error =
                serde_json::from_value::<LimitPolicyPatchRequest>(json!({ "request_quota": json }))
                    .unwrap_err();
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn limit_patch_request_converts_explicitly_to_storage_command() {
        for (request, expected) in [
            (
                LimitPatchRequest::Missing,
                storage::LimitPatchValue::Missing,
            ),
            (
                LimitPatchRequest::Inherit,
                storage::LimitPatchValue::Inherit,
            ),
            (LimitPatchRequest::Clear, storage::LimitPatchValue::Clear),
            (
                LimitPatchRequest::Set(42),
                storage::LimitPatchValue::Set(42),
            ),
        ] {
            let actual: storage::LimitPatchValue = request.into();
            assert_eq!(actual, expected);
        }
    }
}
