//! Error types for RustResort
//!
//! All errors in the application are converted to `AppError`,
//! which implements `IntoResponse` for proper HTTP error responses.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

/// Application-wide error type
///
/// This enum represents all possible errors that can occur
/// in the application. It implements `IntoResponse` to
/// automatically convert errors to appropriate HTTP responses.
#[derive(Debug, Error)]
pub enum AppError {
    /// Resource not found (404)
    #[error("Resource not found")]
    NotFound,

    /// Authentication required (401)
    #[error("Authentication required")]
    Unauthorized,

    /// Access denied (403)
    #[error("Access denied")]
    Forbidden,

    /// Validation error (400)
    #[error("Validation error: {0}")]
    Validation(String),

    /// Unprocessable entity (422)
    #[error("Unprocessable entity: {0}")]
    Unprocessable(String),

    /// Database error (500)
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// R2 storage error (500)
    #[error("Storage error: {0}")]
    Storage(String),

    /// HTTP client error (502)
    #[error("HTTP client error: {0}")]
    HttpClient(#[from] reqwest::Error),

    /// Federation error (502)
    #[error("Federation error: {0}")]
    Federation(String),

    /// Signature verification failed (401)
    #[error("Invalid signature")]
    InvalidSignature,

    /// Configuration error (500)
    #[error("Configuration error: {0}")]
    Config(String),

    /// Encryption/decryption error (500)
    #[error("Encryption error: {0}")]
    Encryption(String),

    /// Rate limit exceeded (429)
    #[error("Rate limit exceeded")]
    RateLimited,

    /// Internal server error (500)
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),

    /// Not implemented (501)
    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

impl From<config::ConfigError> for AppError {
    fn from(err: config::ConfigError) -> Self {
        AppError::Config(err.to_string())
    }
}

impl IntoResponse for AppError {
    /// Convert error to HTTP response
    ///
    /// Maps each error variant to appropriate HTTP status code
    /// and JSON error body.
    fn into_response(self) -> Response {
        use axum::Json;

        let (status, error_message, error_type) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, self.to_string(), "not_found"),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string(), "unauthorized"),
            AppError::InvalidSignature => (
                StatusCode::UNAUTHORIZED,
                self.to_string(),
                "invalid_signature",
            ),
            AppError::Forbidden => (StatusCode::FORBIDDEN, self.to_string(), "forbidden"),
            AppError::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone(), "validation"),
            AppError::Unprocessable(msg) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                msg.clone(),
                "unprocessable",
            ),
            AppError::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                self.to_string(),
                "rate_limited",
            ),
            AppError::NotImplemented(msg) => {
                (StatusCode::NOT_IMPLEMENTED, msg.clone(), "not_implemented")
            }
            AppError::Federation(msg) => (StatusCode::BAD_GATEWAY, msg.clone(), "federation"),
            AppError::HttpClient(_) => (StatusCode::BAD_GATEWAY, self.to_string(), "http_client"),
            AppError::Database(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
                "database",
            ),
            AppError::Storage(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone(), "storage"),
            AppError::Config(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone(), "config"),
            AppError::Encryption(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg.clone(), "encryption")
            }
            AppError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
                "internal",
            ),
        };

        // Record error metric
        use crate::metrics::ERRORS_TOTAL;
        ERRORS_TOTAL
            .with_label_values(&[error_type, "unknown"])
            .inc();

        let body = Json(serde_json::json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}

/// Result type alias using AppError
pub type Result<T> = std::result::Result<T, AppError>;
