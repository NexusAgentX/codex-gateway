use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch},
};
use serde_json::json;

use crate::{AppState, storage};

use super::{
    ApiError,
    auth::{Administrator, AdministratorJson, admin_audit, authenticate},
};

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
) -> Result<Json<storage::UserLimitState>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    let current_key_id =
        (!user.api_key_id.starts_with("panel:")).then_some(user.api_key_id.as_str());
    Ok(Json(
        storage::user_limit_state(&state.db, &user.user_id, current_key_id).await?,
    ))
}

async fn admin_user_limits(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Administrator(_admin): Administrator,
) -> Result<Json<storage::UserLimitState>, ApiError> {
    storage::get_user(&state.db, &id)
        .await?
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "user not found", "not_found"))?;
    Ok(Json(storage::user_limit_state(&state.db, &id, None).await?))
}

async fn admin_update_user_limits(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AdministratorJson(admin, input): AdministratorJson<storage::LimitPolicyPatch>,
) -> Result<Json<storage::UserLimitState>, ApiError> {
    validate_limit_policy(&input)?;
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
    Ok(Json(storage::user_limit_state(&state.db, &id, None).await?))
}

async fn admin_api_key_limits(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Administrator(_admin): Administrator,
) -> Result<Json<storage::LimitSubjectState>, ApiError> {
    let key = storage::get_api_key(&state.db, &id).await?.ok_or_else(|| {
        ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found")
    })?;
    let state = storage::user_limit_state(&state.db, &key.user_id, Some(&id)).await?;
    state
        .current_key
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found"))
        .map(Json)
}

async fn admin_update_api_key_limits(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AdministratorJson(admin, input): AdministratorJson<storage::LimitPolicyPatch>,
) -> Result<Json<storage::LimitSubjectState>, ApiError> {
    validate_limit_policy(&input)?;
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
        .map(Json)
}

async fn admin_limits(
    State(state): State<AppState>,
    Administrator(_admin): Administrator,
) -> Result<Json<storage::AdminLimitState>, ApiError> {
    Ok(Json(storage::admin_limit_state(&state.db).await?))
}

async fn admin_update_system_limits(
    State(state): State<AppState>,
    AdministratorJson(admin, input): AdministratorJson<storage::LimitPolicyPatch>,
) -> Result<Json<storage::LimitPolicy>, ApiError> {
    validate_limit_policy(&input)?;
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
    Ok(Json(policy))
}

fn validate_limit_policy(input: &storage::LimitPolicyPatch) -> Result<(), ApiError> {
    for (name, value) in [
        ("request_quota", &input.request_quota),
        ("token_quota", &input.token_quota),
        ("rate_limit_requests", &input.rate_limit_requests),
        ("concurrency_limit", &input.concurrency_limit),
    ] {
        if matches!(value, storage::LimitPatchValue::Set(value) if *value < 0) {
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
