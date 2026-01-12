//! Status endpoints

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::{Deserialize, Serialize};

use super::accounts::PaginationParams;
use crate::api::metrics::{DB_QUERIES_TOTAL, DB_QUERY_DURATION_SECONDS, HTTP_REQUESTS_TOTAL, HTTP_REQUEST_DURATION_SECONDS, POSTS_TOTAL};
use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::AppError;

/// Status creation request
#[derive(Debug, Deserialize)]
pub struct CreateStatusRequest {
    pub status: Option<String>,
    pub media_ids: Option<Vec<String>>,
    pub in_reply_to_id: Option<String>,
    pub sensitive: Option<bool>,
    pub spoiler_text: Option<String>,
    pub visibility: Option<String>,
    pub language: Option<String>,
}

/// Status source response
#[derive(Debug, Serialize)]
struct StatusSourceResponse {
    id: String,
    text: String,
    spoiler_text: String,
}

/// POST /api/v1/statuses
pub async fn create_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Json(req): Json<CreateStatusRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::data::{EntityId, Status};
    use chrono::Utc;

    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["POST", "/api/v1/statuses"])
        .start_timer();

    // Get account
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "accounts"])
        .start_timer();
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    DB_QUERIES_TOTAL.with_label_values(&["SELECT", "accounts"]).inc();
    db_timer.observe_duration();

    // Validate
    let content = req
        .status
        .ok_or(AppError::Validation("status is required".to_string()))?;
    if content.is_empty() {
        return Err(AppError::Validation("status cannot be empty".to_string()));
    }

    // Create status
    let status_id = EntityId::new().0;
    let uri = format!(
        "{}/users/{}/statuses/{}",
        state.config.server.base_url(),
        account.username,
        status_id
    );

    let status = Status {
        id: status_id.clone(),
        uri: uri.clone(),
        content: format!("<p>{}</p>", html_escape::encode_text(&content)),
        content_warning: req.spoiler_text,
        visibility: req.visibility.unwrap_or_else(|| "public".to_string()),
        language: req.language.or(Some("en".to_string())),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };

    // Save to database
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["INSERT", "statuses"])
        .start_timer();
    state.db.insert_status(&status).await?;
    DB_QUERIES_TOTAL.with_label_values(&["INSERT", "statuses"]).inc();
    db_timer.observe_duration();

    // Update posts total metric
    POSTS_TOTAL.inc();

    // Attach media if provided
    if let Some(media_ids) = req.media_ids {
        for media_id in media_ids {
            // Verify media exists and attach it to the status
            let db_timer = DB_QUERY_DURATION_SECONDS
                .with_label_values(&["SELECT", "media"])
                .start_timer();
            let media_exists = state.db.get_media(&media_id).await?.is_some();
            DB_QUERIES_TOTAL.with_label_values(&["SELECT", "media"]).inc();
            db_timer.observe_duration();

            if media_exists {
                let db_timer = DB_QUERY_DURATION_SECONDS
                    .with_label_values(&["INSERT", "media_attachments"])
                    .start_timer();
                state
                    .db
                    .attach_media_to_status(&media_id, &status_id)
                    .await?;
                DB_QUERIES_TOTAL.with_label_values(&["INSERT", "media_attachments"]).inc();
                db_timer.observe_duration();
            }
        }
    }

    // Convert to API response
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        Some(false),
        Some(false),
        Some(false),
    );

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["POST", "/api/v1/statuses", "200"])
        .inc();

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/statuses/:id
pub async fn get_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["GET", "/api/v1/statuses/:id"])
        .start_timer();

    // Get status from database
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "statuses"])
        .start_timer();
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;
    DB_QUERIES_TOTAL.with_label_values(&["SELECT", "statuses"]).inc();
    db_timer.observe_duration();

    // Get account
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "accounts"])
        .start_timer();
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    DB_QUERIES_TOTAL.with_label_values(&["SELECT", "accounts"]).inc();
    db_timer.observe_duration();

    // Check if favourited/reblogged/bookmarked
    let favourited = state.db.is_favourited(&id).await.ok();
    let bookmarked = state.db.is_bookmarked(&id).await.ok();

    // Convert to API response
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        favourited,
        Some(false), // reblogged - TODO: implement
        bookmarked,
    );

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/v1/statuses/:id", "200"])
        .inc();

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// DELETE /api/v1/statuses/:id
pub async fn delete_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["DELETE", "/api/v1/statuses/:id"])
        .start_timer();

    // Get status to verify it exists and is local
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "statuses"])
        .start_timer();
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;
    DB_QUERIES_TOTAL.with_label_values(&["SELECT", "statuses"]).inc();
    db_timer.observe_duration();

    // Only allow deleting local statuses
    if !status.is_local {
        return Err(AppError::Forbidden);
    }

    // Get account for response
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "accounts"])
        .start_timer();
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    DB_QUERIES_TOTAL.with_label_values(&["SELECT", "accounts"]).inc();
    db_timer.observe_duration();

    // Delete the status
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["DELETE", "statuses"])
        .start_timer();
    state.db.delete_status(&id).await?;
    DB_QUERIES_TOTAL.with_label_values(&["DELETE", "statuses"]).inc();
    db_timer.observe_duration();

    // Update posts total metric
    POSTS_TOTAL.dec();

    // Return the deleted status
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        Some(false),
        Some(false),
        Some(false),
    );

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["DELETE", "/api/v1/statuses/:id", "200"])
        .inc();

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/statuses/:id/context
pub async fn get_status_context(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::ContextResponse;

    // Get the status to verify it exists
    let _status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Get account
    let _account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // TODO: Implement proper reply tree traversal
    // For now, return empty ancestors and descendants
    // In a full implementation, we would:
    // 1. Traverse up the reply chain to get ancestors
    // 2. Query for statuses that reply to this one for descendants

    let context = ContextResponse {
        ancestors: vec![],
        descendants: vec![],
    };

    Ok(Json(serde_json::to_value(context).unwrap()))
}

/// GET /api/v1/statuses/:id/reblogged_by
pub async fn get_reblogged_by(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(_params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get the status to verify it exists
    let _status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // For single-user instance, only the owner can reblog
    // In a full implementation, we would query the reposts table
    // and fetch account information for each user who reblogged

    // For now, return empty array as we don't track individual rebloggers
    Ok(Json(vec![]))
}

/// GET /api/v1/statuses/:id/favourited_by
pub async fn get_favourited_by(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(_params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get the status to verify it exists
    let _status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // For single-user instance, only the owner can favourite
    // Check if the status is favourited by the owner
    let is_favourited = state.db.is_favourited(&id).await?;

    if is_favourited {
        // Return the owner's account
        let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

        let account_response = crate::api::account_to_response(&account, &state.config);
        Ok(Json(vec![serde_json::to_value(account_response).unwrap()]))
    } else {
        // Not favourited, return empty array
        Ok(Json(vec![]))
    }
}

/// GET /api/v1/statuses/:id/source
pub async fn get_status_source(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get the status
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Only allow getting source for local statuses
    if !status.is_local {
        return Err(AppError::Forbidden);
    }

    // Return the source
    let source = StatusSourceResponse {
        id: status.id.clone(),
        text: status.content.clone(),
        spoiler_text: status.content_warning.unwrap_or_default(),
    };

    Ok(Json(serde_json::to_value(source).unwrap()))
}

/// POST /api/v1/statuses/:id/favourite
pub async fn favourite_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get status
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Add favourite
    state.db.insert_favourite(&id).await?;

    // Return status with favourited=true
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        Some(true),
        Some(false),
        state.db.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unfavourite
pub async fn unfavourite_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get status
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Remove favourite
    state.db.delete_favourite(&id).await?;

    // Return status with favourited=false
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        Some(false),
        Some(false),
        state.db.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/reblog
pub async fn reblog_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::data::EntityId;

    // Get original status
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Create repost record
    let repost_id = EntityId::new().0;
    let repost_uri = format!(
        "{}/users/{}/statuses/{}/activity",
        state.config.server.base_url(),
        account.username,
        repost_id
    );

    state.db.insert_repost(&id, &repost_uri).await?;

    // Return the original status with reblogged=true
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        state.db.is_favourited(&id).await.ok(),
        Some(true),
        state.db.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unreblog
pub async fn unreblog_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get status
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Remove repost
    state.db.delete_repost(&id).await?;

    // Return status with reblogged=false
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        state.db.is_favourited(&id).await.ok(),
        Some(false),
        state.db.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/bookmark
pub async fn bookmark_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get status
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Add bookmark
    state.db.insert_bookmark(&id).await?;

    // Return status with bookmarked=true
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        state.db.is_favourited(&id).await.ok(),
        Some(false),
        Some(true),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unbookmark
pub async fn unbookmark_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get status
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Remove bookmark
    state.db.delete_bookmark(&id).await?;

    // Return status with bookmarked=false
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        state.db.is_favourited(&id).await.ok(),
        Some(false),
        Some(false),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// Update status request
#[derive(Debug, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: Option<String>,
    pub spoiler_text: Option<String>,
    pub sensitive: Option<bool>,
    pub media_ids: Option<Vec<String>>,
}

/// PUT /api/v1/statuses/:id
/// Edit an existing status
///
/// Note: For simplicity in single-user instance, this creates a new version
/// without preserving full edit history.
pub async fn update_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateStatusRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get the status
    let mut status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Only allow editing local statuses
    if !status.is_local {
        return Err(AppError::Forbidden);
    }

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Update fields if provided
    if let Some(content) = req.status {
        if !content.is_empty() {
            status.content = format!("<p>{}</p>", html_escape::encode_text(&content));
        }
    }

    if let Some(spoiler_text) = req.spoiler_text {
        status.content_warning = Some(spoiler_text);
    }

    // TODO: Handle media_ids updates
    // For now, we skip media updates as it requires more complex logic

    // Save updated status
    // Note: In a full implementation, we would create a new version in an edit_history table
    state.db.insert_status(&status).await?;

    // Return updated status
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        state.db.is_favourited(&id).await.ok(),
        Some(false),
        state.db.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/statuses/:id/history
/// Get edit history for a status
///
/// For single-user instance without full edit history tracking,
/// this returns only the current version.
pub async fn get_status_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get the status
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // For now, return only the current version
    // In a full implementation, we would query an edit_history table
    let current_version = serde_json::json!({
        "content": status.content,
        "spoiler_text": status.content_warning.unwrap_or_default(),
        "sensitive": false,
        "created_at": status.created_at.to_rfc3339(),
        "account": crate::api::account_to_response(&account, &state.config),
    });

    Ok(Json(vec![current_version]))
}

/// POST /api/v1/statuses/:id/pin
/// Pin a status to profile
///
/// For single-user instance, this is a no-op that returns success.
/// Pinned statuses are not currently tracked in the database.
pub async fn pin_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get status
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Only allow pinning local statuses
    if !status.is_local {
        return Err(AppError::Validation(
            "Can only pin own statuses".to_string(),
        ));
    }

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // TODO: Store pinned status in database
    // For now, just return the status with pinned=true
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        state.db.is_favourited(&id).await.ok(),
        Some(false),
        state.db.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unpin
/// Unpin a status from profile
///
/// For single-user instance, this is a no-op that returns success.
pub async fn unpin_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get status
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // TODO: Remove pinned status from database
    // For now, just return the status with pinned=false
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        state.db.is_favourited(&id).await.ok(),
        Some(false),
        state.db.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/mute
/// Mute notifications from a conversation
///
/// For single-user instance, this is a no-op that returns success.
/// Conversation muting is not currently tracked.
pub async fn mute_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get status
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // TODO: Store muted conversation in database
    // For now, just return the status with muted=true
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        state.db.is_favourited(&id).await.ok(),
        Some(false),
        state.db.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unmute
/// Unmute notifications from a conversation
///
/// For single-user instance, this is a no-op that returns success.
pub async fn unmute_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get status
    let status = state.db.get_status(&id).await?.ok_or(AppError::NotFound)?;

    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // TODO: Remove muted conversation from database
    // For now, just return the status with muted=false
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        state.db.is_favourited(&id).await.ok(),
        Some(false),
        state.db.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}
