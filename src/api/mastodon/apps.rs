//! Mastodon app registration and OAuth endpoints.

use axum::{
    body::Bytes,
    extract::{ConnectInfo, OriginalUri, Query, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Json, Redirect, Response},
};
use axum_extra::extract::CookieJar;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::Digest;
use std::collections::HashSet;
use std::net::SocketAddr;
use subtle::ConstantTimeEq;
use url::Url;

use crate::AppsApiState;
use crate::auth::verify_session_token;
use crate::error::AppError;

const OAUTH_ACCESS_TOKEN_TTL_SECONDS: i64 = 7_200;
const OAUTH_CLIENT_SECRET_HASH_PREFIX: &str = "sha256:";
const OOB_REDIRECT_URI: &str = "urn:ietf:wg:oauth:2.0:oob";

#[derive(Debug, Deserialize)]
pub struct CreateAppRequest {
    pub client_name: String,
    pub redirect_uris: String,
    pub scopes: Option<String>,
    pub website: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AppResponse {
    pub id: String,
    pub name: String,
    pub website: Option<String>,
    pub redirect_uri: String,
    pub redirect_uris: String,
    pub client_id: String,
    pub client_secret: String,
    pub vapid_key: Option<String>,
    pub scopes: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    pub code: Option<String>,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeRequest {
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub scope: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RevokeTokenRequest {
    pub client_id: String,
    pub client_secret: String,
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub scope: String,
    pub created_at: i64,
    pub expires_in: i64,
}

struct AuthorizeContext {
    app_id: String,
    app_name: String,
    redirect_uri: String,
    requested_scopes: String,
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

fn hash_client_secret(secret: &str) -> String {
    let digest = sha2::Sha256::digest(secret.as_bytes());
    format!(
        "{}{}",
        OAUTH_CLIENT_SECRET_HASH_PREFIX,
        URL_SAFE_NO_PAD.encode(digest)
    )
}

fn verify_client_secret(stored_secret: &str, provided_secret: &str) -> bool {
    if stored_secret.starts_with(OAUTH_CLIENT_SECRET_HASH_PREFIX) {
        let hashed = hash_client_secret(provided_secret);
        stored_secret.as_bytes().ct_eq(hashed.as_bytes()).into()
    } else {
        stored_secret
            .as_bytes()
            .ct_eq(provided_secret.as_bytes())
            .into()
    }
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

fn parse_body<T: DeserializeOwned>(headers: &HeaderMap, body: &[u8]) -> Result<T, AppError> {
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    let parse_json = || {
        serde_json::from_slice(body)
            .map_err(|error| AppError::Validation(format!("invalid JSON body: {error}")))
    };
    let parse_form = || {
        serde_urlencoded::from_bytes(body)
            .map_err(|error| AppError::Validation(format!("invalid form body: {error}")))
    };

    if content_type.starts_with("application/json") {
        return parse_json();
    }
    if content_type.starts_with("application/x-www-form-urlencoded") {
        return parse_form();
    }

    parse_json().or_else(|_| parse_form())
}

fn current_local_session(
    state: &AppsApiState,
    jar: &CookieJar,
    headers: &HeaderMap,
) -> Option<crate::auth::Session> {
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(token) = bearer
        && let Ok(session) = verify_session_token(token, &state.config.auth.session_secret)
    {
        return Some(session);
    }

    if let Some(token) = jar.get("session").map(|cookie| cookie.value())
        && let Ok(session) = verify_session_token(token, &state.config.auth.session_secret)
    {
        return Some(session);
    }

    None
}

async fn validate_authorize_request(
    state: &AppsApiState,
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
            "redirect_uri does not match a registered redirect URI".to_string(),
        ));
    }

    let requested_scopes = req
        .scope
        .as_deref()
        .map(normalize_scopes)
        .filter(|scopes| !scopes.is_empty())
        .unwrap_or_else(|| normalize_scopes(&app.scopes));
    if !scopes_are_subset(&requested_scopes, &app.scopes) {
        return Err(AppError::Unauthorized);
    }

    Ok(AuthorizeContext {
        app_id: app.id,
        app_name: app.name,
        redirect_uri: redirect_uri.to_string(),
        requested_scopes,
    })
}

async fn issue_authorization_code_response(
    state: &AppsApiState,
    context: AuthorizeContext,
    oauth_state: Option<&str>,
) -> Result<Response, AppError> {
    use crate::data::{EntityId, OAuthAuthorizationCode};

    let code_value = EntityId::new_string();
    let authorization_code = OAuthAuthorizationCode {
        id: EntityId::new_string(),
        app_id: context.app_id,
        code: code_value.clone(),
        redirect_uri: context.redirect_uri.clone(),
        scopes: context.requested_scopes.clone(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(10),
    };
    state
        .db
        .insert_oauth_authorization_code(&authorization_code)
        .await?;

    if context.redirect_uri == OOB_REDIRECT_URI {
        let escaped_code = html_escape::encode_text(&code_value);
        let escaped_app_name = html_escape::encode_text(&context.app_name);
        let html = format!(
            r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Authorization Code</title>
</head>
<body>
  <h1>Authorization Complete</h1>
  <p><strong>{}</strong> can now be connected with this authorization code:</p>
  <pre>{}</pre>
</body>
</html>"#,
            escaped_app_name, escaped_code
        );
        return Ok(Html(html).into_response());
    }

    Ok(Redirect::to(&build_authorize_redirect_location(
        &context.redirect_uri,
        &code_value,
        oauth_state,
    ))
    .into_response())
}

/// POST /api/v1/apps
pub async fn create_app(
    State(state): State<AppsApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::data::{EntityId, OAuthApp};

    let req: CreateAppRequest = parse_body(&headers, &body)?;
    if req.client_name.trim().is_empty() {
        return Err(AppError::Validation("client_name is required".to_string()));
    }
    if req.redirect_uris.trim().is_empty() {
        return Err(AppError::Validation(
            "redirect_uris is required".to_string(),
        ));
    }

    let scopes = req
        .scopes
        .as_deref()
        .map(normalize_scopes)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "read".to_string());
    let client_secret = EntityId::new_string();
    let app = OAuthApp {
        id: EntityId::new_string(),
        name: req.client_name.trim().to_string(),
        website: req.website.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }),
        redirect_uri: req.redirect_uris.trim().to_string(),
        client_id: EntityId::new_string(),
        client_secret: hash_client_secret(&client_secret),
        vapid_key: Some(state.web_push_sender.server_key().await?),
        scopes: scopes.clone(),
        created_at: Utc::now(),
    };
    state.db.insert_oauth_app(&app).await?;

    Ok(Json(serde_json::json!(AppResponse {
        id: app.id,
        name: app.name,
        website: app.website,
        redirect_uri: app.redirect_uri.clone(),
        redirect_uris: app.redirect_uri,
        client_id: app.client_id,
        client_secret,
        vapid_key: app.vapid_key,
        scopes,
    })))
}

/// GET /oauth/authorize
pub async fn authorize(
    State(state): State<AppsApiState>,
    jar: CookieJar,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(req): Query<AuthorizeRequest>,
) -> Result<Response, AppError> {
    let Some(_session) = current_local_session(&state, &jar, &headers) else {
        let next = uri
            .path_and_query()
            .map(|value| value.as_str())
            .unwrap_or("/oauth/authorize");
        let login_url = format!("/login?next={}", urlencoding::encode(next));
        return Ok(Redirect::to(&login_url).into_response());
    };

    let context = validate_authorize_request(&state, &req).await?;
    issue_authorization_code_response(&state, context, req.state.as_deref()).await
}

/// GET /api/v1/apps/verify_credentials
pub async fn verify_app_credentials(
    State(state): State<AppsApiState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let token = bearer.ok_or(AppError::Unauthorized)?;
    let oauth_token = state
        .db
        .get_oauth_token(token)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let app = state
        .db
        .get_oauth_app_by_id(&oauth_token.app_id)
        .await?
        .ok_or(AppError::Unauthorized)?;
    Ok(Json(serde_json::json!({
        "id": app.id,
        "name": app.name,
        "website": app.website,
        "redirect_uri": app.redirect_uri,
        "redirect_uris": app.redirect_uri,
        "client_id": app.client_id,
        "vapid_key": app.vapid_key,
        "scopes": app.scopes,
    })))
}

/// POST /oauth/token
pub async fn create_token(
    State(state): State<AppsApiState>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::data::{EntityId, OAuthToken};

    let peer_addr = connect_info.as_ref().map(|ConnectInfo(addr)| *addr);
    crate::auth::check_auth_rate_limit(
        state.auth_rate_limiter.as_ref(),
        peer_addr,
        &headers,
        &state.config.server.trusted_proxy_ips,
        "oauth_token",
    )
    .await?;

    let req: TokenRequest = parse_body(&headers, &body)?;
    if req.grant_type != "client_credentials" && req.grant_type != "authorization_code" {
        return Err(AppError::Validation("invalid grant_type".to_string()));
    }

    let app = state
        .db
        .get_oauth_app_by_client_id(&req.client_id)
        .await?
        .ok_or(AppError::Unauthorized)?;
    if !verify_client_secret(&app.client_secret, &req.client_secret) {
        return Err(AppError::Unauthorized);
    }

    let scopes = match req.grant_type.as_str() {
        "client_credentials" => {
            let requested_scopes = req
                .scope
                .as_deref()
                .map(normalize_scopes)
                .filter(|value| !value.is_empty())
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
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    AppError::Validation(
                        "code is required for authorization_code grant".to_string(),
                    )
                })?;
            let redirect_uri = req
                .redirect_uri
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
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
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| normalize_scopes(&authorization_code.scopes));
            if !scopes_are_subset(&requested_scopes, &authorization_code.scopes)
                || !scopes_are_subset(&requested_scopes, &app.scopes)
            {
                return Err(AppError::Unauthorized);
            }
            requested_scopes
        }
        _ => unreachable!(),
    };

    let issued_at = Utc::now();
    let token = OAuthToken {
        id: EntityId::new_string(),
        app_id: app.id,
        access_token: EntityId::new_string(),
        grant_type: req.grant_type,
        scopes: scopes.clone(),
        created_at: issued_at,
        expires_at: issued_at + chrono::Duration::seconds(OAUTH_ACCESS_TOKEN_TTL_SECONDS),
        revoked: false,
    };
    state.db.insert_oauth_token(&token).await?;

    Ok(Json(serde_json::json!(TokenResponse {
        access_token: token.access_token,
        token_type: "Bearer".to_string(),
        scope: token.scopes,
        created_at: issued_at.timestamp(),
        expires_in: OAUTH_ACCESS_TOKEN_TTL_SECONDS,
    })))
}

/// POST /oauth/revoke
pub async fn revoke_token(
    State(state): State<AppsApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    let req: RevokeTokenRequest = parse_body(&headers, &body)?;
    let app = state
        .db
        .get_oauth_app_by_client_id(&req.client_id)
        .await?
        .ok_or(AppError::Unauthorized)?;
    if !verify_client_secret(&app.client_secret, &req.client_secret) {
        return Err(AppError::Unauthorized);
    }
    state.db.revoke_oauth_token(&req.token).await?;
    Ok(Json(serde_json::json!({})))
}

#[cfg(test)]
mod tests {
    use super::{hash_client_secret, normalize_scopes, verify_client_secret};

    #[test]
    fn normalize_scopes_collapses_whitespace() {
        assert_eq!(
            normalize_scopes(" read:accounts   write:statuses "),
            "read:accounts write:statuses"
        );
    }

    #[test]
    fn verify_client_secret_accepts_hashed_storage() {
        let stored = hash_client_secret("plain-secret");
        assert!(verify_client_secret(&stored, "plain-secret"));
        assert!(!verify_client_secret(&stored, "wrong-secret"));
    }
}
