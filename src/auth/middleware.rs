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
use chrono::{Duration, Utc};
use std::collections::HashSet;
use std::sync::Arc;

use super::session::{Session, verify_session_token};
use crate::AuthState;
use crate::error::AppError;

/// OAuth scope requirement attached to a route definition.
#[derive(Debug, Clone, Copy)]
pub struct OAuthScopeRequirement(pub &'static [&'static str]);

/// OAuth scope requirement that enforces all declared scopes.
#[derive(Debug, Clone, Copy)]
pub struct OAuthScopeAllRequirement(pub &'static [&'static str]);

#[derive(Debug, Clone, Copy)]
enum OAuthScopeMatch {
    Any(&'static [&'static str]),
    All(&'static [&'static str]),
}

impl OAuthScopeMatch {
    fn scopes(self) -> &'static [&'static str] {
        match self {
            Self::Any(scopes) | Self::All(scopes) => scopes,
        }
    }
}

fn normalize_mastodon_path(path: &str) -> &str {
    path.strip_prefix("/api").unwrap_or(path)
}

fn required_oauth_scopes(request: &Request<axum::body::Body>) -> Option<OAuthScopeMatch> {
    if let Some(requirement) = request.extensions().get::<OAuthScopeAllRequirement>() {
        return Some(OAuthScopeMatch::All(requirement.0));
    }

    request
        .extensions()
        .get::<OAuthScopeRequirement>()
        .map(|requirement| OAuthScopeMatch::Any(requirement.0))
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

fn has_any_required_scope(scope_set: &HashSet<String>, required_scopes: &[&str]) -> bool {
    required_scopes
        .iter()
        .any(|required| scope_grants(scope_set, required))
}

fn has_all_required_scopes(scope_set: &HashSet<String>, required_scopes: &[&str]) -> bool {
    required_scopes
        .iter()
        .all(|required| scope_grants(scope_set, required))
}

fn parse_scope_set(scopes: &str) -> HashSet<String> {
    scopes
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn build_oauth_session(state: &AuthState) -> Session {
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
    State(state): State<AuthState>,
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
            if let Some(scope_requirement) = required_oauth_scopes(&request) {
                let required_scopes = scope_requirement.scopes();
                // Empty required scope list means session-only endpoint.
                if required_scopes.is_empty() {
                    return Err(AppError::Forbidden);
                }
                let has_scope = match scope_requirement {
                    OAuthScopeMatch::Any(scopes) => has_any_required_scope(&scope_set, scopes),
                    OAuthScopeMatch::All(scopes) => has_all_required_scopes(&scope_set, scopes),
                };
                if !has_scope {
                    return Err(AppError::Forbidden);
                }
            } else {
                // Fail closed for OAuth-protected Mastodon API endpoints that
                // forgot to declare route-level scope metadata.
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
