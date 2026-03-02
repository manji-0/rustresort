//! Bookmark and Favourite endpoints

use axum::{
    extract::{Query, State},
    response::Json,
};

use super::accounts::PaginationParams;
use crate::TimelineApiState;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::service::TimelineService;

/// GET /api/v1/bookmarks
pub async fn get_bookmarks(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let limit = params.limit.unwrap_or(20).min(40);
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let timeline_items = timeline_service
        .bookmarks_timeline(limit, params.max_id.as_deref())
        .await?;
    let timeline_statuses: Vec<_> = timeline_items
        .iter()
        .map(|item| item.status.clone())
        .collect();
    let remote_account_stats = crate::api::load_remote_account_stats_map(
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        &state.config.server.protocol,
        &timeline_statuses,
    )
    .await?;

    // Convert to API responses
    let mut responses = vec![];
    for item in &timeline_items {
        let remote_stats = remote_account_stats
            .get(item.status.account_address.trim())
            .copied();
        let response = crate::api::status_to_response_with_account_stats_and_remote_stats(
            &item.status,
            &account,
            &state.config,
            account_stats,
            remote_stats,
            crate::api::StatusInteractions::new(
                Some(item.favourited),
                Some(item.reblogged),
                None,
                Some(item.bookmarked),
                None,
            ),
        );
        responses.push(serde_json::to_value(response).unwrap());
    }

    Ok(Json(responses))
}

/// GET /api/v1/favourites
pub async fn get_favourites(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let limit = params.limit.unwrap_or(20).min(40);
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let timeline_items = timeline_service
        .favourites_timeline(limit, params.max_id.as_deref())
        .await?;
    let timeline_statuses: Vec<_> = timeline_items
        .iter()
        .map(|item| item.status.clone())
        .collect();
    let remote_account_stats = crate::api::load_remote_account_stats_map(
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        &state.config.server.protocol,
        &timeline_statuses,
    )
    .await?;

    // Convert to API responses
    let mut responses = vec![];
    for item in &timeline_items {
        let remote_stats = remote_account_stats
            .get(item.status.account_address.trim())
            .copied();
        let response = crate::api::status_to_response_with_account_stats_and_remote_stats(
            &item.status,
            &account,
            &state.config,
            account_stats,
            remote_stats,
            crate::api::StatusInteractions::new(
                Some(item.favourited),
                Some(item.reblogged),
                None,
                Some(item.bookmarked),
                None,
            ),
        );
        responses.push(serde_json::to_value(response).unwrap());
    }

    Ok(Json(responses))
}
