//! Bookmark and Favourite endpoints

use axum::{
    extract::{Query, State},
    response::Json,
};

use super::accounts::PaginationParams;
use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::AppError;

/// GET /api/v1/bookmarks
pub async fn get_bookmarks(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    let limit = params.limit.unwrap_or(20).min(40);
    let statuses = state
        .db
        .get_bookmarked_statuses(limit, params.max_id.as_deref())
        .await?;

    // Convert to API responses
    let mut responses = vec![];
    for status in &statuses {
        let favourited = state.db.is_favourited(&status.id).await.ok();
        let response = crate::api::status_to_response(
            status,
            &account,
            &state.config,
            favourited,
            Some(false),
            Some(true), // bookmarked=true
        );
        responses.push(serde_json::to_value(response).unwrap());
    }

    Ok(Json(responses))
}

/// GET /api/v1/favourites
pub async fn get_favourites(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    let limit = params.limit.unwrap_or(20).min(40);
    let statuses = state
        .db
        .get_favourited_statuses(limit, params.max_id.as_deref())
        .await?;

    // Convert to API responses
    let mut responses = vec![];
    for status in &statuses {
        let bookmarked = state.db.is_bookmarked(&status.id).await.ok();
        let response = crate::api::status_to_response(
            status,
            &account,
            &state.config,
            Some(true), // favourited=true
            Some(false),
            bookmarked,
        );
        responses.push(serde_json::to_value(response).unwrap());
    }

    Ok(Json(responses))
}
