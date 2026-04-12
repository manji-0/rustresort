use axum::{Json, extract::State, response::IntoResponse};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::PushApiState;
use crate::auth::CurrentUser;
use crate::data::{PushAlerts, PushSubscription};
use crate::error::AppError;

#[derive(Debug, Deserialize)]
pub struct PushSubscriptionKeysRequest {
    pub p256dh: String,
    pub auth: String,
}

#[derive(Debug, Deserialize)]
pub struct PushSubscriptionEnvelopeRequest {
    pub endpoint: String,
    pub keys: PushSubscriptionKeysRequest,
    #[serde(default = "default_push_standard")]
    pub standard: bool,
}

#[derive(Debug, Deserialize)]
pub struct PushSubscriptionDataRequest {
    #[serde(default)]
    pub alerts: PushAlerts,
    #[serde(default = "default_push_policy")]
    pub policy: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePushSubscriptionRequest {
    pub subscription: PushSubscriptionEnvelopeRequest,
    #[serde(default)]
    pub data: Option<PushSubscriptionDataRequest>,
}

#[derive(Debug, Serialize)]
pub struct PushSubscriptionResponse {
    pub id: String,
    pub endpoint: String,
    pub standard: bool,
    pub alerts: PushAlerts,
    pub policy: String,
    pub server_key: String,
}

fn default_push_policy() -> String {
    "all".to_string()
}

const fn default_push_standard() -> bool {
    true
}

fn validate_push_policy(policy: &str) -> Result<(), AppError> {
    if ["all", "followed", "follower", "none"].contains(&policy) {
        Ok(())
    } else {
        Err(AppError::Validation(
            "policy must be one of all, followed, follower, or none".to_string(),
        ))
    }
}

fn validate_push_subscription_request(
    request: &CreatePushSubscriptionRequest,
) -> Result<(), AppError> {
    if !request.subscription.standard {
        return Err(AppError::Validation(
            "legacy non-standard webpush subscriptions are not supported".to_string(),
        ));
    }
    let endpoint = request.subscription.endpoint.trim();
    if endpoint.is_empty() {
        return Err(AppError::Validation(
            "subscription endpoint must not be empty".to_string(),
        ));
    }
    let parsed = url::Url::parse(endpoint).map_err(|_| {
        AppError::Validation("subscription endpoint must be a valid URL".to_string())
    })?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err(AppError::Validation(
            "subscription endpoint must use http or https".to_string(),
        ));
    }

    let p256dh = request.subscription.keys.p256dh.trim();
    let auth = request.subscription.keys.auth.trim();
    if p256dh.is_empty() || auth.is_empty() {
        return Err(AppError::Validation(
            "subscription keys must not be empty".to_string(),
        ));
    }
    URL_SAFE_NO_PAD.decode(p256dh).map_err(|_| {
        AppError::Validation("subscription[keys][p256dh] must be base64url".to_string())
    })?;
    let auth_bytes = URL_SAFE_NO_PAD.decode(auth).map_err(|_| {
        AppError::Validation("subscription[keys][auth] must be base64url".to_string())
    })?;
    if auth_bytes.len() != 16 {
        return Err(AppError::Validation(
            "subscription[keys][auth] must decode to 16 bytes".to_string(),
        ));
    }

    if let Some(data) = &request.data {
        validate_push_policy(data.policy.trim())?;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct UpdatePushSubscriptionRequest {
    pub data: PushSubscriptionDataRequest,
}

fn decode_alerts(subscription: &PushSubscription) -> Result<PushAlerts, AppError> {
    serde_json::from_str(&subscription.alerts_json)
        .map_err(|error| AppError::internal(format!("Invalid stored push alerts JSON: {}", error)))
}

async fn response_from_subscription(
    state: &PushApiState,
    subscription: PushSubscription,
) -> Result<PushSubscriptionResponse, AppError> {
    let alerts = decode_alerts(&subscription)?;
    Ok(PushSubscriptionResponse {
        id: subscription.id,
        endpoint: subscription.endpoint,
        standard: true,
        alerts,
        policy: subscription.policy,
        server_key: state.web_push_sender.server_key().await?,
    })
}

pub async fn get_subscription(
    State(state): State<PushApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<PushSubscriptionResponse>, AppError> {
    let subscription = state
        .db
        .get_push_subscription()
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(
        response_from_subscription(&state, subscription).await?,
    ))
}

pub async fn create_subscription(
    State(state): State<PushApiState>,
    CurrentUser(_session): CurrentUser,
    Json(request): Json<CreatePushSubscriptionRequest>,
) -> Result<Json<PushSubscriptionResponse>, AppError> {
    validate_push_subscription_request(&request)?;
    let data = request.data.unwrap_or(PushSubscriptionDataRequest {
        alerts: PushAlerts::default(),
        policy: default_push_policy(),
    });
    let subscription = state
        .db
        .upsert_push_subscription(
            request.subscription.endpoint.trim(),
            request.subscription.keys.p256dh.trim(),
            request.subscription.keys.auth.trim(),
            &data.alerts,
            data.policy.trim(),
        )
        .await?;
    Ok(Json(
        response_from_subscription(&state, subscription).await?,
    ))
}

pub async fn update_subscription(
    State(state): State<PushApiState>,
    CurrentUser(_session): CurrentUser,
    Json(request): Json<UpdatePushSubscriptionRequest>,
) -> Result<Json<PushSubscriptionResponse>, AppError> {
    validate_push_policy(request.data.policy.trim())?;
    let existing = state
        .db
        .get_push_subscription()
        .await?
        .ok_or(AppError::NotFound)?;
    let subscription = state
        .db
        .upsert_push_subscription(
            existing.endpoint.trim(),
            existing.p256dh.trim(),
            existing.auth.trim(),
            &request.data.alerts,
            request.data.policy.trim(),
        )
        .await?;
    Ok(Json(
        response_from_subscription(&state, subscription).await?,
    ))
}

pub async fn delete_subscription(
    State(state): State<PushApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<impl IntoResponse, AppError> {
    state.db.delete_push_subscription().await?;
    Ok(StatusCode::NO_CONTENT)
}
