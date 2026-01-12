//! GitHub OAuth flow
//!
//! Implements the OAuth 2.0 authorization code flow with GitHub.

use axum::{
    Router,
    extract::{Query, State},
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;

use crate::AppState;
use crate::error::AppError;

/// Create authentication router
///
/// Routes:
/// - GET /login - Login page
/// - GET /auth/github - Redirect to GitHub
/// - GET /auth/github/callback - OAuth callback
/// - POST /logout - Logout
pub fn auth_router() -> Router<AppState> {
    Router::new()
        .route("/login", get(login_page))
        .route("/auth/github", get(github_redirect))
        .route("/auth/github/callback", get(github_callback))
        .route("/logout", axum::routing::post(logout))
}

// =============================================================================
// Login Page
// =============================================================================

/// GET /login
///
/// Renders a simple login page with GitHub sign-in button.
async fn login_page() -> impl IntoResponse {
    Html(
        r#"
        <!DOCTYPE html>
        <html>
        <head><title>Login - RustResort</title></head>
        <body>
            <h1>RustResort</h1>
            <p>Please sign in with GitHub</p>
            <a href="/auth/github">Sign in with GitHub</a>
        </body>
        </html>
    "#,
    )
}

// =============================================================================
// GitHub OAuth
// =============================================================================

/// GET /auth/github
///
/// Redirects user to GitHub authorization page.
///
/// # Steps
/// 1. Generate CSRF state token
/// 2. Store state in cookie
/// 3. Redirect to GitHub with client_id, redirect_uri, scope, state
async fn github_redirect(
    State(_state): State<AppState>,
    _jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    // TODO: Implement GitHub OAuth redirect
    Ok(Redirect::to("/login"))
}

/// Query parameters from GitHub callback
#[derive(Debug, Deserialize)]
struct GitHubCallbackQuery {
    /// Authorization code
    code: String,
    /// CSRF state token
    state: String,
}

/// GitHub token response
#[derive(Debug, Deserialize)]
struct GitHubTokenResponse {
    access_token: String,
    token_type: String,
    scope: String,
}

/// GitHub user info
#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
    id: u64,
    avatar_url: String,
    name: Option<String>,
}

/// GET /auth/github/callback
///
/// Handles OAuth callback from GitHub.
///
/// # Steps
/// 1. Verify CSRF state
/// 2. Exchange code for access token
/// 3. Fetch user info from GitHub
/// 4. Verify username matches configured admin
/// 5. Create session and set cookie
/// 6. Redirect to home
async fn github_callback(
    State(_state): State<AppState>,
    Query(_query): Query<GitHubCallbackQuery>,
    _jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    // TODO: Implement OAuth callback
    Ok(Redirect::to("/login"))
}

// =============================================================================
// Logout
// =============================================================================

/// POST /logout
///
/// Clears session cookie and redirects to login.
async fn logout(_jar: CookieJar) -> impl IntoResponse {
    // TODO: Clear session cookie
    Redirect::to("/login")
}

// =============================================================================
// Helpers
// =============================================================================

/// Generate a random CSRF state token
fn generate_csrf_state() -> String {
    // TODO: Generate proper CSRF token
    "placeholder_state".to_string()
}

/// Verify CSRF state from cookie matches callback state
fn verify_csrf_state(_state: &str, _jar: &CookieJar) -> Result<(), AppError> {
    // TODO: Implement CSRF verification
    Ok(())
}
