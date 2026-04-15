//! Notification endpoints

use axum::{
    extract::{Path, RawQuery, State},
    response::Json,
};
use serde::Deserialize;

use super::accounts::PaginationParams;
use super::accounts::resolve_account_response_for_identity;
use crate::TimelineApiState;
use crate::auth::CurrentUser;
use crate::data::{NotificationType, PersistedReason, Status, StatusVisibility};
use crate::error::AppError;

#[derive(Debug, Deserialize)]
pub struct NotificationQueryParams {
    pub pagination: PaginationParams,
}

fn parse_notification_query(
    raw_query: Option<&str>,
) -> Result<(NotificationQueryParams, Vec<String>, Vec<String>), AppError> {
    let mut pagination = PaginationParams {
        max_id: None,
        since_id: None,
        min_id: None,
        limit: None,
    };

    let mut include = Vec::new();
    let mut exclude = Vec::new();
    let Some(raw_query) = raw_query else {
        return Ok((NotificationQueryParams { pagination }, include, exclude));
    };

    for (key, value) in url::form_urlencoded::parse(raw_query.as_bytes()) {
        match key.as_ref() {
            "max_id" => pagination.max_id = Some(value.into_owned()),
            "since_id" => pagination.since_id = Some(value.into_owned()),
            "min_id" => pagination.min_id = Some(value.into_owned()),
            "limit" => {
                pagination.limit = Some(value.parse::<usize>().map_err(|_| {
                    AppError::Validation("limit must be a positive integer".to_string())
                })?)
            }
            "types[]" => include.push(value.into_owned()),
            "exclude_types[]" => exclude.push(value.into_owned()),
            _ => {}
        }
    }

    Ok((NotificationQueryParams { pagination }, include, exclude))
}

fn parse_notification_type_filter(raw: &str) -> Option<NotificationType> {
    match raw.trim() {
        "mention" => Some(NotificationType::Mention),
        "favourite" => Some(NotificationType::Favourite),
        "reblog" => Some(NotificationType::Reblog),
        "follow" => Some(NotificationType::Follow),
        "follow_request" => Some(NotificationType::FollowRequest),
        "status" => Some(NotificationType::Status),
        "poll" => Some(NotificationType::Poll),
        "update" => Some(NotificationType::Update),
        "admin.sign_up" => Some(NotificationType::AdminSignUp),
        "admin.report" => Some(NotificationType::AdminReport),
        "severed_relationships" => Some(NotificationType::SeveredRelationships),
        "moderation_warning" => Some(NotificationType::ModerationWarning),
        "quote" => Some(NotificationType::Quote),
        "quoted_update" => Some(NotificationType::QuotedUpdate),
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
        quote_of_uri: cached.quote_of_uri.clone(),
        persisted_reason: PersistedReason::CacheOnly,
        created_at: cached.created_at,
        fetched_at: Some(chrono::Utc::now()),
    })
}

async fn build_notification_status_response(
    state: &TimelineApiState,
    status: &Status,
) -> Result<crate::api::dto::StatusResponse, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
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
        .cloned();
    let thread_uri = state.db.resolve_thread_root_uri(status).await?;
    let interactions = crate::api::StatusInteractions::new(
        Some(state.db.is_favourited(&status.id).await?),
        Some(state.db.is_reposted(&status.id).await?),
        Some(state.db.is_thread_muted(&thread_uri).await?),
        Some(state.db.is_bookmarked(&status.id).await?),
        Some(state.db.is_status_pinned(&status.id).await?),
    );

    crate::api::build_status_response_with_account_stats_and_remote_stats(
        state.db.as_ref(),
        status,
        &account,
        &state.config,
        account_stats,
        remote_stats,
        interactions,
    )
    .await
}

/// GET /api/v1/notifications
pub async fn get_notifications(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    raw_query: RawQuery,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    use crate::api::dto::NotificationResponse;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    // Get notifications
    let (params, raw_include_types, raw_exclude_types) =
        parse_notification_query(raw_query.0.as_deref())?;
    let limit = params.pagination.limit.unwrap_or(20).min(40);
    let include_types = raw_include_types
        .iter()
        .filter_map(|value| parse_notification_type_filter(value))
        .collect::<Vec<_>>();
    let exclude_types = raw_exclude_types
        .iter()
        .filter_map(|value| parse_notification_type_filter(value))
        .collect::<Vec<_>>();
    let fetch_limit = limit.max(40);
    let mut notifications = Vec::new();
    let mut cursor = params.pagination.max_id.clone();

    while notifications.len() < limit {
        let batch = state
            .db
            .get_notifications(
                fetch_limit,
                cursor.as_deref(),
                false, // Get all notifications, not just unread
            )
            .await?;
        if batch.is_empty() {
            break;
        }

        let reached_end = batch.len() < fetch_limit;
        cursor = batch.last().map(|notification| notification.id.clone());

        for notification in batch {
            if notification_is_included(
                notification.notification_type,
                &include_types,
                &exclude_types,
            ) {
                notifications.push(notification);
                if notifications.len() == limit {
                    break;
                }
            }
        }

        if reached_end {
            break;
        }
    }

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
            Some(build_notification_status_response(&state, &status).await?)
        } else {
            None
        };

        let response = NotificationResponse {
            id: notification.id.clone(),
            notification_type: notification.notification_type.to_string(),
            group_key: format!("ungrouped-{}", notification.id),
            created_at: notification.created_at,
            account: resolve_account_response_for_identity(
                state.config.as_ref(),
                state.db.as_ref(),
                state.profile_cache.as_ref(),
                None,
                &notification.origin_account_address,
            )
            .await
            .unwrap_or_else(|| {
                crate::api::account_to_response_with_stats(&account, &state.config, account_stats)
            }),
            status: status_response,
            report: None,
            event: None,
            moderation_warning: None,
        };

        responses.push(serde_json::to_value(response).unwrap());
    }

    Ok(Json(responses))
}

/// GET /api/v2/notifications
pub async fn get_notifications_v2(
    state: State<TimelineApiState>,
    session: CurrentUser,
    raw_query: RawQuery,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    get_notifications(state, session, raw_query).await
}

/// POST /api/v1/notifications/:id/dismiss
pub async fn dismiss_notification(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !state.db.delete_notification(&id).await? {
        return Err(AppError::NotFound);
    }

    Ok(Json(serde_json::json!({})))
}

/// POST /api/v1/notifications/clear
pub async fn clear_notifications(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    state.db.clear_notifications().await?;

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
        Some(build_notification_status_response(&state, &status).await?)
    } else {
        None
    };

    let response = NotificationResponse {
        id: notification.id.clone(),
        notification_type: notification.notification_type.to_string(),
        group_key: format!("ungrouped-{}", notification.id),
        created_at: notification.created_at,
        account: resolve_account_response_for_identity(
            state.config.as_ref(),
            state.db.as_ref(),
            state.profile_cache.as_ref(),
            None,
            &notification.origin_account_address,
        )
        .await
        .unwrap_or_else(|| {
            crate::api::account_to_response_with_stats(&account, &state.config, account_stats)
        }),
        status: status_response,
        report: None,
        event: None,
        moderation_warning: None,
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/notifications/unread_count
/// Get the count of unread notifications
pub async fn get_unread_count(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let count = state.db.count_unread_notifications().await?;

    Ok(Json(serde_json::json!({
        "count": count
    })))
}
