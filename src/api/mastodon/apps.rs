//! Mastodon app registration and OAuth endpoints.

use axum::{
    body::Bytes,
    extract::{ConnectInfo, OriginalUri, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json, Redirect, Response},
};
use axum_extra::extract::CookieJar;
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD},
};
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

const OAUTH_REFRESH_TOKEN_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;
const OAUTH_CLIENT_SECRET_HASH_PREFIX: &str = "sha256:";
const OOB_REDIRECT_URI: &str = "urn:ietf:wg:oauth:2.0:oob";

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RedirectUrisField {
    String(String),
    Array(Vec<String>),
}

#[derive(Debug, Deserialize)]
pub struct CreateAppRequest {
    pub client_name: String,
    pub redirect_uris: RedirectUrisField,
    pub scopes: Option<String>,
    pub website: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AppResponse {
    pub id: String,
    pub name: String,
    pub website: Option<String>,
    pub redirect_uri: String,
    pub redirect_uris: Vec<String>,
    pub client_id: String,
    pub client_secret: String,
    pub client_secret_expires_at: i64,
    pub vapid_key: Option<String>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    pub code: Option<String>,
    pub code_verifier: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub redirect_uri: Option<String>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeRequest {
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub scope: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RevokeTokenRequest {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub scope: String,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
}

#[derive(Debug, Serialize)]
struct OAuthErrorResponse {
    error: String,
    error_description: String,
}

struct AuthorizeContext {
    app_id: String,
    app_name: String,
    redirect_uri: String,
    requested_scopes: String,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
}

fn normalize_scopes(scopes: &str) -> String {
    scopes.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn oauth_error_response(
    status: StatusCode,
    error: &str,
    description: impl Into<String>,
) -> Response {
    (
        status,
        Json(serde_json::json!(OAuthErrorResponse {
            error: error.to_string(),
            error_description: description.into(),
        })),
    )
        .into_response()
}

fn scopes_to_vec(scopes: &str) -> Vec<String> {
    normalize_scopes(scopes)
        .split_whitespace()
        .map(|scope| scope.to_string())
        .collect()
}

fn redirect_uris_to_vec(redirect_uris: &str) -> Vec<String> {
    redirect_uris
        .split_whitespace()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect()
}

fn normalize_redirect_uris_field(field: RedirectUrisField) -> Result<String, AppError> {
    let values = match field {
        RedirectUrisField::String(value) => redirect_uris_to_vec(&value),
        RedirectUrisField::Array(values) => values
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect(),
    };

    if values.is_empty() {
        return Err(AppError::Validation(
            "redirect_uris must contain at least one URI".to_string(),
        ));
    }

    Ok(values.join("\n"))
}

fn scope_matches(granted: &str, required: &str) -> bool {
    granted == required
        || required
            .strip_prefix(granted)
            .is_some_and(|suffix| suffix.starts_with(':'))
}

fn scopes_are_subset(requested: &str, allowed: &str) -> bool {
    let requested_set: HashSet<&str> = requested.split_whitespace().collect();
    let allowed_set: Vec<&str> = allowed.split_whitespace().collect();
    requested_set.iter().all(|requested_scope| {
        allowed_set
            .iter()
            .any(|allowed_scope| scope_matches(allowed_scope, requested_scope))
    })
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

fn build_authorize_error_redirect_location(
    redirect_uri: &str,
    error: &str,
    description: &str,
    state: Option<&str>,
) -> String {
    if let Ok(mut redirect) = Url::parse(redirect_uri) {
        let mut serializer =
            url::form_urlencoded::Serializer::new(redirect.query().unwrap_or("").to_string());
        serializer.append_pair("error", error);
        serializer.append_pair("error_description", description);
        if let Some(state) = state {
            serializer.append_pair("state", state);
        }
        redirect.set_query(Some(&serializer.finish()));
        return redirect.to_string();
    }

    let separator = if redirect_uri.contains('?') { '&' } else { '?' };
    let mut location = format!(
        "{}{}error={}&error_description={}",
        redirect_uri,
        separator,
        urlencoding::encode(error),
        urlencoding::encode(description)
    );
    if let Some(state) = state {
        location.push_str("&state=");
        location.push_str(&urlencoding::encode(state));
    }
    location
}

fn authorize_error_parts(error: &AppError) -> (StatusCode, &'static str, String) {
    match error {
        AppError::Unauthorized => (
            StatusCode::BAD_REQUEST,
            "invalid_client",
            "client authentication failed".to_string(),
        ),
        AppError::Validation(description) if description.contains("scope") => (
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            description.clone(),
        ),
        AppError::Validation(description) => (
            StatusCode::BAD_REQUEST,
            "invalid_request",
            description.clone(),
        ),
        _ => (
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "authorization request is invalid".to_string(),
        ),
    }
}

async fn authorize_error_response(
    state: &AppsApiState,
    req: &AuthorizeRequest,
    error: AppError,
) -> Result<Response, AppError> {
    let (status, code, description) = authorize_error_parts(&error);
    let redirect_uri = req
        .redirect_uri
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let client_id = req
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let (Some(client_id), Some(redirect_uri)) = (client_id, redirect_uri)
        && let Some(app) = state.db.get_oauth_app_by_client_id(client_id).await?
        && is_registered_redirect_uri(&app.redirect_uri, redirect_uri)
        && redirect_uri != OOB_REDIRECT_URI
    {
        return Ok(Redirect::to(&build_authorize_error_redirect_location(
            redirect_uri,
            code,
            &description,
            req.state.as_deref(),
        ))
        .into_response());
    }

    Ok(oauth_error_response(status, code, description))
}

fn normalize_pkce_method(method: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(method) = method.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    match method {
        "plain" | "S256" => Ok(Some(method.to_string())),
        _ => Err(AppError::Validation(
            "code_challenge_method must be plain or S256".to_string(),
        )),
    }
}

fn verify_pkce_code_verifier(
    code_verifier: &str,
    code_challenge: &str,
    code_challenge_method: Option<&str>,
) -> bool {
    match code_challenge_method.unwrap_or("plain") {
        "plain" => {
            use subtle::ConstantTimeEq;
            code_verifier
                .as_bytes()
                .ct_eq(code_challenge.as_bytes())
                .into()
        }
        "S256" => {
            let digest = sha2::Sha256::digest(code_verifier.as_bytes());
            URL_SAFE_NO_PAD.encode(digest) == code_challenge
        }
        _ => false,
    }
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

fn parse_basic_client_auth(headers: &HeaderMap) -> Result<Option<(String, String)>, AppError> {
    let Some(authorization) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
    else {
        return Ok(None);
    };

    let Some(encoded) = authorization.strip_prefix("Basic ") else {
        return Ok(None);
    };

    let decoded = BASE64_STANDARD
        .decode(encoded.trim())
        .map_err(|_| AppError::Unauthorized)?;
    let decoded = String::from_utf8(decoded).map_err(|_| AppError::Unauthorized)?;
    let (client_id, client_secret) = decoded.split_once(':').ok_or(AppError::Unauthorized)?;

    if client_id.trim().is_empty() || client_secret.trim().is_empty() {
        return Err(AppError::Unauthorized);
    }

    Ok(Some((
        client_id.trim().to_string(),
        client_secret.trim().to_string(),
    )))
}

fn resolve_client_credentials(
    headers: &HeaderMap,
    client_id: Option<String>,
    client_secret: Option<String>,
) -> Result<(String, String), AppError> {
    if let Some((basic_client_id, basic_client_secret)) = parse_basic_client_auth(headers)? {
        return Ok((basic_client_id, basic_client_secret));
    }

    let client_id = client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Validation("client_id is required".to_string()))?;
    let client_secret = client_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Validation("client_secret is required".to_string()))?;

    Ok((client_id.to_string(), client_secret.to_string()))
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
        return Err(AppError::Validation(
            "requested scope exceeds the application's registered scopes".to_string(),
        ));
    }

    let code_challenge = req
        .code_challenge
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let code_challenge_method = normalize_pkce_method(req.code_challenge_method.as_deref())?;
    if code_challenge.is_none() && code_challenge_method.is_some() {
        return Err(AppError::Validation(
            "code_challenge is required when code_challenge_method is provided".to_string(),
        ));
    }

    Ok(AuthorizeContext {
        app_id: app.id,
        app_name: app.name,
        redirect_uri: redirect_uri.to_string(),
        requested_scopes,
        code_challenge,
        code_challenge_method,
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
        code_challenge: context.code_challenge.clone(),
        code_challenge_method: context.code_challenge_method.clone(),
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
    let CreateAppRequest {
        client_name,
        redirect_uris,
        scopes,
        website,
    } = req;
    if client_name.trim().is_empty() {
        return Err(AppError::Validation("client_name is required".to_string()));
    }
    let redirect_uri = normalize_redirect_uris_field(redirect_uris)?;

    let scopes = scopes
        .as_deref()
        .map(normalize_scopes)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "read".to_string());
    let client_secret = EntityId::new_string();
    let app = OAuthApp {
        id: EntityId::new_string(),
        name: client_name.trim().to_string(),
        website: website.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }),
        redirect_uri,
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
        redirect_uris: redirect_uris_to_vec(&app.redirect_uri),
        client_id: app.client_id,
        client_secret,
        client_secret_expires_at: 0,
        vapid_key: app.vapid_key,
        scopes: scopes_to_vec(&scopes),
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

    let context = match validate_authorize_request(&state, &req).await {
        Ok(context) => context,
        Err(error) => return authorize_error_response(&state, &req, error).await,
    };
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
        "redirect_uri": app.redirect_uri.clone(),
        "redirect_uris": redirect_uris_to_vec(&app.redirect_uri),
        "client_id": app.client_id,
        "client_secret_expires_at": 0,
        "vapid_key": app.vapid_key,
        "scopes": scopes_to_vec(&app.scopes),
    })))
}

/// POST /oauth/token
pub async fn create_token(
    State(state): State<AppsApiState>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
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

    let req: TokenRequest = match parse_body(&headers, &body) {
        Ok(req) => req,
        Err(AppError::Validation(description)) => {
            return Ok(oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                description,
            ));
        }
        Err(error) => return Err(error),
    };
    if req.grant_type != "client_credentials"
        && req.grant_type != "authorization_code"
        && req.grant_type != "refresh_token"
    {
        return Ok(oauth_error_response(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "grant_type must be client_credentials, authorization_code, or refresh_token",
        ));
    }

    let (client_id, client_secret) = match resolve_client_credentials(
        &headers,
        req.client_id.clone(),
        req.client_secret.clone(),
    ) {
        Ok(credentials) => credentials,
        Err(AppError::Validation(description)) => {
            return Ok(oauth_error_response(
                StatusCode::UNAUTHORIZED,
                "invalid_client",
                description,
            ));
        }
        Err(AppError::Unauthorized) => {
            return Ok(oauth_error_response(
                StatusCode::UNAUTHORIZED,
                "invalid_client",
                "client authentication failed",
            ));
        }
        Err(error) => return Err(error),
    };

    let Some(app) = state.db.get_oauth_app_by_client_id(&client_id).await? else {
        return Ok(oauth_error_response(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "client authentication failed",
        ));
    };
    if !verify_client_secret(&app.client_secret, &client_secret) {
        return Ok(oauth_error_response(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "client authentication failed",
        ));
    }

    let issued_at = Utc::now();
    let refresh_expires_at = issued_at + chrono::Duration::seconds(OAUTH_REFRESH_TOKEN_TTL_SECONDS);
    let (grant_type, scopes, refresh_token) = match req.grant_type.as_str() {
        "client_credentials" => {
            let requested_scopes = req
                .scope
                .as_deref()
                .map(normalize_scopes)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| normalize_scopes(&app.scopes));
            if !scopes_are_subset(&requested_scopes, &app.scopes) {
                return Ok(oauth_error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_scope",
                    "requested scope exceeds the application's registered scopes",
                ));
            }
            ("client_credentials".to_string(), requested_scopes, None)
        }
        "authorization_code" => {
            let Some(code) = req
                .code
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return Ok(oauth_error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "code is required for authorization_code grant",
                ));
            };
            let Some(redirect_uri) = req
                .redirect_uri
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return Ok(oauth_error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "redirect_uri is required for authorization_code grant",
                ));
            };
            let Some(authorization_code) = state
                .db
                .consume_oauth_authorization_code(code, &app.id, redirect_uri, Utc::now())
                .await?
            else {
                return Ok(oauth_error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "authorization code is invalid, expired, or already used",
                ));
            };
            if let Some(code_challenge) = authorization_code.code_challenge.as_deref() {
                let Some(code_verifier) = req
                    .code_verifier
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return Ok(oauth_error_response(
                        StatusCode::BAD_REQUEST,
                        "invalid_request",
                        "code_verifier is required when PKCE is used",
                    ));
                };
                if !verify_pkce_code_verifier(
                    code_verifier,
                    code_challenge,
                    authorization_code.code_challenge_method.as_deref(),
                ) {
                    return Ok(oauth_error_response(
                        StatusCode::BAD_REQUEST,
                        "invalid_grant",
                        "code_verifier does not match the authorization code challenge",
                    ));
                }
            }
            (
                "authorization_code".to_string(),
                normalize_scopes(&authorization_code.scopes),
                Some(EntityId::new_string()),
            )
        }
        "refresh_token" => {
            let Some(refresh_token) = req
                .refresh_token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return Ok(oauth_error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "refresh_token is required for refresh_token grant",
                ));
            };
            let Some(existing) = state
                .db
                .get_oauth_token_by_refresh_token(refresh_token)
                .await?
            else {
                return Ok(oauth_error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "refresh token is invalid or expired",
                ));
            };
            if existing.app_id != app.id {
                return Ok(oauth_error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "refresh token does not belong to this client",
                ));
            }
            if !matches!(
                existing.grant_type.as_str(),
                "authorization_code" | "refresh_token"
            ) {
                return Ok(oauth_error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "refresh token cannot be used for this grant type",
                ));
            }
            let _ = state.db.revoke_oauth_token(refresh_token).await?;
            (
                "refresh_token".to_string(),
                existing.scopes,
                Some(EntityId::new_string()),
            )
        }
        _ => unreachable!(),
    };

    let token = OAuthToken {
        id: EntityId::new_string(),
        app_id: app.id,
        access_token: EntityId::new_string(),
        refresh_token: refresh_token.clone(),
        grant_type,
        scopes: scopes.clone(),
        created_at: issued_at,
        expires_at: None,
        refresh_expires_at: refresh_token.as_ref().map(|_| refresh_expires_at),
        revoked: false,
    };
    state.db.insert_oauth_token(&token).await?;

    Ok(Json(serde_json::json!(TokenResponse {
        access_token: token.access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        scope: token.scopes,
        created_at: issued_at.timestamp(),
        expires_in: None,
    }))
    .into_response())
}

/// POST /oauth/revoke
pub async fn revoke_token(
    State(state): State<AppsApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let req: RevokeTokenRequest = match parse_body(&headers, &body) {
        Ok(req) => req,
        Err(AppError::Validation(description)) => {
            return Ok(oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                description,
            ));
        }
        Err(error) => return Err(error),
    };
    let (client_id, client_secret) =
        match resolve_client_credentials(&headers, req.client_id, req.client_secret) {
            Ok(credentials) => credentials,
            Err(AppError::Validation(description)) => {
                return Ok(oauth_error_response(
                    StatusCode::UNAUTHORIZED,
                    "invalid_client",
                    description,
                ));
            }
            Err(AppError::Unauthorized) => {
                return Ok(oauth_error_response(
                    StatusCode::UNAUTHORIZED,
                    "invalid_client",
                    "client authentication failed",
                ));
            }
            Err(error) => return Err(error),
        };
    let Some(app) = state.db.get_oauth_app_by_client_id(&client_id).await? else {
        return Ok(oauth_error_response(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "client authentication failed",
        ));
    };
    if !verify_client_secret(&app.client_secret, &client_secret) {
        return Ok(oauth_error_response(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "client authentication failed",
        ));
    }
    if req.token.trim().is_empty() {
        return Ok(oauth_error_response(
            StatusCode::FORBIDDEN,
            "unauthorized_client",
            "You are not authorized to revoke this token",
        ));
    }

    if let Some(owner_app_id) = state.db.lookup_oauth_token_owner(&req.token).await?
        && owner_app_id != app.id
    {
        return Ok(oauth_error_response(
            StatusCode::FORBIDDEN,
            "unauthorized_client",
            "You are not authorized to revoke this token",
        ));
    }

    let _ = state
        .db
        .revoke_oauth_token_for_app(&app.id, &req.token)
        .await?;
    Ok(Json(serde_json::json!({})).into_response())
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
