//! Conversations endpoints (Direct Messages)

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::Deserialize;

use crate::{AppState, auth::CurrentUser, error::AppError};

#[derive(Debug, Deserialize)]
pub struct ConversationsParams {
    /// Maximum number of results to return (default 20)
    limit: Option<usize>,
    /// Return results older than this ID
    max_id: Option<String>,
    /// Return results newer than this ID
    since_id: Option<String>,
    /// Return results immediately newer than this ID
    min_id: Option<String>,
}

/// GET /api/v1/conversations - Get conversations
///
/// View all conversations (direct message threads).
pub async fn get_conversations(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<ConversationsParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let limit = params.limit.unwrap_or(20).min(40);

    let conversations = state.db.get_conversations(limit).await?;

    let mut response = Vec::new();
    for (conversation_id, last_status_id, unread) in conversations {
        // Get participants
        let participant_addresses = state
            .db
            .get_conversation_participants(&conversation_id)
            .await?;

        // Create minimal account info for participants
        let accounts: Vec<serde_json::Value> = participant_addresses
            .iter()
            .map(|address| {
                serde_json::json!({
                    "id": address.clone(),
                    "username": address.split('@').next().unwrap_or(address),
                    "acct": address,
                    "display_name": "",
                    "note": "",
                    "url": format!("https://{}", address.split('@').nth(1).unwrap_or("")),
                    "avatar": "",
                    "header": "",
                    "followers_count": 0,
                    "following_count": 0,
                    "statuses_count": 0,
                    "created_at": chrono::Utc::now().to_rfc3339(),
                })
            })
            .collect();

        // Get last status if available
        let last_status = if let Some(status_id) = last_status_id {
            state.db.get_status(&status_id).await?.map(|status| {
                // Create minimal status response
                serde_json::json!({
                    "id": status.id,
                    "content": status.content,
                    "created_at": status.created_at,
                    "visibility": status.visibility,
                })
            })
        } else {
            None
        };

        response.push(serde_json::json!({
            "id": conversation_id,
            "unread": unread,
            "accounts": accounts,
            "last_status": last_status,
        }));
    }

    Ok(Json(serde_json::json!(response)))
}

/// DELETE /api/v1/conversations/:id - Remove a conversation
///
/// Remove a conversation from the list.
pub async fn delete_conversation(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = state.db.delete_conversation(&id).await?;

    if !deleted {
        return Err(AppError::NotFound);
    }

    Ok(Json(serde_json::json!({})))
}

/// POST /api/v1/conversations/:id/read - Mark as read
///
/// Mark a conversation as read.
pub async fn mark_conversation_read(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let updated = state.db.mark_conversation_read(&id).await?;

    if !updated {
        return Err(AppError::NotFound);
    }

    // Return the updated conversation
    // For simplicity, just return success
    Ok(Json(serde_json::json!({
        "id": id,
        "unread": false,
    })))
}

// Helper function to create conversation response (for future use)
#[allow(dead_code)]
fn conversation_to_response(
    id: &str,
    unread: bool,
    accounts: Vec<serde_json::Value>,
    last_status: Option<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "unread": unread,
        "accounts": accounts,
        "last_status": last_status
    })
}
