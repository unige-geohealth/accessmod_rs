//! API errors: an HTTP status and the JSON document the CLI prints with
//! `--json` (`status`, `kind`, `issues`).

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use accessmod_io::error::{Error, ErrorKind};

/// Error of a request.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Engine or project error.
    #[error(transparent)]
    Engine(#[from] Error),
    /// No such resource.
    #[error("{0}")]
    NotFound(String),
    /// The user may not do this.
    #[error("{0}")]
    Forbidden(String),
    /// The request is malformed (not the job: the HTTP request itself).
    #[error("{0}")]
    BadRequest(String),
    /// A bug: a panic or a lost task, reported instead of crashing.
    #[error("internal error: {0}")]
    Internal(String),
}

impl ApiError {
    fn parts(&self) -> (StatusCode, &'static str, Vec<String>) {
        match self {
            ApiError::Engine(error) => {
                let status = match error.kind() {
                    ErrorKind::Validation => StatusCode::UNPROCESSABLE_ENTITY,
                    ErrorKind::Cancelled => StatusCode::CONFLICT,
                    ErrorKind::Io | ErrorKind::Computation => StatusCode::INTERNAL_SERVER_ERROR,
                };
                (status, error.kind().name(), error.issues())
            }
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, "not_found", vec![m.clone()]),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, "forbidden", vec![m.clone()]),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", vec![m.clone()]),
            ApiError::Internal(m) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                vec![m.clone()],
            ),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, kind, issues) = self.parts();
        if status.is_server_error() {
            tracing::error!("{kind}: {}", issues.join("; "));
        }
        (
            status,
            Json(json!({"status": "error", "kind": kind, "issues": issues})),
        )
            .into_response()
    }
}

/// Result of a request.
pub type ApiResult<T> = Result<T, ApiError>;
