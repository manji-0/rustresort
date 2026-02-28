//! Apps and OAuth endpoints

use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Json, Redirect, Response},
};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use base64::Engine;
use chrono::Utc;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use url::Url;

use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::AppError;

const OAUTH_AUTHORIZE_CONFIRM_COOKIE_PREFIX: &str = "oauth_authorize_confirm_";
const OAUTH_ACCESS_TOKEN_TTL_SECONDS: i64 = 7_200;
/// App registration request
#[derive(Debug, Deserialize)]
pub struct CreateAppRequest {
    pub client_name: String,
    pub redirect_uris: String,
    pub scopes: Option<String>,
    pub website: Option<String>,
}

/// App response
#[derive(Debug, Serialize)]
pub struct AppResponse {
    pub id: String,
    pub name: String,
    pub website: Option<String>,
    pub redirect_uri: String,
    pub client_id: String,
    pub client_secret: String,
    pub vapid_key: Option<String>,
}

/// OAuth token request
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    pub code: Option<String>,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: Option<String>,
    pub scope: Option<String>,
}

/// OAuth authorize request query
#[derive(Debug, Deserialize)]
pub struct AuthorizeRequest {
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub approve: Option<bool>,
    pub confirm: Option<String>,
}

/// OAuth token response
#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub scope: String,
    pub expires_in: i64,
    pub created_at: i64,
}

fn normalize_scopes(scopes: &str) -> String {
    scopes.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn scopes_are_subset(requested: &str, allowed: &str) -> bool {
    let requested_set: HashSet<&str> = requested.split_whitespace().collect();
    let allowed_set: HashSet<&str> = allowed.split_whitespace().collect();
    requested_set.is_subset(&allowed_set)
}

fn is_registered_redirect_uri(registered_redirect_uris: &str, redirect_uri: &str) -> bool {
    registered_redirect_uris
        .split_whitespace()
        .any(|registered| registered == redirect_uri)
}

fn build_authorize_redirect_location(
    redirect_uri: &str,
    code: &str,
    state: Option<&str>,
) -> String {
    if let Ok(mut redirect) = Url::parse(redirect_uri) {
        let mut serializer =
            url::form_urlencoded::Serializer::new(redirect.query().unwrap_or("").to_string());
        serializer.append_pair("code", code);
        if let Some(state) = state {
            serializer.append_pair("state", state);
        }
        redirect.set_query(Some(&serializer.finish()));
        return redirect.to_string();
    }

    // Fallback for unexpected non-URL values.
    let separator = if redirect_uri.contains('?') { '&' } else { '?' };
    let mut location = format!(
        "{}{}code={}",
        redirect_uri,
        separator,
        urlencoding::encode(code)
    );
    if let Some(state) = state {
        location.push_str("&state=");
        location.push_str(&urlencoding::encode(state));
    }
    location
}

fn build_authorize_error_redirect_location(
    redirect_uri: &str,
    error: &str,
    state: Option<&str>,
) -> String {
    if let Ok(mut redirect) = Url::parse(redirect_uri) {
        let mut serializer =
            url::form_urlencoded::Serializer::new(redirect.query().unwrap_or("").to_string());
        serializer.append_pair("error", error);
        if let Some(state) = state {
            serializer.append_pair("state", state);
        }
        redirect.set_query(Some(&serializer.finish()));
        return redirect.to_string();
    }

    let separator = if redirect_uri.contains('?') { '&' } else { '?' };
    let mut location = format!(
        "{}{}error={}",
        redirect_uri,
        separator,
        urlencoding::encode(error)
    );
    if let Some(state) = state {
        location.push_str("&state=");
        location.push_str(&urlencoding::encode(state));
    }
    location
}

fn generate_authorize_confirm_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn authorize_confirm_cookie_name(confirm_token: &str) -> String {
    format!("{}{}", OAUTH_AUTHORIZE_CONFIRM_COOKIE_PREFIX, confirm_token)
}

fn build_authorize_confirm_cookie(confirm_token: &str, secure: bool) -> Cookie<'static> {
    Cookie::build((
        authorize_confirm_cookie_name(confirm_token),
        "1".to_string(),
    ))
    .path("/oauth/authorize")
    .http_only(true)
    .secure(secure)
    .same_site(SameSite::Lax)
    .build()
}

fn clear_authorize_confirm_cookie(confirm_token: &str) -> Cookie<'static> {
    let mut cookie = Cookie::build((authorize_confirm_cookie_name(confirm_token), "".to_string()))
        .path("/oauth/authorize")
        .http_only(true)
        .build();
    cookie.make_removal();
    cookie
}

fn escape_html_attr(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&#x27;".to_string(),
            _ => ch.to_string(),
        })
        .collect::<String>()
}

fn render_hidden_input(name: &str, value: &str) -> String {
    format!(
        "<input type=\"hidden\" name=\"{}\" value=\"{}\" />",
        escape_html_attr(name),
        escape_html_attr(value)
    )
}

fn render_authorize_consent_page(
    app_name: &str,
    client_id: &str,
    redirect_uri: &str,
    requested_scopes: &str,
    state: Option<&str>,
    confirm_token: &str,
) -> String {
    let escaped_app_name = html_escape::encode_text(app_name);
    let escaped_redirect_uri = html_escape::encode_text(redirect_uri);
    let escaped_scopes = html_escape::encode_text(requested_scopes);

    let shared_inputs = [
        render_hidden_input("response_type", "code"),
        render_hidden_input("client_id", client_id),
        render_hidden_input("redirect_uri", redirect_uri),
        render_hidden_input("scope", requested_scopes),
        render_hidden_input("confirm", confirm_token),
    ]
    .join("\n");
    let state_input = state
        .map(|value| render_hidden_input("state", value))
        .unwrap_or_default();

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Authorize Application</title>
</head>
<body>
  <h1>Authorize Application</h1>
  <p><strong>{}</strong> is requesting access to your RustResort account.</p>
  <p><strong>Redirect URI:</strong> {}</p>
  <p><strong>Requested scopes:</strong> {}</p>
  <form method="get" action="/oauth/authorize">
    {}
    {}
    {}
    <button type="submit">Authorize</button>
  </form>
  <form method="get" action="/oauth/authorize">
    {}
    {}
    {}
    <button type="submit">Deny</button>
  </form>
</body>
</html>"#,
        escaped_app_name,
        escaped_redirect_uri,
        escaped_scopes,
        shared_inputs,
        state_input,
        render_hidden_input("approve", "true"),
        shared_inputs,
        state_input,
        render_hidden_input("approve", "false"),
    )
}

struct AuthorizeContext {
    app_id: String,
    app_name: String,
    client_id: String,
    redirect_uri: String,
    requested_scopes: String,
}

async fn validate_authorize_request(
    state: &AppState,
    req: &AuthorizeRequest,
) -> Result<AuthorizeContext, AppError> {
    let response_type = req
        .response_type
        .as_deref()
        .ok_or_else(|| AppError::Validation("response_type is required".to_string()))?;
    if response_type != "code" {
        return Err(AppError::Validation(
            "response_type must be 'code'".to_string(),
        ));
    }

    let client_id = req
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Validation("client_id is required".to_string()))?;
    let redirect_uri = req
        .redirect_uri
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Validation("redirect_uri is required".to_string()))?;

    let app = state
        .db
        .get_oauth_app_by_client_id(client_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    if !is_registered_redirect_uri(&app.redirect_uri, redirect_uri) {
        return Err(AppError::Validation(
            "redirect_uri does not match registered redirect URI".to_string(),
        ));
    }

    let requested_scopes = req
        .scope
        .as_deref()
        .map(normalize_scopes)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| normalize_scopes(&app.scopes));
    if !scopes_are_subset(&requested_scopes, &app.scopes) {
        return Err(AppError::Unauthorized);
    }

    Ok(AuthorizeContext {
        app_id: app.id,
        app_name: app.name,
        client_id: app.client_id,
        redirect_uri: redirect_uri.to_string(),
        requested_scopes,
    })
}

async fn issue_authorization_code(
    state: &AppState,
    app_id: &str,
    redirect_uri: &str,
    requested_scopes: &str,
    oauth_state: Option<&str>,
) -> Result<Redirect, AppError> {
    use crate::data::{EntityId, OAuthAuthorizationCode};

    let code_value = EntityId::new().0;
    let authorization_code = OAuthAuthorizationCode {
        id: EntityId::new().0,
        app_id: app_id.to_string(),
        code: code_value.clone(),
        redirect_uri: redirect_uri.to_string(),
        scopes: requested_scopes.to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(10),
    };

    state
        .db
        .insert_oauth_authorization_code(&authorization_code)
        .await?;

    let location = build_authorize_redirect_location(redirect_uri, &code_value, oauth_state);
    Ok(Redirect::to(&location))
}

/// POST /api/v1/apps
pub async fn create_app(
    State(state): State<AppState>,
    Json(req): Json<CreateAppRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::data::{EntityId, OAuthApp};

    // Validate
    if req.client_name.is_empty() {
        return Err(AppError::Validation("client_name is required".to_string()));
    }

    if req.redirect_uris.is_empty() {
        return Err(AppError::Validation(
            "redirect_uris is required".to_string(),
        ));
    }

    // Generate app credentials
    let app_id = EntityId::new().0;
    let client_id = EntityId::new().0;
    let client_secret = EntityId::new().0;

    // Default scopes if not provided
    let scopes = req.scopes.unwrap_or_else(|| "read".to_string());

    // Create app
    let app = OAuthApp {
        id: app_id.clone(),
        name: req.client_name.clone(),
        website: req.website.clone(),
        redirect_uri: req.redirect_uris.clone(),
        client_id: client_id.clone(),
        client_secret: client_secret.clone(),
        scopes: scopes.clone(),
        created_at: Utc::now(),
    };

    // Save to database
    state.db.insert_oauth_app(&app).await?;

    // Return response
    let response = AppResponse {
        id: app.id,
        name: app.name,
        website: app.website,
        redirect_uri: app.redirect_uri,
        client_id: app.client_id,
        client_secret: app.client_secret,
        vapid_key: None, // TODO: Implement push notifications
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /oauth/authorize
pub async fn authorize(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(req): Query<AuthorizeRequest>,
) -> Result<Response, AppError> {
    let context = validate_authorize_request(&state, &req).await?;

    if let Some(confirm_token) = req.confirm.as_deref() {
        if confirm_token.is_empty() {
            return Err(AppError::Unauthorized);
        }
        let confirm_cookie_name = authorize_confirm_cookie_name(confirm_token);
        if jar.get(&confirm_cookie_name).is_none() {
            return Err(AppError::Unauthorized);
        }

        let clear_confirm_cookie = clear_authorize_confirm_cookie(confirm_token);
        let jar = jar.remove(clear_confirm_cookie);

        if !req.approve.unwrap_or(false) {
            let denied_location = build_authorize_error_redirect_location(
                &context.redirect_uri,
                "access_denied",
                req.state.as_deref(),
            );
            return Ok((jar, Redirect::to(&denied_location)).into_response());
        }

        let redirect = issue_authorization_code(
            &state,
            &context.app_id,
            &context.redirect_uri,
            &context.requested_scopes,
            req.state.as_deref(),
        )
        .await?;
        return Ok((jar, redirect).into_response());
    }

    let confirm_token = generate_authorize_confirm_token();
    let confirm_cookie =
        build_authorize_confirm_cookie(&confirm_token, state.config.server.protocol == "https");
    let consent_page = render_authorize_consent_page(
        &context.app_name,
        &context.client_id,
        &context.redirect_uri,
        &context.requested_scopes,
        req.state.as_deref(),
        &confirm_token,
    );

    Ok((jar.add(confirm_cookie), Html(consent_page)).into_response())
}

/// GET /api/v1/apps/verify_credentials
pub async fn verify_app_credentials(
    State(_state): State<AppState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get the app from the current session
    // For now, return a simple response
    // TODO: Implement proper app verification from session

    let response = serde_json::json!({
        "name": "RustResort",
        "website": null,
    });

    Ok(Json(response))
}

/// POST /oauth/token
pub async fn create_token(
    State(state): State<AppState>,
    Json(req): Json<TokenRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::data::{EntityId, OAuthToken};

    // Validate grant_type
    if req.grant_type != "client_credentials" && req.grant_type != "authorization_code" {
        return Err(AppError::Validation("Invalid grant_type".to_string()));
    }

    // Verify client credentials
    let app = state
        .db
        .get_oauth_app_by_client_id(&req.client_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    if app.client_secret != req.client_secret {
        return Err(AppError::Unauthorized);
    }

    let scopes = match req.grant_type.as_str() {
        "client_credentials" => {
            let requested_scopes = req
                .scope
                .as_deref()
                .map(normalize_scopes)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| normalize_scopes(&app.scopes));

            if !scopes_are_subset(&requested_scopes, &app.scopes) {
                return Err(AppError::Unauthorized);
            }

            requested_scopes
        }
        "authorization_code" => {
            let code = req
                .code
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    AppError::Validation(
                        "code is required for authorization_code grant".to_string(),
                    )
                })?;
            let redirect_uri = req
                .redirect_uri
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    AppError::Validation(
                        "redirect_uri is required for authorization_code grant".to_string(),
                    )
                })?;

            let authorization_code = state
                .db
                .consume_oauth_authorization_code(code, &app.id, redirect_uri, Utc::now())
                .await?
                .ok_or(AppError::Unauthorized)?;

            let requested_scopes = req
                .scope
                .as_deref()
                .map(normalize_scopes)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| normalize_scopes(&authorization_code.scopes));

            if !scopes_are_subset(&requested_scopes, &authorization_code.scopes)
                || !scopes_are_subset(&requested_scopes, &app.scopes)
            {
                return Err(AppError::Unauthorized);
            }

            requested_scopes
        }
        _ => {
            return Err(AppError::Validation("Invalid grant_type".to_string()));
        }
    };

    // Generate access token
    let token_id = EntityId::new().0;
    let access_token = EntityId::new().0;

    // Create token
    let issued_at = Utc::now();
    let token = OAuthToken {
        id: token_id.clone(),
        app_id: app.id.clone(),
        access_token: access_token.clone(),
        grant_type: req.grant_type.clone(),
        scopes,
        created_at: issued_at,
        expires_at: issued_at + chrono::Duration::seconds(OAUTH_ACCESS_TOKEN_TTL_SECONDS),
        revoked: false,
    };

    // Save to database
    state.db.insert_oauth_token(&token).await?;

    // Return response
    let response = TokenResponse {
        access_token: token.access_token,
        token_type: "Bearer".to_string(),
        scope: token.scopes,
        expires_in: OAUTH_ACCESS_TOKEN_TTL_SECONDS,
        created_at: token.created_at.timestamp(),
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /oauth/revoke
pub async fn revoke_token(
    State(state): State<AppState>,
    Json(req): Json<RevokeTokenRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify client credentials
    let app = state
        .db
        .get_oauth_app_by_client_id(&req.client_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    if app.client_secret != req.client_secret {
        return Err(AppError::Unauthorized);
    }

    // Revoke the token
    state.db.revoke_oauth_token(&req.token).await?;

    Ok(Json(serde_json::json!({})))
}

#[derive(Debug, Deserialize)]
pub struct RevokeTokenRequest {
    pub client_id: String,
    pub client_secret: String,
    pub token: String,
}

#[cfg(test)]
mod tests {
    use super::{authorize_confirm_cookie_name, build_authorize_confirm_cookie};

    #[test]
    fn authorize_confirm_cookie_name_is_token_scoped() {
        let first = authorize_confirm_cookie_name("token-a");
        let second = authorize_confirm_cookie_name("token-b");
        assert_ne!(first, second);
        assert_eq!(first, "oauth_authorize_confirm_token-a");
    }

    #[test]
    fn build_authorize_confirm_cookie_uses_token_scoped_name() {
        let cookie = build_authorize_confirm_cookie("token-a", false);
        assert_eq!(cookie.name(), "oauth_authorize_confirm_token-a");
    }
}
