//! Authentication middleware
//!
//! Protects routes that require authentication.

use axum::{
    async_trait,
    extract::{FromRequestParts, State},
    http::{Request, request::Parts},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::CookieJar;

use super::session::{Session, verify_session_token};
use crate::AppState;
use crate::error::AppError;

/// Middleware to require authentication
///
/// Extracts and verifies session from cookie or Authorization header.
/// Adds Session to request extensions if valid.
///
/// # Usage
/// ```ignore
/// let protected_routes = Router::new()
///     .route("/api/v1/...", ...)
///     .layer(middleware::from_fn_with_state(state, require_auth));
/// ```
pub async fn require_auth(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    // Try to get token from Authorization header first
    let token = request
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .or_else(|| {
            // Fallback to cookie
            jar.get("session").map(|cookie| cookie.value())
        });

    let token = token.ok_or(AppError::Unauthorized)?;

    // Verify token and get session
    let session = verify_session_token(token, &state.config.auth.session_secret)?;

    // Add session to request extensions
    request.extensions_mut().insert(session);

    // Continue to next handler
    Ok(next.run(request).await)
}

/// Extractor for current authenticated user
///
/// Use in handlers to get the current session.
///
/// # Usage
/// ```ignore
/// async fn handler(
///     CurrentUser(session): CurrentUser,
/// ) -> impl IntoResponse {
///     format!("Hello, {}", session.github_username)
/// }
/// ```
#[derive(Debug, Clone)]
pub struct CurrentUser(pub Session);

#[async_trait]
impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    /// Extract current user from request
    ///
    /// Requires that require_auth middleware has run.
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Get Session from request extensions
        parts
            .extensions
            .get::<Session>()
            .cloned()
            .map(CurrentUser)
            .ok_or(AppError::Unauthorized)
    }
}

/// Optional current user extractor
///
/// Returns None if not authenticated, instead of error.
#[derive(Debug, Clone)]
pub struct MaybeUser(pub Option<Session>);

#[async_trait]
impl<S> FromRequestParts<S> for MaybeUser
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Get Session from extensions, return None if missing
        let session = parts.extensions.get::<Session>().cloned();
        Ok(MaybeUser(session))
    }
}
