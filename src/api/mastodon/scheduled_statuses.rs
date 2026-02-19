//! Scheduled statuses endpoints

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::Deserialize;

use crate::{AppState, auth::CurrentUser, error::AppError};

#[derive(Debug, Deserialize)]
pub struct ScheduledStatusesParams {
    /// Maximum number of results to return (default 20)
    limit: Option<usize>,
    /// Return results older than this ID
    max_id: Option<String>,
    /// Return results newer than this ID
    since_id: Option<String>,
    /// Return results immediately newer than this ID
    min_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateScheduledStatusParams {
    /// ISO 8601 DateTime to schedule the status
    scheduled_at: String,
}

/// GET /api/v1/scheduled_statuses - Get scheduled statuses
///
/// View scheduled statuses.
pub async fn get_scheduled_statuses(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<ScheduledStatusesParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let limit = params.limit.unwrap_or(20).min(40);

    let scheduled_statuses = state.db.get_all_scheduled_statuses(limit).await?;

    Ok(Json(serde_json::json!(scheduled_statuses)))
}

/// GET /api/v1/scheduled_statuses/:id - Get a scheduled status
///
/// View a single scheduled status.
pub async fn get_scheduled_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let scheduled_status = state
        .db
        .get_scheduled_status(&id)
        .await?
        .ok_or(AppError::NotFound)?;

    Ok(Json(scheduled_status))
}

/// PUT /api/v1/scheduled_statuses/:id - Update a scheduled status
///
/// Update the scheduled time of a scheduled status.
pub async fn update_scheduled_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(params): Json<UpdateScheduledStatusParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate scheduled_at format and ensure it is in the future.
    let scheduled_at = chrono::DateTime::parse_from_rfc3339(params.scheduled_at.trim())
        .map_err(|_| AppError::Validation("Invalid scheduled_at format".to_string()))?
        .with_timezone(&chrono::Utc);
    if scheduled_at <= chrono::Utc::now() {
        return Err(AppError::Unprocessable(
            "scheduled_at must be in the future".to_string(),
        ));
    }

    // Update the scheduled time
    let updated = state
        .db
        .update_scheduled_status(&id, &scheduled_at.to_rfc3339())
        .await?;

    if !updated {
        return Err(AppError::NotFound);
    }

    // Return the updated scheduled status
    let scheduled_status = state
        .db
        .get_scheduled_status(&id)
        .await?
        .ok_or(AppError::NotFound)?;

    Ok(Json(scheduled_status))
}

/// DELETE /api/v1/scheduled_statuses/:id - Cancel a scheduled status
///
/// Cancel a scheduled status.
pub async fn delete_scheduled_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = state.db.delete_scheduled_status(&id).await?;

    if !deleted {
        return Err(AppError::NotFound);
    }

    Ok(Json(serde_json::json!({})))
}

// Helper function to create scheduled status response (for future use)
#[allow(dead_code)]
fn scheduled_status_to_response(
    id: &str,
    scheduled_at: &str,
    params: serde_json::Value,
    media_attachments: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "scheduled_at": scheduled_at,
        "params": params,
        "media_attachments": media_attachments
    })
}
