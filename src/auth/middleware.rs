//! Authentication middleware
//!
//! Protects routes that require authentication.

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, State},
    http::{HeaderMap, Request, request::Parts},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::CookieJar;
use chrono::{Duration, Utc};

use super::session::{Session, verify_session_token};
use crate::AppState;
use crate::error::AppError;

fn extract_token_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(ToOwned::to_owned)
        .or_else(|| {
            let jar = CookieJar::from_headers(headers);
            jar.get("session").map(|cookie| cookie.value().to_owned())
        })
}

async fn authenticate_token(token: &str, state: &AppState) -> Result<Session, AppError> {
    if let Ok(session) = verify_session_token(token, &state.config.auth.session_secret) {
        return Ok(session);
    }

    if state.db.get_oauth_token(token).await?.is_some() {
        let now = Utc::now();
        return Ok(Session {
            github_username: state.config.auth.github_username.clone(),
            github_id: 0,
            avatar_url: String::new(),
            name: Some(state.config.admin.display_name.clone()),
            created_at: now,
            expires_at: now + Duration::seconds(state.config.auth.session_max_age),
        });
    }

    Err(AppError::Unauthorized)
}

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
    _jar: CookieJar,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let token = extract_token_from_headers(request.headers()).ok_or(AppError::Unauthorized)?;

    // Verify token and get session
    let session = authenticate_token(&token, &state).await?;

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
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    /// Extract current user from request
    ///
    /// Accepts both session tokens and OAuth access tokens.
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if let Some(session) = parts.extensions.get::<Session>().cloned() {
            return Ok(CurrentUser(session));
        }

        let state = AppState::from_ref(_state);
        let token = extract_token_from_headers(&parts.headers).ok_or(AppError::Unauthorized)?;
        let session = authenticate_token(&token, &state).await?;
        parts.extensions.insert(session.clone());

        Ok(CurrentUser(session))
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
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        if let Some(session) = parts.extensions.get::<Session>().cloned() {
            return Ok(MaybeUser(Some(session)));
        }

        let app_state = AppState::from_ref(state);
        let session = match extract_token_from_headers(&parts.headers) {
            Some(token) => authenticate_token(&token, &app_state).await.ok(),
            None => None,
        };

        if let Some(session) = &session {
            parts.extensions.insert(session.clone());
        }

        Ok(MaybeUser(session))
    }
}
