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

    /// OAuth bearer token is invalid (401)
    #[error("The access token is invalid")]
    InvalidAccessToken,

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

    /// JSON serialization/deserialization error (500)
    #[error("Serialization error during {operation}: {source}")]
    Serialization {
        operation: &'static str,
        #[source]
        source: serde_json::Error,
    },

    /// Async task join error (500)
    #[error("Task join error during {operation}: {source}")]
    TaskJoin {
        operation: &'static str,
        #[source]
        source: tokio::task::JoinError,
    },

    /// Rate limit exceeded (429)
    #[error("Rate limit exceeded")]
    RateLimited,

    /// Internal server error (500)
    #[error("Internal error: {0}")]
    Internal(String),

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

        let (status, error_message, error_type, should_log_detail) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, self.to_string(), "not_found", false),
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                self.to_string(),
                "unauthorized",
                false,
            ),
            AppError::InvalidAccessToken => (
                StatusCode::UNAUTHORIZED,
                self.to_string(),
                "invalid_access_token",
                false,
            ),
            AppError::InvalidSignature => (
                StatusCode::UNAUTHORIZED,
                self.to_string(),
                "invalid_signature",
                false,
            ),
            AppError::Forbidden => (StatusCode::FORBIDDEN, self.to_string(), "forbidden", false),
            AppError::Validation(msg) => {
                (StatusCode::BAD_REQUEST, msg.clone(), "validation", false)
            }
            AppError::Unprocessable(msg) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                msg.clone(),
                "unprocessable",
                false,
            ),
            AppError::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                self.to_string(),
                "rate_limited",
                false,
            ),
            AppError::NotImplemented(msg) => (
                StatusCode::NOT_IMPLEMENTED,
                msg.clone(),
                "not_implemented",
                false,
            ),
            AppError::Federation(_) => (
                StatusCode::BAD_GATEWAY,
                "Federation error".to_string(),
                "federation",
                true,
            ),
            AppError::HttpClient(_) => (
                StatusCode::BAD_GATEWAY,
                "Upstream HTTP error".to_string(),
                "http_client",
                true,
            ),
            AppError::Database(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
                "database",
                true,
            ),
            AppError::Storage(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Storage error".to_string(),
                "storage",
                true,
            ),
            AppError::Config(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Configuration error".to_string(),
                "config",
                true,
            ),
            AppError::Encryption(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Encryption error".to_string(),
                "encryption",
                true,
            ),
            AppError::Serialization { .. } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Serialization error".to_string(),
                "serialization",
                true,
            ),
            AppError::TaskJoin { .. } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Task execution error".to_string(),
                "task_join",
                true,
            ),
            AppError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
                "internal",
                true,
            ),
        };

        if should_log_detail {
            tracing::error!(error = %self, %error_type, "Request failed with internal details");
        }

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

impl AppError {
    /// Construct internal error from any displayable error context.
    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self::Internal(error.to_string())
    }

    /// Construct JSON serialization/deserialization error with operation context.
    pub fn serialization(operation: &'static str, source: serde_json::Error) -> Self {
        Self::Serialization { operation, source }
    }

    /// Construct task join error with operation context.
    pub fn task_join(operation: &'static str, source: tokio::task::JoinError) -> Self {
        Self::TaskJoin { operation, source }
    }
}

/// Result type alias using AppError
pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::AppError;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn storage_errors_are_sanitized() {
        let response =
            AppError::Storage("s3 endpoint timeout at secret-host".to_string()).into_response();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
        assert!(body_text.contains("Storage error"));
        assert!(!body_text.contains("secret-host"));
    }

    #[tokio::test]
    async fn validation_errors_keep_message() {
        let response = AppError::Validation("invalid media id".to_string()).into_response();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
        assert!(body_text.contains("invalid media id"));
    }

    #[tokio::test]
    async fn serialization_errors_are_sanitized() {
        let source = serde_json::from_str::<serde_json::Value>("{")
            .expect_err("invalid JSON should produce serde error");
        let response = AppError::serialization("test parse", source).into_response();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
        assert!(body_text.contains("Serialization error"));
        assert!(!body_text.contains("test parse"));
    }

    #[tokio::test]
    async fn task_join_errors_are_sanitized() {
        let join_error = tokio::task::spawn(async { panic!("sensitive panic detail") })
            .await
            .expect_err("task should panic");
        let response = AppError::task_join("task test", join_error).into_response();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
        assert!(body_text.contains("Task execution error"));
        assert!(!body_text.contains("sensitive panic detail"));
    }
}
