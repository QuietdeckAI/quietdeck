//! Errors the REST API returns.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use quietdeck_types::api::ApiError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    /// The relay's one failure: the target is offline. 409 so a CLI can tell it from a bad request.
    #[error("{0}")]
    Offline(String),
    #[error("{0}")]
    Unavailable(String),
    /// A retired route (410): what replaced it is in the message.
    #[error("{0}")]
    Gone(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::Internal(e.into())
    }
}

pub type AppResult<T> = Result<T, AppError>;

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::BadRequest(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Conflict(_) | AppError::Offline(_) => StatusCode::CONFLICT,
            AppError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            AppError::Gone(_) => StatusCode::GONE,
            AppError::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let error = match &self {
            AppError::Internal(_) => "internal error; the deck's log has the details".to_string(),
            other => other.to_string(),
        };
        (status, Json(ApiError { error })).into_response()
    }
}

/// Validate an input type with its `check` (garde rules plus cross-field rules).
pub fn bad<E: std::fmt::Display>(e: E) -> AppError {
    AppError::BadRequest(e.to_string())
}
