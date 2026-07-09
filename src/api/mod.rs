use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    AppState, auth,
    storage::{self, CreateApiKey, CreateUser, UpsertModel, UpsertUpstream},
};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/api/login", post(login))
        .route("/api/me", get(me))
        .route("/api/overview", get(overview))
        .route("/api/api-keys", get(my_api_keys).post(create_my_api_key))
        .route("/api/requests", get(my_requests))
        .route("/api/usage/daily", get(my_usage))
        .route("/api/admin/users", get(admin_users).post(admin_create_user))
        .route("/api/admin/api-keys", get(admin_api_keys))
        .route(
            "/api/admin/upstreams",
            get(admin_upstreams).post(admin_create_upstream),
        )
        .route(
            "/api/admin/upstreams/{id}/health",
            post(admin_check_upstream_health),
        )
        .route(
            "/api/admin/models",
            get(admin_models).post(admin_create_model),
        )
        .route("/api/admin/requests", get(admin_requests))
        .route("/api/admin/usage/daily", get(admin_usage))
        .route("/responses", post(crate::proxy::proxy_responses))
        .route("/v1/responses", post(crate::proxy::proxy_responses))
        .route("/responses/compact", post(crate::proxy::proxy_responses))
        .route("/v1/responses/compact", post(crate::proxy::proxy_responses))
        .route("/v1/models", get(crate::proxy::models))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Result<Json<Health>, ApiError> {
    sqlx::query("SELECT 1").execute(&state.db).await?;
    Ok(Json(Health {
        status: "ok",
        service: "codex-gateway",
    }))
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    service: &'static str,
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    user: LoginUser,
    key: storage::ApiKeySummary,
    plaintext: String,
}

#[derive(Serialize)]
struct LoginUser {
    id: String,
    email: String,
    role: String,
}

async fn login(
    State(state): State<AppState>,
    Json(input): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let user = storage::find_user_credentials_by_email(&state.db, &input.email)
        .await?
        .ok_or_else(|| {
            ApiError::gateway(StatusCode::UNAUTHORIZED, "invalid login", "invalid_login")
        })?;
    if user.status != "active" || !auth::verify_password(&input.password, &user.password_hash) {
        return Err(ApiError::gateway(
            StatusCode::UNAUTHORIZED,
            "invalid login",
            "invalid_login",
        ));
    }

    storage::mark_user_login(&state.db, &user.id).await?;
    let (key, plaintext) = storage::create_api_key(
        &state.db,
        &state.config.app_secret,
        &user.id,
        &CreateApiKey {
            name: "web-login".to_string(),
            expires_at: None,
        },
    )
    .await?;

    Ok(Json(LoginResponse {
        user: LoginUser {
            id: user.id,
            email: user.email,
            role: user.role,
        },
        key,
        plaintext,
    }))
}

async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<auth::AuthenticatedUser>, ApiError> {
    Ok(Json(authenticate(&state, &headers).await?))
}

async fn overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    let usage = storage::list_daily_usage(&state.db, Some(&user.user_id)).await?;
    let requests = storage::list_request_logs(&state.db, Some(&user.user_id)).await?;
    Ok(Json(json!({
        "user": user,
        "daily_usage": usage,
        "recent_requests": requests.into_iter().take(20).collect::<Vec<_>>()
    })))
}

async fn my_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::ApiKeySummary>>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    Ok(Json(
        storage::list_api_keys_for_user(&state.db, &user.user_id).await?,
    ))
}

#[derive(Serialize)]
struct CreatedApiKey {
    key: storage::ApiKeySummary,
    plaintext: String,
}

async fn create_my_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateApiKey>,
) -> Result<Json<CreatedApiKey>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    let (key, plaintext) =
        storage::create_api_key(&state.db, &state.config.app_secret, &user.user_id, &input).await?;
    Ok(Json(CreatedApiKey { key, plaintext }))
}

async fn my_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::RequestLogRow>>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    Ok(Json(
        storage::list_request_logs(&state.db, Some(&user.user_id)).await?,
    ))
}

async fn my_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::DailyUsageRow>>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    Ok(Json(
        storage::list_daily_usage(&state.db, Some(&user.user_id)).await?,
    ))
}

async fn admin_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::User>>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::list_users(&state.db).await?))
}

async fn admin_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateUser>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers).await?;
    let id = storage::ensure_user(&state.db, &input).await?;
    Ok(Json(json!({ "id": id })))
}

async fn admin_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::ApiKeySummary>>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::list_api_keys(&state.db).await?))
}

async fn admin_upstreams(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::Upstream>>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::list_upstreams(&state.db).await?))
}

async fn admin_create_upstream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UpsertUpstream>,
) -> Result<Json<storage::Upstream>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::create_upstream(&state.db, &input).await?))
}

async fn admin_check_upstream_health(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers).await?;
    let upstream = storage::get_upstream(&state.db, &id)
        .await?
        .ok_or_else(|| {
            ApiError::gateway(StatusCode::NOT_FOUND, "upstream not found", "not_found")
        })?;
    let status = crate::upstream::check_upstream_health(&state.http, &state.db, &upstream)
        .await
        .map_err(|error| {
            tracing::warn!(?error, upstream_id = %id, "upstream health check failed");
            ApiError::gateway(
                StatusCode::BAD_GATEWAY,
                "upstream health check failed",
                "upstream_unavailable",
            )
        })?;
    Ok(Json(json!({ "id": id, "health": status })))
}

async fn admin_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::Model>>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::list_models(&state.db).await?))
}

async fn admin_create_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UpsertModel>,
) -> Result<Json<storage::Model>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::create_model(&state.db, &input).await?))
}

async fn admin_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::RequestLogRow>>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::list_request_logs(&state.db, None).await?))
}

async fn admin_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::DailyUsageRow>>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::list_daily_usage(&state.db, None).await?))
}

pub async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<auth::AuthenticatedUser, ApiError> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    auth::authenticate_api_key(&state.db, &state.config.app_secret, header)
        .await
        .map_err(ApiError::from_auth)
}

pub async fn require_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<auth::AuthenticatedUser, ApiError> {
    let user = authenticate(state, headers).await?;
    if auth::is_admin(&user) {
        Ok(user)
    } else {
        Err(ApiError::forbidden("admin role required", "forbidden"))
    }
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
    kind: &'static str,
    code: &'static str,
}

impl ApiError {
    pub fn gateway(status: StatusCode, message: impl Into<String>, code: &'static str) -> Self {
        Self {
            status,
            message: message.into(),
            kind: "gateway_error",
            code,
        }
    }

    pub fn forbidden(message: impl Into<String>, code: &'static str) -> Self {
        Self::gateway(StatusCode::FORBIDDEN, message, code)
    }

    pub fn from_auth(error: auth::AuthError) -> Self {
        match error {
            auth::AuthError::Missing | auth::AuthError::Invalid => Self::gateway(
                StatusCode::UNAUTHORIZED,
                "invalid API key",
                "invalid_api_key",
            ),
            auth::AuthError::Disabled => Self::gateway(
                StatusCode::FORBIDDEN,
                "disabled API key",
                "disabled_api_key",
            ),
            auth::AuthError::Expired => {
                Self::gateway(StatusCode::FORBIDDEN, "expired API key", "expired_api_key")
            }
            auth::AuthError::DisabledUser => {
                Self::gateway(StatusCode::FORBIDDEN, "disabled user", "disabled_user")
            }
            auth::AuthError::Storage(_) => Self::gateway(
                StatusCode::INTERNAL_SERVER_ERROR,
                "gateway storage error",
                "gateway_internal_error",
            ),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        tracing::error!(?error, "database error");
        Self::gateway(
            StatusCode::INTERNAL_SERVER_ERROR,
            "gateway storage error",
            "gateway_internal_error",
        )
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        tracing::error!(?error, "gateway error");
        Self::gateway(
            StatusCode::INTERNAL_SERVER_ERROR,
            "gateway internal error",
            "gateway_internal_error",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({
            "error": {
                "message": self.message,
                "type": self.kind,
                "code": self.code
            }
        }));
        (self.status, body).into_response()
    }
}
