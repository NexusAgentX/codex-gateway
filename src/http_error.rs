use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use crate::{auth, storage};

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
    kind: &'static str,
    code: &'static str,
    details: Option<Value>,
}

impl ApiError {
    pub fn gateway(status: StatusCode, message: impl Into<String>, code: &'static str) -> Self {
        Self {
            status,
            message: message.into(),
            kind: "gateway_error",
            code,
            details: None,
        }
    }

    pub fn limit(rejection: storage::LimitRejection) -> Self {
        let status = if rejection.code == "quota_exceeded" {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::TOO_MANY_REQUESTS
        };
        Self {
            status,
            message: rejection.message.clone(),
            kind: "limit_error",
            code: rejection.code,
            details: Some(json!({
                "scope": rejection.scope,
                "subject_id": rejection.subject_id,
                "limit_name": rejection.limit_name,
                "limit": rejection.limit,
                "used": rejection.used,
                "reset_at": rejection.reset_at
            })),
        }
    }

    pub fn bad_request(message: impl Into<String>, code: &'static str) -> Self {
        Self::gateway(StatusCode::BAD_REQUEST, message, code)
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

impl From<storage::LimitAdmissionError> for ApiError {
    fn from(error: storage::LimitAdmissionError) -> Self {
        match error {
            storage::LimitAdmissionError::Rejected(rejection) => Self::limit(rejection),
            storage::LimitAdmissionError::Storage(error) => Self::from(error),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut error = json!({
            "message": self.message,
            "type": self.kind,
            "code": self.code
        });
        if let Some(details) = self.details
            && let Some(object) = error.as_object_mut()
        {
            object.insert("details".to_string(), details);
        }
        let body = Json(json!({
            "error": {
                "message": error["message"],
                "type": error["type"],
                "code": error["code"],
                "details": error.get("details")
            }
        }));
        (self.status, body).into_response()
    }
}
