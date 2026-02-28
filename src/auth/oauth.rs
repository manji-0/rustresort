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
use axum_extra::extract::cookie::{Cookie, SameSite};
use base64::Engine;
use chrono::{Duration, Utc};
use rand::RngCore;
use serde::Deserialize;

use crate::AppState;
use crate::auth::session::{Session, create_session_token};
use crate::error::AppError;

const GITHUB_AUTHORIZE_ENDPOINT: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_ENDPOINT: &str = "https://github.com/login/oauth/access_token";
const GITHUB_USER_ENDPOINT: &str = "https://api.github.com/user";
const OAUTH_CSRF_COOKIE: &str = "oauth_state";
const SESSION_COOKIE: &str = "session";

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
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    let secure_cookies = state.config.should_use_secure_cookies();
    let csrf_state = generate_csrf_state();
    let auth_url = build_github_authorize_url(&state, &csrf_state)?;
    let csrf_cookie = build_csrf_cookie(&csrf_state, secure_cookies);

    Ok((jar.add(csrf_cookie), Redirect::to(&auth_url)))
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
    access_token: Option<String>,
    token_type: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
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
    State(state): State<AppState>,
    Query(query): Query<GitHubCallbackQuery>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    let secure_cookies = state.config.should_use_secure_cookies();
    verify_csrf_state(&query.state, &jar)?;

    let access_token = exchange_github_code(
        state.http_client.as_ref(),
        &state.config.auth.github.client_id,
        &state.config.auth.github.client_secret,
        &query.code,
    )
    .await?;

    let github_user = fetch_github_user(state.http_client.as_ref(), &access_token).await?;
    if !github_user
        .login
        .eq_ignore_ascii_case(&state.config.auth.github_username)
    {
        tracing::warn!(
            attempted_user = %github_user.login,
            allowed_user = %state.config.auth.github_username,
            "Unauthorized GitHub login attempt"
        );
        return Err(AppError::Unauthorized);
    }

    let now = Utc::now();
    let session = Session {
        github_username: github_user.login,
        github_id: github_user.id,
        avatar_url: github_user.avatar_url,
        name: github_user.name,
        created_at: now,
        expires_at: now + Duration::seconds(state.config.auth.session_max_age),
    };
    let session_token = create_session_token(&session, &state.config.auth.session_secret)?;

    let session_cookie = build_session_cookie(&session_token, secure_cookies);
    let clear_csrf_cookie = clear_cookie(OAUTH_CSRF_COOKIE, secure_cookies);

    let jar = jar.remove(clear_csrf_cookie).add(session_cookie);
    Ok((jar, Redirect::to("/")))
}

// =============================================================================
// Logout
// =============================================================================

/// POST /logout
///
/// Clears session cookie and redirects to login.
async fn logout(State(state): State<AppState>, _jar: CookieJar) -> impl IntoResponse {
    let secure_cookies = state.config.should_use_secure_cookies();
    let clear_session = clear_cookie(SESSION_COOKIE, secure_cookies);
    let clear_csrf = clear_cookie(OAUTH_CSRF_COOKIE, secure_cookies);
    (
        _jar.remove(clear_session).remove(clear_csrf),
        Redirect::to("/login"),
    )
}

// =============================================================================
// Helpers
// =============================================================================

/// Generate a random CSRF state token
fn generate_csrf_state() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Verify CSRF state from cookie matches callback state
fn verify_csrf_state(state: &str, jar: &CookieJar) -> Result<(), AppError> {
    let expected = jar
        .get(OAUTH_CSRF_COOKIE)
        .map(|cookie| cookie.value())
        .ok_or(AppError::Unauthorized)?;

    if state.is_empty() || state != expected {
        return Err(AppError::Unauthorized);
    }

    Ok(())
}

fn build_github_authorize_url(state: &AppState, csrf_state: &str) -> Result<String, AppError> {
    let redirect_uri = format!("{}/auth/github/callback", state.config.server.base_url());
    let mut url = url::Url::parse(GITHUB_AUTHORIZE_ENDPOINT)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid GitHub authorize URL: {e}")))?;

    url.query_pairs_mut()
        .append_pair("client_id", &state.config.auth.github.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", "read:user")
        .append_pair("state", csrf_state);

    Ok(url.to_string())
}

fn build_csrf_cookie(state: &str, secure: bool) -> Cookie<'static> {
    Cookie::build((OAUTH_CSRF_COOKIE, state.to_string()))
        .path("/")
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .build()
}

fn build_session_cookie(session_token: &str, secure: bool) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, session_token.to_string()))
        .path("/")
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .build()
}

fn clear_cookie(name: &'static str, secure: bool) -> Cookie<'static> {
    let mut cookie = Cookie::build((name, "".to_string()))
        .path("/")
        .http_only(true)
        .secure(secure)
        .build();
    cookie.make_removal();
    cookie
}

async fn exchange_github_code(
    http_client: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    code: &str,
) -> Result<String, AppError> {
    let token_response = http_client
        .post(GITHUB_TOKEN_ENDPOINT)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
        ])
        .send()
        .await?;

    if !token_response.status().is_success() {
        return Err(AppError::Unauthorized);
    }

    let payload: GitHubTokenResponse = token_response.json().await?;
    if payload.error.is_some() {
        tracing::warn!(
            error = ?payload.error,
            error_description = ?payload.error_description,
            "GitHub token exchange returned an error"
        );
        return Err(AppError::Unauthorized);
    }

    let token_type = payload.token_type.unwrap_or_default();
    if !token_type.eq_ignore_ascii_case("bearer") {
        return Err(AppError::Unauthorized);
    }

    payload.access_token.ok_or(AppError::Unauthorized)
}

async fn fetch_github_user(
    http_client: &reqwest::Client,
    access_token: &str,
) -> Result<GitHubUser, AppError> {
    let response = http_client
        .get(GITHUB_USER_ENDPOINT)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .header("User-Agent", "RustResort")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(AppError::Unauthorized);
    }

    response.json::<GitHubUser>().await.map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csrf_state_generation_returns_urlsafe_token() {
        let state = generate_csrf_state();
        assert!(!state.is_empty());
        assert!(!state.contains('+'));
        assert!(!state.contains('/'));
        assert!(!state.contains('='));
    }

    #[test]
    fn verify_csrf_state_accepts_matching_cookie_value() {
        let jar = CookieJar::new().add(
            Cookie::build((OAUTH_CSRF_COOKIE, "state123".to_string()))
                .path("/")
                .build(),
        );
        assert!(verify_csrf_state("state123", &jar).is_ok());
    }

    #[test]
    fn verify_csrf_state_rejects_mismatch() {
        let jar = CookieJar::new().add(
            Cookie::build((OAUTH_CSRF_COOKIE, "state123".to_string()))
                .path("/")
                .build(),
        );
        assert!(verify_csrf_state("other", &jar).is_err());
    }

    #[test]
    fn build_session_cookie_sets_secure_attributes() {
        let cookie = build_session_cookie("token", true);
        assert_eq!(cookie.secure(), Some(true));
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
    }

    #[test]
    fn build_csrf_cookie_sets_expected_attributes() {
        let cookie = build_csrf_cookie("state", true);
        assert_eq!(cookie.secure(), Some(true));
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
    }
}
