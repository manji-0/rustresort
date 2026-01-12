//! Apps and OAuth endpoints

use axum::{extract::State, response::Json};
use chrono::Utc;
use serde::{Deserialize, Serialize};

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

/// OAuth token response
#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub scope: String,
    pub created_at: i64,
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

    // For single-user instance, we'll use a simplified OAuth flow
    // Generate access token
    let token_id = EntityId::new().0;
    let access_token = EntityId::new().0;

    // Default scopes
    let scopes = req.scope.unwrap_or_else(|| "read write follow".to_string());

    // Create token
    let token = OAuthToken {
        id: token_id.clone(),
        app_id: app.id.clone(),
        access_token: access_token.clone(),
        scopes: scopes.clone(),
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
