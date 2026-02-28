//! Bookmark and Favourite endpoints

use axum::{
    extract::{Query, State},
    response::Json,
};

use super::accounts::PaginationParams;
use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::service::TimelineService;

/// GET /api/v1/bookmarks
pub async fn get_bookmarks(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    let limit = params.limit.unwrap_or(20).min(40);
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let timeline_items = timeline_service
        .bookmarks_timeline(limit, params.max_id.as_deref())
        .await?;

    // Convert to API responses
    let mut responses = vec![];
    for item in &timeline_items {
        let response = crate::api::status_to_response(
            &item.status,
            &account,
            &state.config,
            Some(item.favourited),
            Some(item.reblogged),
            None,
            Some(item.bookmarked),
            None,
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
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let timeline_items = timeline_service
        .favourites_timeline(limit, params.max_id.as_deref())
        .await?;

    // Convert to API responses
    let mut responses = vec![];
    for item in &timeline_items {
        let response = crate::api::status_to_response(
            &item.status,
            &account,
            &state.config,
            Some(item.favourited),
            Some(item.reblogged),
            None,
            Some(item.bookmarked),
            None,
        );
        responses.push(serde_json::to_value(response).unwrap());
    }

    Ok(Json(responses))
}
