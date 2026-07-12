use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{AppState, storage};

use super::{
    ApiError,
    auth::{Administrator, AdministratorJson, admin_audit, authenticate},
    contracts::{ApiKeyResponse, ApiKeyUsageResponse},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/api-keys", get(my_api_keys).post(create_my_api_key))
        .route("/api/api-keys/{id}/usage", get(my_api_key_usage))
        .route("/api/api-keys/{id}/disable", post(disable_my_api_key))
        .route("/api/api-keys/{id}/revoke", post(revoke_my_api_key))
        .route(
            "/api/admin/api-keys",
            get(admin_api_keys).post(admin_create_api_key),
        )
        .route("/api/admin/api-keys/{id}/usage", get(admin_api_key_usage))
        .route(
            "/api/admin/api-keys/{id}/disable",
            post(admin_disable_api_key),
        )
        .route(
            "/api/admin/api-keys/{id}/revoke",
            post(admin_revoke_api_key),
        )
}

#[derive(Serialize)]
struct CreatedApiKey {
    key: ApiKeyResponse,
    plaintext: String,
}

#[derive(Clone, Debug, Deserialize)]
struct CreateApiKeyRequest {
    name: String,
    expires_at: Option<String>,
}

impl From<CreateApiKeyRequest> for storage::CreateApiKey {
    fn from(value: CreateApiKeyRequest) -> Self {
        Self {
            name: value.name,
            expires_at: value.expires_at,
        }
    }
}

async fn my_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ApiKeyResponse>>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    Ok(Json(
        storage::list_api_keys_for_user(&state.db, &user.user_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

async fn create_my_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateApiKeyRequest>,
) -> Result<Json<CreatedApiKey>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    validate_create_api_key(&input)?;
    let input: storage::CreateApiKey = input.into();
    let (key, plaintext) =
        storage::create_api_key(&state.db, &state.config.app_secret, &user.user_id, &input).await?;
    Ok(Json(CreatedApiKey {
        key: key.into(),
        plaintext,
    }))
}

async fn my_api_key_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiKeyUsageResponse>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    let key = storage::get_api_key(&state.db, &id).await?.ok_or_else(|| {
        ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found")
    })?;
    if key.user_id != user.user_id {
        return Err(ApiError::forbidden(
            "API key does not belong to user",
            "forbidden",
        ));
    }
    Ok(Json(
        storage::api_key_usage_summary_at(&state.db, key, true, state.clock.now())
            .await?
            .try_into()?,
    ))
}

async fn disable_my_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiKeyResponse>, ApiError> {
    update_my_api_key_status(state, headers, id, "disabled").await
}

async fn revoke_my_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiKeyResponse>, ApiError> {
    update_my_api_key_status(state, headers, id, "revoked").await
}

async fn update_my_api_key_status(
    state: AppState,
    headers: HeaderMap,
    id: String,
    status: &'static str,
) -> Result<Json<ApiKeyResponse>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    let key = storage::get_api_key(&state.db, &id).await?.ok_or_else(|| {
        ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found")
    })?;
    if key.user_id != user.user_id {
        return Err(ApiError::forbidden(
            "API key does not belong to user",
            "forbidden",
        ));
    }
    let updated = storage::set_api_key_status(&state.db, &id, status)
        .await?
        .ok_or_else(|| {
            ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found")
        })?;
    Ok(Json(updated.into()))
}

async fn admin_api_keys(
    State(state): State<AppState>,
    Administrator(_admin): Administrator,
) -> Result<Json<Vec<ApiKeyResponse>>, ApiError> {
    Ok(Json(
        storage::list_api_keys(&state.db)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

async fn admin_api_key_usage(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Administrator(_admin): Administrator,
) -> Result<Json<ApiKeyUsageResponse>, ApiError> {
    let key = storage::get_api_key(&state.db, &id).await?.ok_or_else(|| {
        ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found")
    })?;
    Ok(Json(
        storage::api_key_usage_summary_at(&state.db, key, true, state.clock.now())
            .await?
            .try_into()?,
    ))
}

#[derive(Deserialize)]
struct AdminCreateApiKey {
    user_id: String,
    name: String,
    expires_at: Option<String>,
}

async fn admin_create_api_key(
    State(state): State<AppState>,
    AdministratorJson(admin, input): AdministratorJson<AdminCreateApiKey>,
) -> Result<Json<CreatedApiKey>, ApiError> {
    let create = CreateApiKeyRequest {
        name: input.name,
        expires_at: input.expires_at,
    };
    validate_create_api_key(&create)?;
    let create: storage::CreateApiKey = create.into();
    storage::get_user(&state.db, &input.user_id)
        .await?
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "user not found", "not_found"))?;
    let app_secret = state.config.app_secret.clone();
    let (key, plaintext) = storage::with_admin_audit::<_, ApiError, _>(
        &state.db,
        move |conn| {
            Box::pin(async move {
                let (key, plaintext) =
                    storage::create_api_key_conn(conn, &app_secret, &input.user_id, &create)
                        .await?;
                let audit = admin_audit(
                    &admin,
                    "create_api_key",
                    "api_key",
                    Some(key.id.clone()),
                    json!({ "user_id": input.user_id, "name": key.name, "expires_at_set": key.expires_at.is_some() }),
                );
                Ok(((key, plaintext), audit))
            })
        },
    )
    .await?;
    Ok(Json(CreatedApiKey {
        key: key.into(),
        plaintext,
    }))
}

async fn admin_disable_api_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Administrator(admin): Administrator,
) -> Result<Json<ApiKeyResponse>, ApiError> {
    update_admin_api_key_status(state, admin, id, "disabled").await
}

async fn admin_revoke_api_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Administrator(admin): Administrator,
) -> Result<Json<ApiKeyResponse>, ApiError> {
    update_admin_api_key_status(state, admin, id, "revoked").await
}

async fn update_admin_api_key_status(
    state: AppState,
    admin: crate::auth::AuthenticatedUser,
    id: String,
    status: &'static str,
) -> Result<Json<ApiKeyResponse>, ApiError> {
    let updated = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let updated = storage::set_api_key_status_conn(conn, &id, status)
                .await?
                .ok_or_else(|| {
                    ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found")
                })?;
            let audit = admin_audit(
                &admin,
                if status == "revoked" {
                    "revoke_api_key"
                } else {
                    "disable_api_key"
                },
                "api_key",
                Some(id),
                json!({ "status": status }),
            );
            Ok((updated, audit))
        })
    })
    .await?;
    Ok(Json(updated.into()))
}

fn validate_create_api_key(input: &CreateApiKeyRequest) -> Result<(), ApiError> {
    if input.name.trim().is_empty() {
        return Err(ApiError::bad_request(
            "name must not be empty",
            "invalid_request",
        ));
    }
    if let Some(expires_at) = &input.expires_at {
        chrono::DateTime::parse_from_rfc3339(expires_at).map_err(|_| {
            ApiError::bad_request("expires_at must be an RFC3339 timestamp", "invalid_request")
        })?;
    }
    Ok(())
}
