use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch, post},
};
use serde::Deserialize;
use serde_json::json;

use crate::{AppState, storage};

use super::{
    ApiError,
    auth::{Administrator, AdministratorJson, admin_audit},
    contracts::UserResponse,
};

#[derive(Clone, Debug, Deserialize)]
struct CreateUserRequest {
    email: String,
    password: String,
    role: String,
    display_name: Option<String>,
}

impl From<CreateUserRequest> for storage::CreateUser {
    fn from(value: CreateUserRequest) -> Self {
        Self {
            email: value.email,
            password: value.password,
            role: value.role,
            display_name: value.display_name,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct UpdateUserRequest {
    role: Option<String>,
    status: Option<String>,
    display_name: Option<String>,
}

impl From<UpdateUserRequest> for storage::UpdateUser {
    fn from(value: UpdateUserRequest) -> Self {
        Self {
            role: value.role,
            status: value.status,
            display_name: value.display_name,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct ResetPasswordRequest {
    password: String,
}

impl From<ResetPasswordRequest> for storage::ResetPassword {
    fn from(value: ResetPasswordRequest) -> Self {
        Self {
            password: value.password,
        }
    }
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/admin/users", get(admin_users).post(admin_create_user))
        .route("/api/admin/users/{id}", patch(admin_update_user))
        .route("/api/admin/users/{id}/password", post(admin_reset_password))
}

async fn admin_users(
    State(state): State<AppState>,
    Administrator(_admin): Administrator,
) -> Result<Json<Vec<UserResponse>>, ApiError> {
    Ok(Json(
        storage::list_users(&state.db)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

async fn admin_create_user(
    State(state): State<AppState>,
    AdministratorJson(admin, input): AdministratorJson<CreateUserRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    validate_create_user(&input)?;
    let input: storage::CreateUser = input.into();
    let id = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let id = storage::ensure_user_conn(conn, &input).await?;
            let audit = admin_audit(
                &admin,
                "create_user",
                "user",
                Some(id.clone()),
                json!({ "email": input.email, "role": input.role }),
            );
            Ok((id, audit))
        })
    })
    .await?;
    Ok(Json(json!({ "id": id })))
}

async fn admin_update_user(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AdministratorJson(admin, input): AdministratorJson<UpdateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    validate_update_user(&input)?;
    let input: storage::UpdateUser = input.into();
    let user = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let user = storage::update_user_conn(conn, &id, &input)
                .await?
                .ok_or_else(|| {
                    ApiError::gateway(StatusCode::NOT_FOUND, "user not found", "not_found")
                })?;
            let audit = admin_audit(
                &admin,
                "update_user",
                "user",
                Some(id),
                json!({
                    "role_changed": input.role.is_some(),
                    "status_changed": input.status.is_some(),
                    "display_name_changed": input.display_name.is_some()
                }),
            );
            Ok((user, audit))
        })
    })
    .await?;
    Ok(Json(user.into()))
}

async fn admin_reset_password(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AdministratorJson(admin, input): AdministratorJson<ResetPasswordRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    validate_password(&input.password)?;
    let input: storage::ResetPassword = input.into();
    let response_id = id.clone();
    storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            if !storage::reset_user_password_command_conn(conn, &id, &input).await? {
                return Err(ApiError::gateway(
                    StatusCode::NOT_FOUND,
                    "user not found",
                    "not_found",
                ));
            }
            let audit = admin_audit(
                &admin,
                "reset_user_password",
                "user",
                Some(id),
                json!({ "password_reset": true }),
            );
            Ok(((), audit))
        })
    })
    .await?;
    let id = response_id;
    Ok(Json(json!({ "id": id, "password_reset": true })))
}

fn validate_create_user(input: &CreateUserRequest) -> Result<(), ApiError> {
    validate_email(&input.email)?;
    validate_password(&input.password)?;
    validate_role(&input.role)?;
    validate_optional_name("display_name", input.display_name.as_deref())?;
    Ok(())
}

fn validate_update_user(input: &UpdateUserRequest) -> Result<(), ApiError> {
    if input.role.is_none() && input.status.is_none() && input.display_name.is_none() {
        return Err(ApiError::bad_request(
            "no user fields supplied",
            "invalid_request",
        ));
    }
    if let Some(role) = &input.role {
        validate_role(role)?;
    }
    if let Some(status) = &input.status {
        validate_user_status(status)?;
    }
    validate_optional_name("display_name", input.display_name.as_deref())?;
    Ok(())
}

fn validate_email(email: &str) -> Result<(), ApiError> {
    validate_required("email", email)?;
    if !email.contains('@') || email.contains(char::is_whitespace) {
        return Err(ApiError::bad_request(
            "email must be a valid address",
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.len() < 8 {
        return Err(ApiError::bad_request(
            "password must be at least 8 characters",
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_role(role: &str) -> Result<(), ApiError> {
    if !matches!(role, "admin" | "user") {
        return Err(ApiError::bad_request(
            "role must be admin or user",
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_user_status(status: &str) -> Result<(), ApiError> {
    if !matches!(status, "active" | "disabled") {
        return Err(ApiError::bad_request(
            "status must be active or disabled",
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_optional_name(field: &str, value: Option<&str>) -> Result<(), ApiError> {
    if let Some(value) = value {
        validate_required(field, value)?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_password_request_converts_to_storage_command() {
        let command: storage::ResetPassword = ResetPasswordRequest {
            password: "new-pass-123".into(),
        }
        .into();
        assert_eq!(command.password, "new-pass-123");
    }
}
