//! Lists endpoints

use axum::{
    body::to_bytes,
    extract::Request,
    extract::{Path, Query, State},
    http::{
        HeaderMap,
        header::{CONTENT_TYPE, LINK},
    },
    response::{IntoResponse, Json},
};
use futures::stream::{self, StreamExt};
use serde::de::DeserializeOwned;
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
    pub exclusive: bool,
}

/// Create list request
#[derive(Debug, Deserialize)]
pub struct CreateListRequest {
    pub title: String,
    pub replies_policy: Option<String>, // "followed", "list", "none"
    pub exclusive: Option<bool>,
}

/// Update list request
#[derive(Debug, Deserialize)]
pub struct UpdateListRequest {
    pub title: Option<String>,
    pub replies_policy: Option<String>,
    pub exclusive: Option<bool>,
}

/// Add accounts to list request
#[derive(Debug, Deserialize)]
pub struct AddAccountsRequest {
    #[serde(rename = "account_ids")]
    pub account_ids: Vec<String>,
}

fn parse_add_accounts_request(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<AddAccountsRequest, AppError> {
    parse_json_or_form_body(headers, body)
}

fn parse_json_or_form_body<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<T, AppError> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if content_type.starts_with("application/json") || content_type.is_empty() {
        return serde_json::from_slice(body)
            .map_err(|error| AppError::Validation(format!("invalid JSON body: {error}")));
    }

    if content_type.starts_with("application/x-www-form-urlencoded") {
        return serde_urlencoded::from_bytes(body)
            .map_err(|error| AppError::Validation(format!("invalid form body: {error}")));
    }

    Err(AppError::Validation(
        "unsupported content type for list account payload".to_string(),
    ))
}

/// Pagination parameters
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    pub max_id: Option<String>,
    pub since_id: Option<String>,
    pub min_id: Option<String>,
    pub limit: Option<usize>,
}

fn list_collection_link_header(
    path: &str,
    limit: usize,
    first_id: Option<&str>,
    last_id: Option<&str>,
) -> Option<String> {
    let build_path = |cursor_key: &str, cursor_value: &str| {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        serializer.append_pair("limit", &limit.to_string());
        serializer.append_pair(cursor_key, cursor_value);
        format!("{path}?{}", serializer.finish())
    };

    let mut links = Vec::new();
    if let Some(last_id) = last_id.filter(|value| !value.is_empty()) {
        links.push(format!("<{}>; rel=\"next\"", build_path("max_id", last_id)));
    }
    if let Some(first_id) = first_id.filter(|value| !value.is_empty()) {
        links.push(format!(
            "<{}>; rel=\"prev\"",
            build_path("min_id", first_id)
        ));
    }
    (!links.is_empty()).then(|| links.join(", "))
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
        .map(|(id, title, replies_policy, exclusive)| ListResponse {
            id,
            title,
            replies_policy,
            exclusive,
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
        exclusive: list.3,
    }))
}

/// POST /api/v1/lists
/// Create a new list
pub async fn create_list(
    State(state): State<ListsApiState>,
    CurrentUser(_session): CurrentUser,
    request: Request,
) -> Result<Json<ListResponse>, AppError> {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 64 * 1024)
        .await
        .map_err(|error| AppError::Validation(format!("failed to read request body: {error}")))?;
    let req = if body.is_empty() {
        CreateListRequest {
            title: String::new(),
            replies_policy: None,
            exclusive: None,
        }
    } else {
        parse_json_or_form_body::<CreateListRequest>(&parts.headers, &body)?
    };

    // Validate title
    if req.title.trim().is_empty() {
        return Err(AppError::Validation("Title cannot be empty".to_string()));
    }

    // Default replies_policy to "list"
    let replies_policy = req.replies_policy.unwrap_or_else(|| "list".to_string());
    let exclusive = req.exclusive.unwrap_or(false);

    // Validate replies_policy
    if !["followed", "list", "none"].contains(&replies_policy.as_str()) {
        return Err(AppError::Validation(
            "Invalid replies_policy. Must be 'followed', 'list', or 'none'".to_string(),
        ));
    }

    let id = state
        .db
        .create_list(&req.title, &replies_policy, exclusive)
        .await?;

    Ok(Json(ListResponse {
        id,
        title: req.title,
        replies_policy,
        exclusive,
    }))
}

/// PUT /api/v1/lists/:id
/// Update a list
pub async fn update_list(
    State(state): State<ListsApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<ListResponse>, AppError> {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 64 * 1024)
        .await
        .map_err(|error| AppError::Validation(format!("failed to read request body: {error}")))?;
    let req = if body.is_empty() {
        UpdateListRequest {
            title: None,
            replies_policy: None,
            exclusive: None,
        }
    } else {
        parse_json_or_form_body::<UpdateListRequest>(&parts.headers, &body)?
    };

    // Get existing list
    let existing = state.db.get_list(&id).await?.ok_or(AppError::NotFound)?;

    // Use existing values if not provided
    let title = req.title.unwrap_or(existing.1.clone());
    let replies_policy = req.replies_policy.unwrap_or(existing.2.clone());
    let exclusive = req.exclusive.unwrap_or(existing.3);

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

    state
        .db
        .update_list(&id, &title, &replies_policy, exclusive)
        .await?;

    Ok(Json(ListResponse {
        id,
        title,
        replies_policy,
        exclusive,
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
) -> Result<impl IntoResponse, AppError> {
    // Verify list exists
    state.db.get_list(&id).await?.ok_or(AppError::NotFound)?;

    // Get account addresses in list
    let addresses = state.db.get_list_accounts(&id).await?;
    let limit = params.limit.unwrap_or(40).min(80);
    let default_port = match state.config.server.protocol.to_ascii_lowercase().as_str() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };

    let mut accounts = stream::iter(addresses)
        .map(|address| {
            let state = state.clone();
            async move {
                if let Some(account) = super::accounts::resolve_account_response_for_identity(
                    state.config.as_ref(),
                    state.db.as_ref(),
                    state.profile_cache.as_ref(),
                    Some(state.federation_fetch_client.as_ref()),
                    &address,
                )
                .await
                {
                    return serde_json::to_value(account).unwrap_or_default();
                }

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

    if let Some(max_id) = params.max_id.as_deref() {
        accounts.retain(|account| {
            account
                .get("id")
                .and_then(|value| value.as_str())
                .is_some_and(|value| value < max_id)
        });
    }
    if let Some(min_id) = params.min_id.as_deref() {
        accounts.retain(|account| {
            account
                .get("id")
                .and_then(|value| value.as_str())
                .is_some_and(|value| value > min_id)
        });
        accounts.reverse();
    } else if let Some(since_id) = params.since_id.as_deref() {
        accounts.retain(|account| {
            account
                .get("id")
                .and_then(|value| value.as_str())
                .is_some_and(|value| value > since_id)
        });
    }
    accounts.truncate(limit);

    let first_id = accounts
        .first()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let last_id = accounts
        .last()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let mut headers = HeaderMap::new();
    if let Some(link) = list_collection_link_header(
        &format!("/api/v1/lists/{id}/accounts"),
        limit,
        first_id,
        last_id,
    ) {
        headers.insert(
            LINK,
            link.parse()
                .map_err(|_| AppError::Validation("invalid Link header".to_string()))?,
        );
    }

    Ok((headers, Json(accounts)))
}

/// POST /api/v1/lists/:id/accounts
/// Add accounts to a list
pub async fn add_list_accounts(
    State(state): State<ListsApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify list exists
    state.db.get_list(&id).await?.ok_or(AppError::NotFound)?;
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 64 * 1024)
        .await
        .map_err(|error| AppError::Validation(format!("failed to read request body: {error}")))?;
    let req = if body.is_empty() {
        AddAccountsRequest {
            account_ids: Vec::new(),
        }
    } else {
        parse_add_accounts_request(&parts.headers, &body)?
    };

    let local_account = state.db.get_account().await?;
    let normalized_ids = req
        .account_ids
        .into_iter()
        .map(|account_id| {
            let trimmed = account_id.trim().to_string();
            if let Some(account) = local_account.as_ref()
                && account.id == trimmed
            {
                return format!("{}@{}", account.username, state.config.server.domain);
            }
            trimmed
        })
        .collect::<Vec<_>>();

    state.db.add_accounts_to_list(&id, &normalized_ids).await?;

    Ok(Json(serde_json::json!({})))
}

/// DELETE /api/v1/lists/:id/accounts
/// Remove accounts from a list
pub async fn delete_list_accounts(
    State(state): State<ListsApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify list exists
    state.db.get_list(&id).await?.ok_or(AppError::NotFound)?;
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 64 * 1024)
        .await
        .map_err(|error| AppError::Validation(format!("failed to read request body: {error}")))?;
    let req = if body.is_empty() {
        AddAccountsRequest {
            account_ids: Vec::new(),
        }
    } else {
        parse_add_accounts_request(&parts.headers, &body)?
    };

    let local_account = state.db.get_account().await?;
    let normalized_ids = req
        .account_ids
        .into_iter()
        .map(|account_id| {
            let trimmed = account_id.trim().to_string();
            if let Some(account) = local_account.as_ref()
                && account.id == trimmed
            {
                return format!("{}@{}", account.username, state.config.server.domain);
            }
            trimmed
        })
        .collect::<Vec<_>>();

    state
        .db
        .remove_accounts_from_list(&id, &normalized_ids)
        .await?;

    Ok(Json(serde_json::json!({})))
}
