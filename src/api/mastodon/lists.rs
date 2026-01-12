//! Lists endpoints

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::AppError;

/// List response
#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub id: String,
    pub title: String,
    pub replies_policy: String,
}

/// Create list request
#[derive(Debug, Deserialize)]
pub struct CreateListRequest {
    pub title: String,
    pub replies_policy: Option<String>, // "followed", "list", "none"
}

/// Update list request
#[derive(Debug, Deserialize)]
pub struct UpdateListRequest {
    pub title: Option<String>,
    pub replies_policy: Option<String>,
}

/// Add accounts to list request
#[derive(Debug, Deserialize)]
pub struct AddAccountsRequest {
    #[serde(rename = "account_ids")]
    pub account_ids: Vec<String>,
}

/// Pagination parameters
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    pub max_id: Option<String>,
    pub min_id: Option<String>,
    pub limit: Option<usize>,
}

/// GET /api/v1/lists
/// Get all lists owned by the user
pub async fn get_lists(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<Vec<ListResponse>>, AppError> {
    let lists = state.db.get_all_lists().await?;

    let response: Vec<ListResponse> = lists
        .into_iter()
        .map(|(id, title, replies_policy)| ListResponse {
            id,
            title,
            replies_policy,
        })
        .collect();

    Ok(Json(response))
}

/// GET /api/v1/lists/:id
/// Get a specific list
pub async fn get_list(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<ListResponse>, AppError> {
    let list = state.db.get_list(&id).await?.ok_or(AppError::NotFound)?;

    Ok(Json(ListResponse {
        id: list.0,
        title: list.1,
        replies_policy: list.2,
    }))
}

/// POST /api/v1/lists
/// Create a new list
pub async fn create_list(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Json(req): Json<CreateListRequest>,
) -> Result<Json<ListResponse>, AppError> {
    // Validate title
    if req.title.trim().is_empty() {
        return Err(AppError::Validation("Title cannot be empty".to_string()));
    }

    // Default replies_policy to "list"
    let replies_policy = req.replies_policy.unwrap_or_else(|| "list".to_string());

    // Validate replies_policy
    if !["followed", "list", "none"].contains(&replies_policy.as_str()) {
        return Err(AppError::Validation(
            "Invalid replies_policy. Must be 'followed', 'list', or 'none'".to_string(),
        ));
    }

    let id = state.db.create_list(&req.title, &replies_policy).await?;

    Ok(Json(ListResponse {
        id,
        title: req.title,
        replies_policy,
    }))
}

/// PUT /api/v1/lists/:id
/// Update a list
pub async fn update_list(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateListRequest>,
) -> Result<Json<ListResponse>, AppError> {
    // Get existing list
    let existing = state.db.get_list(&id).await?.ok_or(AppError::NotFound)?;

    // Use existing values if not provided
    let title = req.title.unwrap_or(existing.1.clone());
    let replies_policy = req.replies_policy.unwrap_or(existing.2.clone());

    // Validate title
    if title.trim().is_empty() {
        return Err(AppError::Validation("Title cannot be empty".to_string()));
    }

    // Validate replies_policy
    if !["followed", "list", "none"].contains(&replies_policy.as_str()) {
        return Err(AppError::Validation(
            "Invalid replies_policy. Must be 'followed', 'list', or 'none'".to_string(),
        ));
    }

    state.db.update_list(&id, &title, &replies_policy).await?;

    Ok(Json(ListResponse {
        id,
        title,
        replies_policy,
    }))
}

/// DELETE /api/v1/lists/:id
/// Delete a list
pub async fn delete_list(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = state.db.delete_list(&id).await?;

    if !deleted {
        return Err(AppError::NotFound);
    }

    Ok(Json(serde_json::json!({})))
}

/// GET /api/v1/lists/:id/accounts
/// Get accounts in a list
pub async fn get_list_accounts(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(_params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Verify list exists
    state.db.get_list(&id).await?.ok_or(AppError::NotFound)?;

    // Get account addresses in list
    let addresses = state.db.get_list_accounts(&id).await?;

    // For single-user instance, we can only return minimal account info
    // In a full implementation, we would fetch account details from remote servers
    let accounts: Vec<serde_json::Value> = addresses
        .into_iter()
        .map(|address| {
            serde_json::json!({
                "id": address.clone(),
                "username": address.split('@').next().unwrap_or(&address),
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

    Ok(Json(accounts))
}

/// POST /api/v1/lists/:id/accounts
/// Add accounts to a list
pub async fn add_list_accounts(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<AddAccountsRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify list exists
    state.db.get_list(&id).await?.ok_or(AppError::NotFound)?;

    // Add each account to the list
    for account_id in req.account_ids {
        // For single-user instance, account_id is the account address
        state.db.add_account_to_list(&id, &account_id).await?;
    }

    Ok(Json(serde_json::json!({})))
}

/// DELETE /api/v1/lists/:id/accounts
/// Remove accounts from a list
pub async fn delete_list_accounts(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<AddAccountsRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify list exists
    state.db.get_list(&id).await?.ok_or(AppError::NotFound)?;

    // Remove each account from the list
    for account_id in req.account_ids {
        state.db.remove_account_from_list(&id, &account_id).await?;
    }

    Ok(Json(serde_json::json!({})))
}
