//! Apps and OAuth endpoints

use axum::{
    extract::{Query, State},
    response::{Json, Redirect},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use url::Url;

use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::AppError;

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
}

/// OAuth token response
#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub scope: String,
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
    Query(req): Query<AuthorizeRequest>,
) -> Result<Redirect, AppError> {
    use crate::data::{EntityId, OAuthAuthorizationCode};

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
        .ok_or_else(|| AppError::Validation("client_id is required".to_string()))?;
    let redirect_uri = req
        .redirect_uri
        .as_deref()
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

    let code_value = EntityId::new().0;
    let authorization_code = OAuthAuthorizationCode {
        id: EntityId::new().0,
        app_id: app.id,
        code: code_value.clone(),
        redirect_uri: redirect_uri.to_string(),
        scopes: requested_scopes,
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(10),
    };

    state
        .db
        .insert_oauth_authorization_code(&authorization_code)
        .await?;

    let location =
        build_authorize_redirect_location(redirect_uri, &code_value, req.state.as_deref());
    Ok(Redirect::to(&location))
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
    let token = OAuthToken {
        id: token_id.clone(),
        app_id: app.id.clone(),
        access_token: access_token.clone(),
        scopes,
        created_at: Utc::now(),
        revoked: false,
    };

    // Save to database
    state.db.insert_oauth_token(&token).await?;

    // Return response
    let response = TokenResponse {
        access_token: token.access_token,
        token_type: "Bearer".to_string(),
        scope: token.scopes,
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
