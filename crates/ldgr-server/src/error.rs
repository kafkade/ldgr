use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("payload too large")]
    #[allow(dead_code)]
    PayloadTooLarge,
    #[error("storage quota exceeded: {0}")]
    QuotaExceeded(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<rusqlite::Error> for ServerError {
    fn from(e: rusqlite::Error) -> Self {
        tracing::error!("database error: {e}");
        Self::Internal("database error".into())
    }
}

impl From<tokio::task::JoinError> for ServerError {
    fn from(e: tokio::task::JoinError) -> Self {
        tracing::error!("task join error: {e}");
        Self::Internal("internal error".into())
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::PayloadTooLarge | Self::QuotaExceeded(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        // Don't leak internal details to clients
        let message = match &self {
            Self::Internal(_) => "internal server error".to_string(),
            other => other.to_string(),
        };
        let body = serde_json::json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}
