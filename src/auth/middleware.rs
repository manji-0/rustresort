//! Authentication middleware
//!
//! Protects routes that require authentication.

use axum::{
    async_trait,
    extract::{FromRequestParts, State},
    http::{Method, Request, request::Parts},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::CookieJar;
use chrono::{Duration, Utc};
use std::collections::HashSet;

use super::session::{Session, verify_session_token};
use crate::AppState;
use crate::error::AppError;

fn normalize_mastodon_path(path: &str) -> &str {
    path.strip_prefix("/api").unwrap_or(path)
}

fn path_matches(pattern: &str, path: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
    let path_segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if pattern_segments.len() != path_segments.len() {
        return false;
    }

    pattern_segments
        .iter()
        .zip(path_segments.iter())
        .all(|(pattern_segment, path_segment)| {
            pattern_segment.starts_with(':') || pattern_segment == path_segment
        })
}

fn required_oauth_scopes(method: &Method, path: &str) -> Option<&'static [&'static str]> {
    const READ_ACCOUNTS: &[&str] = &["read:accounts"];
    const WRITE_ACCOUNTS: &[&str] = &["write:accounts"];
    const FOLLOW: &[&str] = &["follow"];
    const READ_STATUSES: &[&str] = &["read:statuses"];
    const WRITE_STATUSES: &[&str] = &["write:statuses"];
    const WRITE_FAVOURITES: &[&str] = &["write:favourites"];
    const READ_NOTIFICATIONS: &[&str] = &["read:notifications"];
    const WRITE_NOTIFICATIONS: &[&str] = &["write:notifications"];
    const WRITE_MEDIA: &[&str] = &["write:media"];
    const READ_LISTS: &[&str] = &["read:lists"];
    const WRITE_LISTS: &[&str] = &["write:lists"];
    const READ_FILTERS: &[&str] = &["read:filters"];
    const WRITE_FILTERS: &[&str] = &["write:filters"];
    const READ_SEARCH: &[&str] = &["read:search"];

    let path = normalize_mastodon_path(path);
    if path.starts_with("/v1/admin/") {
        // Mastodon admin endpoints are reserved for session-authenticated admin access.
        return Some(&[]);
    }

    match *method {
        Method::GET => {
            if path_matches("/v1/apps/verify_credentials", path)
                || path_matches("/v1/accounts/verify_credentials", path)
                || path_matches("/v1/accounts/:id/followers", path)
                || path_matches("/v1/accounts/:id/following", path)
                || path_matches("/v1/accounts/relationships", path)
                || path_matches("/v1/accounts/search", path)
                || path_matches("/v1/accounts/:id/lists", path)
                || path_matches("/v1/accounts/:id/identity_proofs", path)
                || path_matches("/v1/blocks", path)
                || path_matches("/v1/mutes", path)
                || path_matches("/v1/follow_requests", path)
                || path_matches("/v1/follow_requests/:id", path)
            {
                Some(READ_ACCOUNTS)
            } else if path_matches("/v1/accounts/:id/statuses", path)
                || path_matches("/v1/statuses/:id/source", path)
                || path_matches("/v1/statuses/:id/history", path)
                || path_matches("/v1/timelines/home", path)
                || path_matches("/v1/timelines/tag/:hashtag", path)
                || path_matches("/v1/timelines/list/:list_id", path)
                || path_matches("/v1/media/:id", path)
                || path_matches("/v1/bookmarks", path)
                || path_matches("/v1/favourites", path)
                || path_matches("/v1/polls/:id", path)
                || path_matches("/v1/scheduled_statuses", path)
                || path_matches("/v1/scheduled_statuses/:id", path)
                || path_matches("/v1/conversations", path)
                || path_matches("/v1/streaming/health", path)
                || path_matches("/v1/streaming/user", path)
                || path_matches("/v1/streaming/public", path)
                || path_matches("/v1/streaming/public/local", path)
                || path_matches("/v1/streaming/hashtag", path)
                || path_matches("/v1/streaming/list", path)
            {
                Some(READ_STATUSES)
            } else if path_matches("/v1/notifications", path)
                || path_matches("/v1/notifications/:id", path)
                || path_matches("/v1/notifications/unread_count", path)
                || path_matches("/v1/streaming/direct", path)
            {
                Some(READ_NOTIFICATIONS)
            } else if path_matches("/v1/lists", path)
                || path_matches("/v1/lists/:id", path)
                || path_matches("/v1/lists/:id/accounts", path)
            {
                Some(READ_LISTS)
            } else if path_matches("/v1/filters", path)
                || path_matches("/v1/filters/:id", path)
                || path_matches("/v2/filters", path)
            {
                Some(READ_FILTERS)
            } else if path_matches("/v1/search", path) || path_matches("/v2/search", path) {
                Some(READ_SEARCH)
            } else {
                None
            }
        }
        Method::PATCH => {
            if path_matches("/v1/accounts/update_credentials", path) {
                Some(WRITE_ACCOUNTS)
            } else {
                None
            }
        }
        Method::POST => {
            if path_matches("/v1/accounts/:id/follow", path)
                || path_matches("/v1/accounts/:id/unfollow", path)
                || path_matches("/v1/follow_requests/:id/authorize", path)
                || path_matches("/v1/follow_requests/:id/reject", path)
            {
                Some(FOLLOW)
            } else if path_matches("/v1/accounts/:id/block", path)
                || path_matches("/v1/accounts/:id/unblock", path)
                || path_matches("/v1/accounts/:id/mute", path)
                || path_matches("/v1/accounts/:id/unmute", path)
            {
                Some(WRITE_ACCOUNTS)
            } else if path_matches("/v1/statuses", path)
                || path_matches("/v1/statuses/:id/reblog", path)
                || path_matches("/v1/statuses/:id/unreblog", path)
                || path_matches("/v1/statuses/:id/bookmark", path)
                || path_matches("/v1/statuses/:id/unbookmark", path)
                || path_matches("/v1/statuses/:id/pin", path)
                || path_matches("/v1/statuses/:id/unpin", path)
                || path_matches("/v1/statuses/:id/mute", path)
                || path_matches("/v1/statuses/:id/unmute", path)
                || path_matches("/v1/polls/:id/votes", path)
                || path_matches("/v1/conversations/:id/read", path)
            {
                Some(WRITE_STATUSES)
            } else if path_matches("/v1/statuses/:id/favourite", path)
                || path_matches("/v1/statuses/:id/unfavourite", path)
            {
                Some(WRITE_FAVOURITES)
            } else if path_matches("/v1/notifications/:id/dismiss", path)
                || path_matches("/v1/notifications/clear", path)
            {
                Some(WRITE_NOTIFICATIONS)
            } else if path_matches("/v1/media", path) || path_matches("/v2/media", path) {
                Some(WRITE_MEDIA)
            } else if path_matches("/v1/lists", path)
                || path_matches("/v1/lists/:id/accounts", path)
            {
                Some(WRITE_LISTS)
            } else if path_matches("/v1/filters", path) {
                Some(WRITE_FILTERS)
            } else {
                None
            }
        }
        Method::PUT => {
            if path_matches("/v1/statuses/:id", path)
                || path_matches("/v1/scheduled_statuses/:id", path)
            {
                Some(WRITE_STATUSES)
            } else if path_matches("/v1/media/:id", path) {
                Some(WRITE_MEDIA)
            } else if path_matches("/v1/lists/:id", path) {
                Some(WRITE_LISTS)
            } else if path_matches("/v1/filters/:id", path) {
                Some(WRITE_FILTERS)
            } else {
                None
            }
        }
        Method::DELETE => {
            if path_matches("/v1/statuses/:id", path)
                || path_matches("/v1/scheduled_statuses/:id", path)
                || path_matches("/v1/conversations/:id", path)
            {
                Some(WRITE_STATUSES)
            } else if path_matches("/v1/lists/:id", path)
                || path_matches("/v1/lists/:id/accounts", path)
            {
                Some(WRITE_LISTS)
            } else if path_matches("/v1/filters/:id", path) {
                Some(WRITE_FILTERS)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn scope_grants(scope_set: &HashSet<String>, required: &str) -> bool {
    if scope_set.contains(required) {
        return true;
    }

    if required.starts_with("read:") && scope_set.contains("read") {
        return true;
    }
    if required.starts_with("write:") && scope_set.contains("write") {
        return true;
    }

    false
}

fn has_required_scope(scope_set: &HashSet<String>, required_scopes: &[&str]) -> bool {
    required_scopes
        .iter()
        .any(|required| scope_grants(scope_set, required))
}

fn parse_scope_set(scopes: &str) -> HashSet<String> {
    scopes
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn build_oauth_session(state: &AppState) -> Session {
    let now = Utc::now();
    Session {
        github_username: state.config.auth.github_username.clone(),
        github_id: 0,
        avatar_url: String::new(),
        name: Some(state.config.admin.display_name.clone()),
        created_at: now,
        expires_at: now + Duration::seconds(state.config.auth.session_max_age),
    }
}

/// Middleware to require session authentication only.
///
/// Accepts signed session tokens from Authorization bearer or session cookie.
/// OAuth bearer tokens are rejected by this middleware.
pub async fn require_session_auth(
    State(state): State<AppState>,
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
        let session = verify_session_token(token, &state.config.auth.session_secret)?;
        request.extensions_mut().insert(session);
    } else if let Some(cookie_token) = jar.get("session").map(|cookie| cookie.value()) {
        let session = verify_session_token(cookie_token, &state.config.auth.session_secret)?;
        request.extensions_mut().insert(session);
    } else {
        return Err(AppError::Unauthorized);
    }

    Ok(next.run(request).await)
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
    jar: CookieJar,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    // Try to get token from Authorization header first.
    let bearer_token = request
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));

    if let Some(token) = bearer_token {
        if let Ok(session) = verify_session_token(token, &state.config.auth.session_secret) {
            request.extensions_mut().insert(session);
        } else if let Some(oauth_token) = state.db.get_oauth_token(token).await? {
            if oauth_token.grant_type != "authorization_code" {
                return Err(AppError::Unauthorized);
            }

            let scope_set = parse_scope_set(&oauth_token.scopes);
            if let Some(required_scopes) =
                required_oauth_scopes(request.method(), request.uri().path())
            {
                // Empty required scope list means session-only endpoint.
                if required_scopes.is_empty() {
                    return Err(AppError::Forbidden);
                }
                if !has_required_scope(&scope_set, required_scopes) {
                    return Err(AppError::Forbidden);
                }
            } else {
                // Fail closed for unmapped OAuth-protected Mastodon API endpoints.
                let normalized_path = normalize_mastodon_path(request.uri().path());
                if normalized_path.starts_with("/v1/") || normalized_path.starts_with("/v2/") {
                    return Err(AppError::Forbidden);
                }
            }

            request.extensions_mut().insert(build_oauth_session(&state));
        } else {
            return Err(AppError::Unauthorized);
        }
    } else if let Some(cookie_token) = jar.get("session").map(|cookie| cookie.value()) {
        let session = verify_session_token(cookie_token, &state.config.auth.session_secret)?;
        request.extensions_mut().insert(session);
    } else {
        return Err(AppError::Unauthorized);
    }

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
