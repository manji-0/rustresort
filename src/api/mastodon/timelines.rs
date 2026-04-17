//! Timeline endpoints

use axum::{
    extract::{Query, State},
    response::Json,
};
use serde::Deserialize;
use std::collections::HashSet;

use super::accounts::PaginationParams;
use crate::TimelineApiState;
use crate::auth::CurrentUser;
use crate::data::ListTimelineQuery;
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
    pub remote: Option<bool>,
    pub only_media: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct TagTimelineParams {
    #[serde(flatten)]
    pub pagination: PaginationParams,
    pub local: Option<bool>,
    pub only_media: Option<bool>,
    #[serde(default, rename = "any[]")]
    pub any: Vec<String>,
    #[serde(default, rename = "all[]")]
    pub all: Vec<String>,
    #[serde(default, rename = "none[]")]
    pub none: Vec<String>,
}

async fn status_has_media(state: &TimelineApiState, status_id: &str) -> bool {
    state
        .db
        .status_has_any_media(status_id)
        .await
        .unwrap_or(false)
}

fn normalize_tag_set(tags: &[String]) -> Vec<String> {
    tags.iter()
        .map(|tag| tag.trim().trim_start_matches('#').to_ascii_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn status_matches_tag_filters(
    status: &crate::data::Status,
    any: &[String],
    all: &[String],
    none: &[String],
) -> bool {
    let present = crate::data::extract_hashtags_from_content(&status.content)
        .into_iter()
        .map(|tag| tag.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>();

    if !any.is_empty() && !any.iter().any(|tag| present.contains(tag)) {
        return false;
    }
    if !all.iter().all(|tag| present.contains(tag)) {
        return false;
    }
    if none.iter().any(|tag| present.contains(tag)) {
        return false;
    }
    true
}

/// GET /api/v1/timelines/home
pub async fn home_timeline(
    State(state): State<TimelineApiState>,
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
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "accounts"])
        .inc();
    db_timer.observe_duration();

    let limit = params.limit.unwrap_or(20).min(40);
    let effective_min_id = params.min_id.as_deref().or(params.since_id.as_deref());
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "statuses"])
        .start_timer();
    let timeline_items = timeline_service
        .home_timeline(limit, params.max_id.as_deref(), effective_min_id)
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
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "statuses"])
        .inc();
    db_timer.observe_duration();

    // Convert to API responses
    let mut responses = Vec::with_capacity(timeline_items.len());
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
                None,
                Some(item.bookmarked),
                None,
            ),
        )
        .await?;
        responses.push(serde_json::to_value(response).unwrap());
    }

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/v1/timelines/home", "200"])
        .inc();

    Ok(Json(responses))
}

/// GET /api/v1/timelines/public
pub async fn public_timeline(
    State(state): State<TimelineApiState>,
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
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "accounts"])
        .inc();
    db_timer.observe_duration();

    let limit = params.pagination.limit.unwrap_or(20).min(40);
    let local_only = params.local.unwrap_or(false);
    let remote_only = params.remote.unwrap_or(false);
    let only_media = params.only_media.unwrap_or(false);
    let effective_min_id = params
        .pagination
        .min_id
        .as_deref()
        .or(params.pagination.since_id.as_deref());
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "statuses"])
        .start_timer();
    let fetch_limit = if only_media || remote_only {
        limit.saturating_mul(3).min(120)
    } else {
        limit
    };
    let mut timeline_items = Vec::new();
    let mut seen_status_ids = HashSet::new();
    let mut next_max_id = params.pagination.max_id.clone();

    loop {
        let batch = timeline_service
            .public_timeline(
                local_only,
                fetch_limit,
                next_max_id.as_deref(),
                effective_min_id,
            )
            .await?;
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        next_max_id = batch.last().map(|item| item.status.id.clone());

        for item in batch {
            if !seen_status_ids.insert(item.status.id.clone()) {
                continue;
            }
            if remote_only && item.status.is_local {
                continue;
            }
            if only_media && !status_has_media(&state, &item.status.id).await {
                continue;
            }
            timeline_items.push(item);
            if timeline_items.len() >= limit {
                break;
            }
        }

        if timeline_items.len() >= limit || batch_len < fetch_limit || next_max_id.is_none() {
            break;
        }
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
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "statuses"])
        .inc();
    db_timer.observe_duration();

    // Convert to API responses
    let mut responses = Vec::with_capacity(timeline_items.len());
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
                None,
                Some(item.bookmarked),
                None,
            ),
        )
        .await?;
        responses.push(serde_json::to_value(response).unwrap());
    }

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/v1/timelines/public", "200"])
        .inc();

    Ok(Json(responses))
}

/// GET /api/v1/timelines/tag/:hashtag
/// Get statuses with a specific hashtag
pub async fn tag_timeline(
    State(state): State<TimelineApiState>,
    axum::extract::Path(hashtag): axum::extract::Path<String>,
    Query(params): Query<TagTimelineParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let limit = params.pagination.limit.unwrap_or(20).min(40);
    let effective_min_id = params
        .pagination
        .min_id
        .as_deref()
        .or(params.pagination.since_id.as_deref());
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let fetch_limit = if params.local.unwrap_or(false)
        || params.only_media.unwrap_or(false)
        || !params.any.is_empty()
        || !params.all.is_empty()
        || !params.none.is_empty()
    {
        limit.saturating_mul(3).min(120)
    } else {
        limit
    };
    let any = normalize_tag_set(&params.any);
    let all = normalize_tag_set(&params.all);
    let none = normalize_tag_set(&params.none);
    let mut timeline_items = Vec::new();
    let mut seen_status_ids = HashSet::new();
    let mut next_max_id = params.pagination.max_id.clone();
    let local_only = params.local.unwrap_or(false);
    let only_media = params.only_media.unwrap_or(false);

    loop {
        let batch = timeline_service
            .tag_timeline(
                &hashtag,
                fetch_limit,
                next_max_id.as_deref(),
                effective_min_id,
            )
            .await?;
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        next_max_id = batch.last().map(|item| item.status.id.clone());

        for item in batch {
            if !seen_status_ids.insert(item.status.id.clone()) {
                continue;
            }
            if local_only && !item.status.is_local {
                continue;
            }
            if only_media && !status_has_media(&state, &item.status.id).await {
                continue;
            }
            if !status_matches_tag_filters(&item.status, &any, &all, &none) {
                continue;
            }
            timeline_items.push(item);
            if timeline_items.len() >= limit {
                break;
            }
        }

        if timeline_items.len() >= limit || batch_len < fetch_limit || next_max_id.is_none() {
            break;
        }
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

    let mut responses = Vec::with_capacity(timeline_items.len());
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
                None,
                Some(item.bookmarked),
                None,
            ),
        )
        .await?;
        responses.push(serde_json::to_value(response).unwrap());
    }

    Ok(Json(responses))
}

/// GET /api/v1/timelines/list/:list_id
/// Get statuses from a specific list
pub async fn list_timeline(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    axum::extract::Path(list_id): axum::extract::Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let list = state
        .db
        .get_list(&list_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    let local_account_address = format!("{}@{}", account.username, state.config.server.domain);
    let local_account_id = account.id.to_string();
    let default_port = match state.config.server.protocol.as_str() {
        "https" => Some(443),
        "http" => Some(80),
        _ => None,
    };

    let limit = params.limit.unwrap_or(20).min(40);
    let effective_min_id = params.min_id.clone().or(params.since_id.clone());
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let timeline_items = if list.2 == "none" {
        let mut collected = Vec::with_capacity(limit);
        let mut cursor = params.max_id.clone();
        let min_id = effective_min_id.clone();

        while collected.len() < limit {
            let query = ListTimelineQuery {
                list_id: list_id.clone(),
                local_account_address: local_account_address.clone(),
                local_account_id: local_account_id.clone(),
                default_port,
                limit,
                max_id: cursor.clone(),
                min_id: min_id.clone(),
            };
            let page = timeline_service.list_timeline(&query).await?;
            if page.is_empty() {
                break;
            }
            let fetched_count = page.len();
            cursor = page.last().map(|item| item.status.id.clone());

            for item in page {
                if item.status.in_reply_to_uri.is_none() {
                    collected.push(item);
                    if collected.len() >= limit {
                        break;
                    }
                }
            }

            if fetched_count < limit || cursor.is_none() {
                break;
            }
        }

        collected
    } else {
        let query = ListTimelineQuery {
            list_id: list_id.clone(),
            local_account_address: local_account_address.clone(),
            local_account_id: local_account_id.clone(),
            default_port,
            limit,
            max_id: params.max_id.clone(),
            min_id: effective_min_id,
        };
        timeline_service.list_timeline(&query).await?
    };
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

    let mut responses = Vec::with_capacity(timeline_items.len());
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
                None,
                Some(item.bookmarked),
                None,
            ),
        )
        .await?;
        responses.push(serde_json::to_value(response).unwrap());
    }

    Ok(Json(responses))
}
