//! Notification endpoints

use axum::{
    extract::{Path, Query, State},
    response::Json,
};

use super::accounts::PaginationParams;
use crate::AppState;
use crate::auth::CurrentUser;
use crate::data::Status;
use crate::error::AppError;

async fn get_notification_status(state: &AppState, status_uri: &str) -> Option<Status> {
    if let Ok(status) = state.db.get_status_by_uri(status_uri).await {
        if status.is_some() {
            return status;
        }
    }

    let cached = state.timeline_cache.get_by_uri(status_uri).await?;
    Some(Status {
        id: cached.id.clone(),
        uri: cached.uri.clone(),
        content: cached.content.clone(),
        content_warning: None,
        visibility: cached.visibility.clone(),
        language: None,
        account_address: cached.account_address.clone(),
        is_local: false,
        in_reply_to_uri: cached.reply_to_uri.clone(),
        boost_of_uri: cached.boost_of_uri.clone(),
        persisted_reason: "cache_only".to_string(),
        created_at: cached.created_at,
        fetched_at: Some(chrono::Utc::now()),
    })
}

/// GET /api/v1/notifications
pub async fn get_notifications(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    use crate::api::dto::NotificationResponse;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Get notifications
    let limit = params.limit.unwrap_or(20).min(40);
    let notifications = state
        .db
        .get_notifications(
            limit,
            params.max_id.as_deref(),
            false, // Get all notifications, not just unread
        )
        .await?;

    // Convert to API responses
    let mut responses = vec![];
    for notification in notifications {
        // Get status if present
        let status = if let Some(status_uri) = &notification.status_uri {
            get_notification_status(&state, status_uri).await
        } else {
            None
        };

        let status_response = if let Some(status) = status {
            Some(crate::api::status_to_response(
                &status,
                &account,
                &state.config,
                None,
                None,
                None,
            ))
        } else {
            None
        };

        let response = NotificationResponse {
            id: notification.id.clone(),
            notification_type: notification.notification_type.clone(),
            created_at: notification.created_at,
            account: crate::api::account_to_response(&account, &state.config),
            status: status_response,
        };

        responses.push(serde_json::to_value(response).unwrap());
    }

    Ok(Json(responses))
}

/// POST /api/v1/notifications/:id/dismiss
pub async fn dismiss_notification(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Mark notification as read
    state.db.mark_notification_read(&id).await?;

    Ok(Json(serde_json::json!({})))
}

/// POST /api/v1/notifications/clear
pub async fn clear_notifications(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Mark all notifications as read
    state.db.mark_all_notifications_read().await?;

    Ok(Json(serde_json::json!({})))
}

/// GET /api/v1/notifications/:id
/// Get a single notification by ID
pub async fn get_notification(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::NotificationResponse;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    let notification = state
        .db
        .get_notification(&id)
        .await?
        .ok_or(AppError::NotFound)?;

    // Get status if present
    let status = if let Some(status_uri) = &notification.status_uri {
        get_notification_status(&state, status_uri).await
    } else {
        None
    };

    let status_response = if let Some(status) = status {
        Some(crate::api::status_to_response(
            &status,
            &account,
            &state.config,
            None,
            None,
            None,
        ))
    } else {
        None
    };

    let response = NotificationResponse {
        id: notification.id.clone(),
        notification_type: notification.notification_type.clone(),
        created_at: notification.created_at,
        account: crate::api::account_to_response(&account, &state.config),
        status: status_response,
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/notifications/unread_count
/// Get the count of unread notifications
pub async fn get_unread_count(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get unread notifications
    let unread_notifications = state
        .db
        .get_notifications(
            1000, // Get all unread notifications
            None, true, // Only unread
        )
        .await?;

    let count = unread_notifications.len();

    Ok(Json(serde_json::json!({
        "count": count
    })))
}
