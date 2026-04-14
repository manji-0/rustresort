//! Authentication middleware
//!
//! Protects routes that require authentication.

use axum::{
    async_trait,
    extract::{FromRequestParts, State},
    http::{HeaderMap, Request, request::Parts},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::CookieJar;
use chrono::{Duration, Utc};
use rustresort_models::OAuthToken;
use std::sync::Arc;

use super::session::{Session, verify_session_token};
use crate::AuthState;
use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct OAuthAccess {
    pub token_id: String,
    pub app_id: String,
    pub scopes: Vec<String>,
    pub grant_type: String,
}

#[derive(Debug, Clone, Copy)]
pub enum ScopePolicy {
    Any(&'static [&'static str]),
    All(&'static [&'static str]),
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(ToOwned::to_owned)
}

fn session_from_oauth_token(
    account: crate::data::Account,
    token: &OAuthToken,
    session_max_age: i64,
) -> Session {
    let now = Utc::now();
    let expires_at = token
        .expires_at
        .min(now + Duration::seconds(session_max_age.max(60)));
    Session {
        username: account.username,
        display_name: account.display_name,
        auth_method: "oauth".to_string(),
        created_at: token.created_at,
        expires_at,
    }
}

fn oauth_grant_represents_user_session(grant_type: &str) -> bool {
    matches!(grant_type, "authorization_code" | "refresh_token")
}

async fn authenticate_oauth_bearer_token(
    state: &AuthState,
    token: &str,
) -> Result<(Session, Option<OAuthAccess>), AppError> {
    let oauth_token = state
        .db
        .get_oauth_token(token)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let account = state
        .db
        .get_account()
        .await?
        .ok_or(AppError::Unauthorized)?;
    let scopes = oauth_token
        .scopes
        .split_whitespace()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let oauth_access = OAuthAccess {
        token_id: oauth_token.id.clone(),
        app_id: oauth_token.app_id.clone(),
        scopes,
        grant_type: oauth_token.grant_type.clone(),
    };
    let session = session_from_oauth_token(
        account,
        &oauth_token,
        state.config.auth.session_max_age as i64,
    );
    Ok((session, Some(oauth_access)))
}

async fn authenticate_user_oauth_bearer_token(
    state: &AuthState,
    token: &str,
) -> Result<(Session, Option<OAuthAccess>), AppError> {
    let (session, oauth_access) = authenticate_oauth_bearer_token(state, token).await?;
    let Some(oauth_access) = oauth_access else {
        return Err(AppError::Unauthorized);
    };
    if !oauth_grant_represents_user_session(&oauth_access.grant_type) {
        return Err(AppError::Forbidden);
    }
    Ok((session, Some(oauth_access)))
}

async fn authenticate_request(
    state: &AuthState,
    bearer_token: Option<String>,
    cookie_token: Option<String>,
) -> Result<(Session, Option<OAuthAccess>), AppError> {
    if let Some(token) = bearer_token.as_deref() {
        return authenticate_user_oauth_bearer_token(state, token).await;
    }

    if let Some(cookie_token) = cookie_token.as_deref() {
        let session = verify_session_token(cookie_token, &state.config.auth.session_secret)?;
        return Ok((session, None));
    }

    Err(AppError::Unauthorized)
}

pub async fn require_app_auth(
    State(state): State<AuthState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let token = bearer_token(request.headers()).ok_or(AppError::Unauthorized)?;
    let (_, oauth_access) = authenticate_oauth_bearer_token(&state, &token).await?;
    let mut request = request;
    if let Some(oauth_access) = oauth_access {
        request.extensions_mut().insert(oauth_access);
    }
    Ok(next.run(request).await)
}

fn scope_matches(granted: &str, required: &str) -> bool {
    granted == required
        || required
            .split_once(':')
            .map(|(prefix, _)| granted == prefix)
            .unwrap_or(false)
}

fn oauth_scopes_satisfy(granted: &[String], required: &[&str], require_all: bool) -> bool {
    if required.is_empty() {
        return true;
    }
    let matches_required = |required_scope: &&str| {
        granted
            .iter()
            .any(|granted_scope| scope_matches(granted_scope, required_scope))
    };
    if require_all {
        required.iter().all(matches_required)
    } else {
        required.iter().any(matches_required)
    }
}

/// Middleware to require session authentication only.
///
/// Accepts signed session tokens from Authorization bearer or session cookie.
pub async fn require_session_auth(
    State(config): State<Arc<crate::config::AppConfig>>,
    jar: CookieJar,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let bearer_token = request
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));

    if let Some(token) = bearer_token {
        let session = verify_session_token(token, &config.auth.session_secret)?;
        request.extensions_mut().insert(session);
    } else if let Some(cookie_token) = jar.get("session").map(|cookie| cookie.value()) {
        let session = verify_session_token(cookie_token, &config.auth.session_secret)?;
        request.extensions_mut().insert(session);
    } else {
        return Err(AppError::Unauthorized);
    }

    Ok(next.run(request).await)
}

/// Middleware for `/metrics` static bearer-token protection.
///
/// If `metrics.auth_token` is unset, access remains anonymous for backward compatibility.
pub async fn require_metrics_auth(
    State(config): State<Arc<crate::config::AppConfig>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let Some(expected_token) = config
        .metrics
        .auth_token
        .as_deref()
        .filter(|token| !token.is_empty())
    else {
        return Ok(next.run(request).await);
    };

    let provided = request
        .headers()
        .get("Authorization")
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.strip_prefix("Bearer "));

    if provided.is_some_and(|token| {
        use subtle::ConstantTimeEq;
        token.as_bytes().ct_eq(expected_token.as_bytes()).into()
    }) {
        return Ok(next.run(request).await);
    }

    Err(AppError::Unauthorized)
}

/// Middleware to require authentication
///
/// Extracts and verifies OAuth bearer tokens or local session cookies.
/// Adds Session to request extensions if valid.
///
/// # Usage
/// ```ignore
/// let protected_routes = Router::new()
///     .route("/api/v1/...", ...)
///     .layer(middleware::from_fn_with_state(state, require_auth));
/// ```
pub async fn require_auth(
    State(state): State<AuthState>,
    jar: CookieJar,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let bearer_token = bearer_token(request.headers());
    let cookie_token = jar.get("session").map(|cookie| cookie.value().to_string());
    let (session, oauth_access) = authenticate_request(&state, bearer_token, cookie_token).await?;
    request.extensions_mut().insert(session);
    if let Some(oauth_access) = oauth_access {
        request.extensions_mut().insert(oauth_access);
    }

    Ok(next.run(request).await)
}

/// Middleware to require authentication and enforce OAuth scopes.
///
/// API bearer authentication is OAuth-only. Local signed sessions are accepted
/// via the `session` cookie for built-in browser flows.
pub async fn require_auth_scopes(
    State((state, policy)): State<(AuthState, ScopePolicy)>,
    jar: CookieJar,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    require_auth_scopes_with_policy(state, policy, jar, request, next).await
}

pub async fn require_auth_scopes_with_policy(
    state: AuthState,
    policy: ScopePolicy,
    jar: CookieJar,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let bearer_token = bearer_token(request.headers());
    let cookie_token = jar.get("session").map(|cookie| cookie.value().to_string());
    let (session, oauth_access) = authenticate_request(&state, bearer_token, cookie_token).await?;
    request.extensions_mut().insert(session);

    if let Some(oauth_access) = oauth_access {
        let (required, require_all) = match policy {
            ScopePolicy::Any(required) => (required, false),
            ScopePolicy::All(required) => (required, true),
        };
        if !oauth_scopes_satisfy(&oauth_access.scopes, required, require_all) {
            return Err(AppError::Forbidden);
        }
        request.extensions_mut().insert(oauth_access);
    }

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
///     format!("Hello, {}", session.username)
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

#[derive(Debug, Clone)]
pub struct CurrentOAuthAccess(pub OAuthAccess);

#[async_trait]
impl<S> FromRequestParts<S> for CurrentOAuthAccess
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<OAuthAccess>()
            .cloned()
            .map(CurrentOAuthAccess)
            .ok_or(AppError::Unauthorized)
    }
}
