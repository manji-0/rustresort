//! Lists endpoints

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};

use crate::ListsApiState;
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
    #[serde(rename = "max_id")]
    pub _max_id: Option<String>,
    #[serde(rename = "min_id")]
    pub _min_id: Option<String>,
    #[serde(rename = "limit")]
    pub _limit: Option<usize>,
}

/// GET /api/v1/lists
/// Get all lists owned by the user
pub async fn get_lists(
    State(state): State<ListsApiState>,
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
    State(state): State<ListsApiState>,
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
    State(state): State<ListsApiState>,
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
    State(state): State<ListsApiState>,
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
    State(state): State<ListsApiState>,
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
    State(state): State<ListsApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Verify list exists
    state.db.get_list(&id).await?.ok_or(AppError::NotFound)?;

    // Get account addresses in list
    let addresses = state.db.get_list_accounts(&id).await?;
    let limit = params._limit.unwrap_or(40).min(80);
    let default_port = match state.config.server.protocol.to_ascii_lowercase().as_str() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };

    let accounts = stream::iter(addresses.into_iter().take(limit))
        .map(|address| {
            let state = state.clone();
            async move {
                super::accounts::resolve_remote_account_value_for_list(
                    state.config.as_ref(),
                    state.db.as_ref(),
                    state.profile_cache.as_ref(),
                    state.federation_fetch_client.as_ref(),
                    &address,
                    default_port,
                )
                .await
            }
        })
        .buffered(8)
        .collect::<Vec<_>>()
        .await;

    Ok(Json(accounts))
}

/// POST /api/v1/lists/:id/accounts
/// Add accounts to a list
pub async fn add_list_accounts(
    State(state): State<ListsApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<AddAccountsRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify list exists
    state.db.get_list(&id).await?.ok_or(AppError::NotFound)?;

    // For single-user instance, account_id is the account address
    state.db.add_accounts_to_list(&id, &req.account_ids).await?;

    Ok(Json(serde_json::json!({})))
}

/// DELETE /api/v1/lists/:id/accounts
/// Remove accounts from a list
pub async fn delete_list_accounts(
    State(state): State<ListsApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<AddAccountsRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify list exists
    state.db.get_list(&id).await?.ok_or(AppError::NotFound)?;

    state
        .db
        .remove_accounts_from_list(&id, &req.account_ids)
        .await?;

    Ok(Json(serde_json::json!({})))
}
