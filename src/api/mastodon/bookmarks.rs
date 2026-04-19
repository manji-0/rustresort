//! Bookmark and Favourite endpoints

use axum::{
    extract::{Query, State},
    http::{HeaderMap, header::LINK},
    response::IntoResponse,
    response::Json,
};

use super::accounts::PaginationParams;
use crate::TimelineApiState;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::service::TimelineService;

fn has_prev_cursor(params: &PaginationParams) -> bool {
    params.min_id.is_some() || params.since_id.is_some()
}

/// GET /api/v1/bookmarks
pub async fn get_bookmarks(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<impl IntoResponse, AppError> {
    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let limit = params.limit.unwrap_or(20).min(40);
    let lower_bound_id = params.min_id.as_deref().or(params.since_id.as_deref());
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let timeline_items = timeline_service
        .bookmarks_timeline(limit + 1, params.max_id.as_deref(), lower_bound_id)
        .await?;
    let has_next = timeline_items.len() > limit;
    let mut timeline_items = timeline_items;
    if has_next {
        timeline_items.truncate(limit);
    }
    if params.min_id.is_some() {
        timeline_items.reverse();
    }
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
            .cloned();
        let response = crate::api::build_status_response_with_account_stats_and_remote_stats(
            state.db.as_ref(),
            &item.status,
            &account,
            &state.config,
            account_stats,
            remote_stats,
            crate::api::StatusInteractions::new(
                Some(item.favourited),
                Some(item.reblogged),
                Some(item.muted),
                Some(item.bookmarked),
                Some(item.pinned),
            ),
        )
        .await?;
        responses.push(serde_json::to_value(response).unwrap());
    }

    let first_id = responses
        .first()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let last_id = responses
        .last()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let mut headers = HeaderMap::new();
    if let Some(link) = collection_link_header(
        "/api/v1/bookmarks",
        limit,
        first_id,
        last_id,
        has_prev_cursor(&params),
        has_next,
    ) {
        headers.insert(LINK, link.parse().expect("valid link header"));
    }

    Ok((headers, Json(responses)))
}

/// GET /api/v1/favourites
pub async fn get_favourites(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<impl IntoResponse, AppError> {
    // Get account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let limit = params.limit.unwrap_or(20).min(40);
    let lower_bound_id = params.min_id.as_deref().or(params.since_id.as_deref());
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let timeline_items = timeline_service
        .favourites_timeline(limit + 1, params.max_id.as_deref(), lower_bound_id)
        .await?;
    let has_next = timeline_items.len() > limit;
    let mut timeline_items = timeline_items;
    if has_next {
        timeline_items.truncate(limit);
    }
    if params.min_id.is_some() {
        timeline_items.reverse();
    }
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
            .cloned();
        let response = crate::api::build_status_response_with_account_stats_and_remote_stats(
            state.db.as_ref(),
            &item.status,
            &account,
            &state.config,
            account_stats,
            remote_stats,
            crate::api::StatusInteractions::new(
                Some(item.favourited),
                Some(item.reblogged),
                Some(item.muted),
                Some(item.bookmarked),
                Some(item.pinned),
            ),
        )
        .await?;
        responses.push(serde_json::to_value(response).unwrap());
    }

    let first_id = responses
        .first()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let last_id = responses
        .last()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let mut headers = HeaderMap::new();
    if let Some(link) = collection_link_header(
        "/api/v1/favourites",
        limit,
        first_id,
        last_id,
        has_prev_cursor(&params),
        has_next,
    ) {
        headers.insert(LINK, link.parse().expect("valid link header"));
    }

    Ok((headers, Json(responses)))
}

fn collection_link_header(
    path: &str,
    limit: usize,
    first_id: Option<&str>,
    last_id: Option<&str>,
    has_prev: bool,
    has_next: bool,
) -> Option<String> {
    let build = |cursor_key: &str, cursor_value: &str| {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        serializer.append_pair("limit", &limit.to_string());
        serializer.append_pair(cursor_key, cursor_value);
        format!("{path}?{}", serializer.finish())
    };

    let mut links = Vec::new();
    if has_next && let Some(last_id) = last_id.filter(|value| !value.is_empty()) {
        links.push(format!("<{}>; rel=\"next\"", build("max_id", last_id)));
    }
    if has_prev && let Some(first_id) = first_id.filter(|value| !value.is_empty()) {
        links.push(format!("<{}>; rel=\"prev\"", build("min_id", first_id)));
    }
    (!links.is_empty()).then(|| links.join(", "))
}
