//! Timeline endpoints

use axum::{
    extract::{Query, State},
    response::Json,
};

use super::accounts::PaginationParams;
use crate::api::metrics::{DB_QUERIES_TOTAL, DB_QUERY_DURATION_SECONDS, HTTP_REQUESTS_TOTAL, HTTP_REQUEST_DURATION_SECONDS};
use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::AppError;

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
    DB_QUERIES_TOTAL.with_label_values(&["SELECT", "accounts"]).inc();
    db_timer.observe_duration();

    // Get local statuses (home timeline = local statuses for single-user instance)
    let limit = params.limit.unwrap_or(20).min(40);
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "statuses"])
        .start_timer();
    let statuses = state
        .db
        .get_local_statuses(limit, params.max_id.as_deref())
        .await?;
    DB_QUERIES_TOTAL.with_label_values(&["SELECT", "statuses"]).inc();
    db_timer.observe_duration();

    // Convert to API responses
    let responses: Vec<_> = statuses
        .iter()
        .map(|status| {
            let response =
                crate::api::status_to_response(status, &account, &state.config, None, None, None);
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
    Query(params): Query<PaginationParams>,
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
    DB_QUERIES_TOTAL.with_label_values(&["SELECT", "accounts"]).inc();
    db_timer.observe_duration();

    // Get local statuses (public timeline = local statuses for single-user instance)
    let limit = params.limit.unwrap_or(20).min(40);
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "statuses"])
        .start_timer();
    let statuses = state
        .db
        .get_local_statuses(limit, params.max_id.as_deref())
        .await?;
    DB_QUERIES_TOTAL.with_label_values(&["SELECT", "statuses"]).inc();
    db_timer.observe_duration();

    // Filter to only include public visibility statuses
    let public_statuses: Vec<_> = statuses
        .iter()
        .filter(|status| status.visibility == "public")
        .collect();

    // Convert to API responses
    let responses: Vec<_> = public_statuses
        .iter()
        .map(|status| {
            let response =
                crate::api::status_to_response(status, &account, &state.config, None, None, None);
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
///
/// For single-user instance without hashtag indexing,
/// this returns an empty array.
pub async fn tag_timeline(
    State(_state): State<AppState>,
    axum::extract::Path(_hashtag): axum::extract::Path<String>,
    Query(_params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // TODO: Implement hashtag indexing and filtering
    // For now, return empty array as hashtags are not indexed
    // In a full implementation, we would:
    // 1. Parse statuses for hashtags during creation
    // 2. Store hashtag -> status_id mappings in a separate table
    // 3. Query that table to find statuses with the given hashtag

    Ok(Json(vec![]))
}

/// GET /api/v1/timelines/list/:list_id
/// Get statuses from a specific list
///
/// For single-user instance, lists are not supported,
/// so this always returns an error.
pub async fn list_timeline(
    State(_state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    axum::extract::Path(_list_id): axum::extract::Path<String>,
    Query(_params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Lists not implemented for single-user instance
    Err(AppError::NotFound)
}
