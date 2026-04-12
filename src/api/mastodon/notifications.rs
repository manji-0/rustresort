//! Notification endpoints

use axum::{
    extract::{Path, Query, RawQuery, State},
    response::Json,
};
use serde::Deserialize;

use super::accounts::PaginationParams;
use crate::TimelineApiState;
use crate::auth::CurrentUser;
use crate::data::{NotificationType, PersistedReason, Status, StatusVisibility};
use crate::error::AppError;

#[derive(Debug, Deserialize)]
pub struct NotificationQueryParams {
    #[serde(flatten)]
    pub pagination: PaginationParams,
}

fn parse_notification_type_query(raw_query: Option<&str>) -> (Vec<String>, Vec<String>) {
    let Some(raw_query) = raw_query else {
        return (Vec::new(), Vec::new());
    };

    let mut include = Vec::new();
    let mut exclude = Vec::new();
    for (key, value) in url::form_urlencoded::parse(raw_query.as_bytes()) {
        match key.as_ref() {
            "types[]" => include.push(value.into_owned()),
            "exclude_types[]" => exclude.push(value.into_owned()),
            _ => {}
        }
    }

    (include, exclude)
}

fn parse_notification_type_filter(raw: &str) -> Option<NotificationType> {
    match raw.trim() {
        "mention" => Some(NotificationType::Mention),
        "favourite" => Some(NotificationType::Favourite),
        "reblog" => Some(NotificationType::Reblog),
        "follow" => Some(NotificationType::Follow),
        "follow_request" => Some(NotificationType::FollowRequest),
        _ => None,
    }
}

fn notification_is_included(
    notification_type: NotificationType,
    include_types: &[NotificationType],
    exclude_types: &[NotificationType],
) -> bool {
    if !include_types.is_empty() && !include_types.iter().any(|ty| *ty == notification_type) {
        return false;
    }

    !exclude_types.iter().any(|ty| *ty == notification_type)
}

async fn get_notification_status(state: &TimelineApiState, status_uri: &str) -> Option<Status> {
    if let Ok(status) = state.db.get_status_by_uri(status_uri).await
        && status.is_some()
    {
        return status;
    }

    let cached = state.timeline_cache.get_by_uri(status_uri).await?;
    Some(Status {
        id: cached.id.clone(),
        uri: cached.uri.clone(),
        content: cached.content.clone(),
        content_warning: None,
        visibility: StatusVisibility::parse(&cached.visibility)
            .unwrap_or(StatusVisibility::Private),
        language: None,
        account_address: cached.account_address.clone(),
        is_local: false,
        in_reply_to_uri: cached.reply_to_uri.clone(),
        boost_of_uri: cached.boost_of_uri.clone(),
        persisted_reason: PersistedReason::CacheOnly,
        created_at: cached.created_at,
        fetched_at: Some(chrono::Utc::now()),
    })
}

/// GET /api/v1/notifications
pub async fn get_notifications(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    raw_query: RawQuery,
    Query(params): Query<NotificationQueryParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    use crate::api::dto::NotificationResponse;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    // Get notifications
    let limit = params.pagination.limit.unwrap_or(20).min(40);
    let (raw_include_types, raw_exclude_types) =
        parse_notification_type_query(raw_query.0.as_deref());
    let include_types = raw_include_types
        .iter()
        .filter_map(|value| parse_notification_type_filter(value))
        .collect::<Vec<_>>();
    let exclude_types = raw_exclude_types
        .iter()
        .filter_map(|value| parse_notification_type_filter(value))
        .collect::<Vec<_>>();
    let notifications = state
        .db
        .get_notifications(
            limit,
            params.pagination.max_id.as_deref(),
            false, // Get all notifications, not just unread
        )
        .await?;

    // Convert to API responses
    let mut responses = vec![];
    for notification in notifications {
        if !notification_is_included(
            notification.notification_type,
            &include_types,
            &exclude_types,
        ) {
            continue;
        }

        // Get status if present
        let status = if let Some(status_uri) = &notification.status_uri {
            get_notification_status(&state, status_uri).await
        } else {
            None
        };

        let status_response = if let Some(status) = status {
            let status_batch = vec![status.clone()];
            let remote_account_stats = crate::api::load_remote_account_stats_map(
                state.db.as_ref(),
                state.profile_cache.as_ref(),
                &state.config.server.protocol,
                &status_batch,
            )
            .await
            .unwrap_or_default();
            let remote_stats = remote_account_stats
                .get(status.account_address.trim())
                .copied();
            Some(
                crate::api::status_to_response_with_account_stats_and_remote_stats(
                    &status,
                    &account,
                    &state.config,
                    account_stats,
                    remote_stats,
                    crate::api::StatusInteractions::default(),
                ),
            )
        } else {
            None
        };

        let response = NotificationResponse {
            id: notification.id.clone(),
            notification_type: notification.notification_type.to_string(),
            created_at: notification.created_at,
            account: crate::api::account_to_response_with_stats(
                &account,
                &state.config,
                account_stats,
            ),
            status: status_response,
        };

        responses.push(serde_json::to_value(response).unwrap());
    }

    Ok(Json(responses))
}

/// POST /api/v1/notifications/:id/dismiss
pub async fn dismiss_notification(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Mark notification as read
    state.db.mark_notification_read(&id).await?;

    Ok(Json(serde_json::json!({})))
}

/// POST /api/v1/notifications/clear
pub async fn clear_notifications(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Mark all notifications as read
    state.db.mark_all_notifications_read().await?;

    Ok(Json(serde_json::json!({})))
}

/// GET /api/v1/notifications/:id
/// Get a single notification by ID
pub async fn get_notification(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::NotificationResponse;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

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
        let status_batch = vec![status.clone()];
        let remote_account_stats = crate::api::load_remote_account_stats_map(
            state.db.as_ref(),
            state.profile_cache.as_ref(),
            &state.config.server.protocol,
            &status_batch,
        )
        .await
        .unwrap_or_default();
        let remote_stats = remote_account_stats
            .get(status.account_address.trim())
            .copied();
        Some(
            crate::api::status_to_response_with_account_stats_and_remote_stats(
                &status,
                &account,
                &state.config,
                account_stats,
                remote_stats,
                crate::api::StatusInteractions::default(),
            ),
        )
    } else {
        None
    };

    let response = NotificationResponse {
        id: notification.id.clone(),
        notification_type: notification.notification_type.to_string(),
        created_at: notification.created_at,
        account: crate::api::account_to_response_with_stats(&account, &state.config, account_stats),
        status: status_response,
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/notifications/unread_count
/// Get the count of unread notifications
pub async fn get_unread_count(
    State(state): State<TimelineApiState>,
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
