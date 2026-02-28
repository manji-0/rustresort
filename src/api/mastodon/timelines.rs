//! Timeline endpoints

use axum::{
    extract::{Query, State},
    response::Json,
};
use serde::Deserialize;

use super::accounts::PaginationParams;
use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::metrics::{
    DB_QUERIES_TOTAL, DB_QUERY_DURATION_SECONDS, HTTP_REQUEST_DURATION_SECONDS, HTTP_REQUESTS_TOTAL,
};
use crate::service::TimelineService;

#[derive(Debug, Deserialize)]
pub struct PublicTimelineParams {
    #[serde(flatten)]
    pub pagination: PaginationParams,
    pub local: Option<bool>,
}

/// GET /api/v1/timelines/home
pub async fn home_timeline(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["GET", "/api/v1/timelines/home"])
        .start_timer();

    // Get account
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "accounts"])
        .start_timer();
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "accounts"])
        .inc();
    db_timer.observe_duration();

    let limit = params.limit.unwrap_or(20).min(40);
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "statuses"])
        .start_timer();
    let timeline_items = timeline_service
        .home_timeline(limit, params.max_id.as_deref(), params.min_id.as_deref())
        .await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "statuses"])
        .inc();
    db_timer.observe_duration();

    // Convert to API responses
    let responses: Vec<_> = timeline_items
        .iter()
        .map(|item| {
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
            serde_json::to_value(response).unwrap()
        })
        .collect();

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/v1/timelines/home", "200"])
        .inc();

    Ok(Json(responses))
}

/// GET /api/v1/timelines/public
pub async fn public_timeline(
    State(state): State<AppState>,
    Query(params): Query<PublicTimelineParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["GET", "/api/v1/timelines/public"])
        .start_timer();

    // Get account
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "accounts"])
        .start_timer();
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "accounts"])
        .inc();
    db_timer.observe_duration();

    let limit = params.pagination.limit.unwrap_or(20).min(40);
    let local_only = params.local.unwrap_or(false);
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "statuses"])
        .start_timer();
    let timeline_items = timeline_service
        .public_timeline(local_only, limit, params.pagination.max_id.as_deref())
        .await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "statuses"])
        .inc();
    db_timer.observe_duration();

    // Convert to API responses
    let responses: Vec<_> = timeline_items
        .iter()
        .map(|item| {
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
            serde_json::to_value(response).unwrap()
        })
        .collect();

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/v1/timelines/public", "200"])
        .inc();

    Ok(Json(responses))
}

/// GET /api/v1/timelines/tag/:hashtag
/// Get statuses with a specific hashtag
pub async fn tag_timeline(
    State(state): State<AppState>,
    axum::extract::Path(hashtag): axum::extract::Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    let limit = params.limit.unwrap_or(20).min(40);
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let timeline_items = timeline_service
        .tag_timeline(
            &hashtag,
            limit,
            params.max_id.as_deref(),
            params.min_id.as_deref(),
        )
        .await?;

    let responses: Vec<_> = timeline_items
        .iter()
        .map(|item| {
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
            serde_json::to_value(response).unwrap()
        })
        .collect();

    Ok(Json(responses))
}

/// GET /api/v1/timelines/list/:list_id
/// Get statuses from a specific list
pub async fn list_timeline(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    axum::extract::Path(list_id): axum::extract::Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    state
        .db
        .get_list(&list_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let local_account_address = format!("{}@{}", account.username, state.config.server.domain);
    let local_account_id = account.id.clone();

    let limit = params.limit.unwrap_or(20).min(40);
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let timeline_items = timeline_service
        .list_timeline(
            &list_id,
            &local_account_address,
            &local_account_id,
            limit,
            params.max_id.as_deref(),
            params.min_id.as_deref(),
        )
        .await?;

    let responses: Vec<_> = timeline_items
        .iter()
        .map(|item| {
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
            serde_json::to_value(response).unwrap()
        })
        .collect();

    Ok(Json(responses))
}
