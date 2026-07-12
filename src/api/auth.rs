use axum::{
    Json, Router,
    extract::{FromRequest, FromRequestParts, Request, State},
    http::{HeaderMap, StatusCode, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{AppState, auth as core_auth, storage};

use super::ApiError;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/login", post(login))
        .route("/api/me", get(me))
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    user: LoginUser,
    token: String,
    token_type: &'static str,
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
    if user.status != "active" || !core_auth::verify_password(&input.password, &user.password_hash)
    {
        return Err(ApiError::gateway(
            StatusCode::UNAUTHORIZED,
            "invalid login",
            "invalid_login",
        ));
    }

    storage::mark_user_login(&state.db, &user.id).await?;
    let token = core_auth::generate_panel_token(&state.config.app_secret, &user.id);

    Ok(Json(LoginResponse {
        user: LoginUser {
            id: user.id,
            email: user.email,
            role: user.role,
        },
        token,
        token_type: "panel",
    }))
}

async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<core_auth::AuthenticatedUser>, ApiError> {
    Ok(Json(authenticate(&state, &headers).await?))
}

pub async fn authenticate_api_key(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<core_auth::AuthenticatedUser, ApiError> {
    let header = authorization(headers);
    core_auth::authenticate_api_key(&state.db, &state.config.app_secret, header)
        .await
        .map_err(ApiError::from_auth)
}

pub async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<core_auth::AuthenticatedUser, ApiError> {
    let header = authorization(headers);
    let plaintext = core_auth::parse_bearer(header).map_err(ApiError::from_auth)?;
    if !core_auth::is_panel_token(plaintext) {
        return core_auth::authenticate_api_key(&state.db, &state.config.app_secret, header)
            .await
            .map_err(ApiError::from_auth);
    }

    let (user_id, session_id) = core_auth::verify_panel_token(&state.config.app_secret, plaintext)
        .map_err(ApiError::from_auth)?;
    let user = storage::get_user(&state.db, &user_id)
        .await?
        .ok_or_else(|| ApiError::from_auth(core_auth::AuthError::Invalid))?;
    if user.status != "active" {
        return Err(ApiError::from_auth(core_auth::AuthError::DisabledUser));
    }
    Ok(core_auth::AuthenticatedUser {
        user_id: user.id,
        api_key_id: format!("panel:{session_id}"),
        key_prefix: "panel".to_string(),
        email: user.email,
        role: user.role,
    })
}

pub async fn require_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<core_auth::AuthenticatedUser, ApiError> {
    authorize_admin(authenticate(state, headers).await?)
}

pub(super) struct Administrator(pub(super) core_auth::AuthenticatedUser);

impl FromRequestParts<AppState> for Administrator {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        authenticate(state, &parts.headers)
            .await
            .and_then(authorize_admin)
            .map(Self)
    }
}

pub(super) struct AdministratorJson<T>(pub(super) core_auth::AuthenticatedUser, pub(super) T);

impl<T> FromRequest<AppState> for AdministratorJson<T>
where
    T: DeserializeOwned + Send,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        let headers = request.headers().clone();
        let Json(input) = Json::<T>::from_request(request, state)
            .await
            .map_err(IntoResponse::into_response)?;
        let administrator = require_admin(state, &headers)
            .await
            .map_err(IntoResponse::into_response)?;
        Ok(Self(administrator, input))
    }
}

fn authorization(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
}

fn authorize_admin(
    user: core_auth::AuthenticatedUser,
) -> Result<core_auth::AuthenticatedUser, ApiError> {
    if core_auth::is_admin(&user) {
        Ok(user)
    } else {
        Err(ApiError::forbidden("admin role required", "forbidden"))
    }
}

pub(super) fn admin_audit(
    actor: &core_auth::AuthenticatedUser,
    action: &'static str,
    resource_type: &'static str,
    resource_id: Option<String>,
    metadata: serde_json::Value,
) -> storage::AdminAuditInsert {
    storage::AdminAuditInsert {
        actor_user_id: actor.user_id.clone(),
        actor_email: actor.email.clone(),
        action,
        resource_type,
        resource_id,
        status: "success",
        metadata_json: Some(metadata.to_string()),
    }
}
