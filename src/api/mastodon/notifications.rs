//! Notification endpoints

use axum::{
    extract::{Path, RawQuery, State},
    http::{HeaderMap, header::LINK},
    response::{IntoResponse, Json},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

use super::accounts::PaginationParams;
use super::accounts::{
    build_remote_account_placeholder_response, resolve_account_response_for_identity,
};
use crate::TimelineApiState;
use crate::auth::CurrentUser;
use crate::data::{NotificationType, PersistedReason, Status, StatusVisibility};
use crate::error::AppError;

#[derive(Debug, Deserialize)]
pub struct NotificationQueryParams {
    pub pagination: PaginationParams,
    pub account_id: Option<String>,
    pub include_filtered: Option<bool>,
}

fn parse_notification_query(
    raw_query: Option<&str>,
) -> Result<
    (
        NotificationQueryParams,
        Vec<String>,
        Vec<String>,
        Vec<String>,
    ),
    AppError,
> {
    let mut pagination = PaginationParams {
        max_id: None,
        since_id: None,
        min_id: None,
        limit: None,
    };
    let mut account_id = None;
    let mut include_filtered = None;

    let mut include = Vec::new();
    let mut exclude = Vec::new();
    let mut grouped = Vec::new();
    let Some(raw_query) = raw_query else {
        return Ok((
            NotificationQueryParams {
                pagination,
                account_id,
                include_filtered,
            },
            include,
            exclude,
            grouped,
        ));
    };

    for (key, value) in url::form_urlencoded::parse(raw_query.as_bytes()) {
        match key.as_ref() {
            "max_id" => pagination.max_id = Some(value.into_owned()),
            "since_id" => pagination.since_id = Some(value.into_owned()),
            "min_id" => pagination.min_id = Some(value.into_owned()),
            "limit" => {
                pagination.limit = Some(value.parse::<usize>().map_err(|_| {
                    AppError::Validation("limit must be a positive integer".to_string())
                })?)
            }
            "types[]" => include.push(value.into_owned()),
            "exclude_types[]" => exclude.push(value.into_owned()),
            "grouped_types[]" => grouped.push(value.into_owned()),
            "account_id" => account_id = Some(value.into_owned()),
            "include_filtered" => {
                include_filtered = Some(matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "on" | "yes"
                ))
            }
            _ => {}
        }
    }

    Ok((
        NotificationQueryParams {
            pagination,
            account_id,
            include_filtered,
        },
        include,
        exclude,
        grouped,
    ))
}

fn parse_notification_type_filter(raw: &str) -> Option<NotificationType> {
    match raw.trim() {
        "mention" => Some(NotificationType::Mention),
        "favourite" => Some(NotificationType::Favourite),
        "reblog" => Some(NotificationType::Reblog),
        "follow" => Some(NotificationType::Follow),
        "follow_request" => Some(NotificationType::FollowRequest),
        "status" => Some(NotificationType::Status),
        "poll" => Some(NotificationType::Poll),
        "update" => Some(NotificationType::Update),
        "admin.sign_up" => Some(NotificationType::AdminSignUp),
        "admin.report" => Some(NotificationType::AdminReport),
        "severed_relationships" => Some(NotificationType::SeveredRelationships),
        "moderation_warning" => Some(NotificationType::ModerationWarning),
        "quote" => Some(NotificationType::Quote),
        "quoted_update" => Some(NotificationType::QuotedUpdate),
        _ => None,
    }
}

fn notification_is_included(
    notification_type: NotificationType,
    include_types: &[NotificationType],
    exclude_types: &[NotificationType],
) -> bool {
    if !include_types.is_empty() && !include_types.iter().any(|ty| *ty == notification_type) {
        return false;
    }

    !exclude_types.iter().any(|ty| *ty == notification_type)
}

fn notification_is_newer_than(
    notification: &crate::data::Notification,
    cursor: &(chrono::DateTime<chrono::Utc>, String),
) -> bool {
    notification.created_at > cursor.0
        || (notification.created_at == cursor.0 && notification.id > cursor.1)
}

pub(crate) fn notification_group_key(notification: &crate::data::Notification) -> String {
    let scope = notification
        .status_uri
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(notification.origin_account_address.as_str());
    format!(
        "{}::{}",
        notification.notification_type.as_str(),
        URL_SAFE_NO_PAD.encode(scope)
    )
}

fn notification_type_supports_grouping(notification_type: NotificationType) -> bool {
    matches!(
        notification_type,
        NotificationType::Favourite
            | NotificationType::Follow
            | NotificationType::Reblog
            | NotificationType::AdminSignUp
    )
}

fn notification_group_key_v2(
    notification: &crate::data::Notification,
    grouped_types: &[NotificationType],
) -> String {
    let supported = notification_type_supports_grouping(notification.notification_type);
    let enabled = grouped_types.is_empty()
        || grouped_types
            .iter()
            .any(|value| *value == notification.notification_type);
    if supported && enabled {
        notification_group_key(notification)
    } else {
        format!("ungrouped-{}", notification.id)
    }
}

fn parse_notification_group_key(
    group_key: &str,
) -> Result<Option<(NotificationType, String)>, AppError> {
    if let Some(id) = group_key.strip_prefix("ungrouped-") {
        if id.trim().is_empty() {
            return Err(AppError::Validation(
                "invalid notification group key".to_string(),
            ));
        }
        return Ok(None);
    }

    let (raw_type, encoded_scope) = group_key
        .split_once("::")
        .ok_or_else(|| AppError::Validation("invalid notification group key".to_string()))?;
    let notification_type = parse_notification_type_filter(raw_type)
        .ok_or_else(|| AppError::Validation("invalid notification group key".to_string()))?;
    let scope = String::from_utf8(
        URL_SAFE_NO_PAD
            .decode(encoded_scope)
            .map_err(|_| AppError::Validation("invalid notification group key".to_string()))?,
    )
    .map_err(|_| AppError::Validation("invalid notification group key".to_string()))?;
    if scope.trim().is_empty() {
        return Err(AppError::Validation(
            "invalid notification group key".to_string(),
        ));
    }
    Ok(Some((notification_type, scope)))
}

async fn load_notification_group(
    state: &TimelineApiState,
    group_key: &str,
) -> Result<Vec<crate::data::Notification>, AppError> {
    match parse_notification_group_key(group_key)? {
        None => {
            let id = group_key
                .strip_prefix("ungrouped-")
                .expect("checked prefix above");
            let notification = state
                .db
                .get_notification(id)
                .await?
                .ok_or(AppError::NotFound)?;
            Ok(vec![notification])
        }
        Some((notification_type, scope)) => {
            let notifications = state
                .db
                .get_notifications_by_group_scope(notification_type, &scope)
                .await?;
            if notifications.is_empty() {
                return Err(AppError::NotFound);
            }
            Ok(notifications)
        }
    }
}

async fn delete_notification_group(
    state: &TimelineApiState,
    group_key: &str,
) -> Result<u64, AppError> {
    match parse_notification_group_key(group_key)? {
        None => {
            let id = group_key
                .strip_prefix("ungrouped-")
                .expect("checked prefix above");
            Ok(u64::from(state.db.delete_notification(id).await?))
        }
        Some((notification_type, scope)) => {
            state
                .db
                .delete_notifications_by_group_scope(notification_type, &scope)
                .await
        }
    }
}

async fn build_grouped_notifications_result(
    state: &TimelineApiState,
    notifications: Vec<crate::data::Notification>,
    grouped_types: &[NotificationType],
) -> Result<serde_json::Value, AppError> {
    let mut groups: Vec<serde_json::Value> = Vec::new();
    let mut accounts = Vec::new();
    let mut seen_account_ids = HashSet::new();
    let mut statuses = Vec::new();
    let mut seen_status_ids = HashSet::new();
    let mut group_positions = HashMap::<String, usize>::new();

    for notification in notifications {
        let group_key = notification_group_key_v2(&notification, grouped_types);
        let account =
            build_notification_account_response(state, &notification.origin_account_address)
                .await?;
        if seen_account_ids.insert(account.id.clone()) {
            accounts.push(serde_json::to_value(account.clone()).map_err(|error| {
                AppError::serialization("notification v2 account response", error)
            })?);
        }

        let status_id = if let Some(status_uri) = &notification.status_uri {
            if let Some(status) = get_notification_status(state, status_uri).await {
                let status_response = build_notification_status_response(state, &status).await?;
                if seen_status_ids.insert(status_response.id.clone()) {
                    statuses.push(serde_json::to_value(status_response.clone()).map_err(
                        |error| AppError::serialization("notification v2 status response", error),
                    )?);
                }
                Some(status_response.id)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(index) = group_positions.get(&group_key).copied() {
            let group = groups
                .get_mut(index)
                .expect("existing notification group index must be valid");
            let sample_account_ids = group["sample_account_ids"]
                .as_array_mut()
                .expect("sample_account_ids should be an array");
            let account_id_value = serde_json::json!(account.id.clone());
            if !sample_account_ids.contains(&account_id_value) {
                sample_account_ids.push(account_id_value);
            }
            group["notifications_count"] =
                serde_json::json!(group["notifications_count"].as_u64().unwrap_or_default() + 1);
            group["page_min_id"] = serde_json::json!(notification.id);
            if group["status_id"].is_null() && status_id.is_some() {
                group["status_id"] = serde_json::json!(status_id);
            }
            continue;
        }

        group_positions.insert(group_key.clone(), groups.len());
        groups.push(serde_json::json!({
            "group_key": group_key,
            "type": notification.notification_type.to_string(),
            "latest_page_notification_at": notification.created_at,
            "most_recent_notification_id": notification.id,
            "page_min_id": notification.id,
            "page_max_id": notification.id,
            "notifications_count": 1,
            "sample_account_ids": [account.id.clone()],
            "status_id": status_id,
        }));
    }

    Ok(serde_json::json!({
        "accounts": accounts,
        "statuses": statuses,
        "notification_groups": groups,
    }))
}

async fn load_notifications_page(
    state: &TimelineApiState,
    params: &NotificationQueryParams,
    include_types: &[NotificationType],
    exclude_types: &[NotificationType],
) -> Result<(Vec<crate::data::Notification>, bool), AppError> {
    let limit = params.pagination.limit.unwrap_or(20).min(80);
    let fetch_limit = (limit + 1).max(80);
    let include_filtered = params.include_filtered.unwrap_or(false);
    let mut notifications = Vec::new();
    let mut cursor = params.pagination.max_id.clone();
    let min_cursor = if let Some(cursor_id) = params.pagination.min_id.as_deref() {
        state
            .db
            .get_notification(cursor_id)
            .await?
            .map(|notification| (notification.created_at, notification.id))
    } else {
        None
    };
    let since_cursor = if let Some(cursor_id) = params.pagination.since_id.as_deref() {
        state
            .db
            .get_notification(cursor_id)
            .await?
            .map(|notification| (notification.created_at, notification.id))
    } else {
        None
    };

    while notifications.len() <= limit {
        let batch = state
            .db
            .get_notifications(fetch_limit, cursor.as_deref(), false)
            .await?;
        if batch.is_empty() {
            break;
        }

        let reached_end = batch.len() < fetch_limit;
        cursor = batch.last().map(|notification| notification.id.clone());

        for notification in batch {
            if min_cursor
                .as_ref()
                .is_some_and(|cursor| !notification_is_newer_than(&notification, cursor))
            {
                continue;
            }
            if since_cursor
                .as_ref()
                .is_some_and(|cursor| !notification_is_newer_than(&notification, cursor))
            {
                continue;
            }
            if notification_is_included(
                notification.notification_type,
                include_types,
                exclude_types,
            ) {
                if let Some(account_id) = params.account_id.as_deref()
                    && !notification_matches_account_id(state, &notification, account_id).await
                {
                    continue;
                }
                if !include_filtered
                    && let Some(status_uri) = notification.status_uri.as_deref()
                    && let Some(status) = get_notification_status(state, status_uri).await
                    && !crate::api::load_status_filtered_for_context(
                        state.db.as_ref(),
                        &status,
                        Some("notifications"),
                    )
                    .await?
                    .is_empty()
                {
                    continue;
                }
                notifications.push(notification);
                if notifications.len() > limit {
                    break;
                }
            }
        }

        if notifications.len() > limit || reached_end {
            break;
        }
    }

    let has_next = notifications.len() > limit;
    if has_next {
        notifications.truncate(limit);
    }
    if params.pagination.min_id.is_some() {
        notifications.reverse();
    }

    Ok((notifications, has_next))
}

async fn notification_matches_account_id(
    state: &TimelineApiState,
    notification: &crate::data::Notification,
    account_id: &str,
) -> bool {
    if let Some(account) = resolve_account_response_for_identity(
        state.config.as_ref(),
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        None,
        &notification.origin_account_address,
    )
    .await
    {
        return account.id == account_id
            || account.acct.eq_ignore_ascii_case(account_id)
            || account.uri.eq_ignore_ascii_case(account_id)
            || notification
                .origin_account_address
                .eq_ignore_ascii_case(account_id);
    }

    build_remote_account_placeholder_response(
        &notification.origin_account_address,
        state.config.as_ref(),
        0,
    )
    .is_some_and(|account| {
        account.id == account_id
            || account.acct.eq_ignore_ascii_case(account_id)
            || account.uri.eq_ignore_ascii_case(account_id)
            || notification
                .origin_account_address
                .eq_ignore_ascii_case(account_id)
    })
}

async fn get_notification_status(state: &TimelineApiState, status_uri: &str) -> Option<Status> {
    if let Ok(status) = state.db.get_status_by_uri(status_uri).await
        && status.is_some()
    {
        return status;
    }

    let cached = state.timeline_cache.get_by_uri(status_uri).await?;
    Some(Status {
        id: cached.id.clone(),
        uri: cached.uri.clone(),
        content: cached.content.clone(),
        content_warning: cached.content_warning.clone(),
        visibility: StatusVisibility::parse(&cached.visibility)
            .unwrap_or(StatusVisibility::Private),
        language: cached.language.clone(),
        account_address: cached.account_address.clone(),
        is_local: false,
        in_reply_to_uri: cached.reply_to_uri.clone(),
        boost_of_uri: cached.boost_of_uri.clone(),
        quote_of_uri: cached.quote_of_uri.clone(),
        persisted_reason: PersistedReason::CacheOnly,
        created_at: cached.created_at,
        fetched_at: Some(chrono::Utc::now()),
    })
}

async fn build_notification_status_response(
    state: &TimelineApiState,
    status: &Status,
) -> Result<crate::api::dto::StatusResponse, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    let status_batch = vec![status.clone()];
    let remote_account_stats = crate::api::load_remote_account_stats_map(
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        &state.config.server.protocol,
        &status_batch,
    )
    .await
    .unwrap_or_default();
    let remote_stats = remote_account_stats
        .get(status.account_address.trim())
        .cloned();
    let thread_uri = state.db.resolve_thread_root_uri(status).await?;
    let interactions = crate::api::StatusInteractions::new(
        Some(state.db.is_favourited(&status.id).await?),
        Some(state.db.is_reposted(&status.id).await?),
        Some(state.db.is_thread_muted(&thread_uri).await?),
        Some(state.db.is_bookmarked(&status.id).await?),
        Some(state.db.is_status_pinned(&status.id).await?),
    );

    if matches!(status.persisted_reason, PersistedReason::CacheOnly) {
        let cached = if let Some(cached) = state.timeline_cache.get(&status.id).await {
            Some(cached)
        } else {
            state.timeline_cache.get_by_uri(&status.uri).await
        };
        let media = cached
            .as_ref()
            .map(|cached| {
                cached
                    .attachments
                    .iter()
                    .map(crate::api::cached_media_attachment_to_response)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut response = crate::api::status_to_response_with_media(
            status,
            &account,
            &state.config,
            account_stats,
            remote_stats.clone(),
            interactions,
            remote_stats
                .as_ref()
                .map(|stats| stats.force_sensitive)
                .unwrap_or(false),
            &media,
        );
        if let Some(cached) = cached.as_ref() {
            crate::api::apply_cached_status_metadata(&mut response, &cached);
        }
        response.filtered = crate::api::load_status_filtered_for_context(
            state.db.as_ref(),
            status,
            Some("notifications"),
        )
        .await?;
        return Ok(response);
    }

    let mut response = crate::api::build_status_response_with_account_stats_and_remote_stats(
        state.db.as_ref(),
        status,
        &account,
        &state.config,
        account_stats,
        remote_stats,
        interactions,
    )
    .await?;
    crate::api::apply_filtered_context(&mut response, "notifications");
    Ok(response)
}

async fn build_notification_account_response(
    state: &TimelineApiState,
    origin_account_address: &str,
) -> Result<crate::api::dto::AccountResponse, AppError> {
    if let Some(account) = resolve_account_response_for_identity(
        state.config.as_ref(),
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        None,
        origin_account_address,
    )
    .await
    {
        return Ok(account);
    }

    build_remote_account_placeholder_response(origin_account_address, state.config.as_ref(), 0)
        .ok_or(AppError::NotFound)
}

fn notification_link_header(
    endpoint: &str,
    limit: usize,
    first_id: Option<&str>,
    last_id: Option<&str>,
    has_prev: bool,
    has_next: bool,
    include_types: &[String],
    exclude_types: &[String],
    grouped_types: &[String],
    account_id: Option<&str>,
    include_filtered: Option<bool>,
) -> Option<String> {
    let build_path = |cursor_key: &str, cursor_value: &str| {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        serializer.append_pair("limit", &limit.to_string());
        serializer.append_pair(cursor_key, cursor_value);
        for include in include_types {
            serializer.append_pair("types[]", include);
        }
        for exclude in exclude_types {
            serializer.append_pair("exclude_types[]", exclude);
        }
        for grouped in grouped_types {
            serializer.append_pair("grouped_types[]", grouped);
        }
        if let Some(account_id) = account_id.filter(|value| !value.is_empty()) {
            serializer.append_pair("account_id", account_id);
        }
        if let Some(include_filtered) = include_filtered {
            serializer.append_pair(
                "include_filtered",
                if include_filtered { "true" } else { "false" },
            );
        }
        format!("{endpoint}?{}", serializer.finish())
    };

    let mut links = Vec::new();
    if has_next && let Some(last_id) = last_id.filter(|value| !value.is_empty()) {
        links.push(format!("<{}>; rel=\"next\"", build_path("max_id", last_id)));
    }
    if has_prev && let Some(first_id) = first_id.filter(|value| !value.is_empty()) {
        links.push(format!(
            "<{}>; rel=\"prev\"",
            build_path("min_id", first_id)
        ));
    }
    (!links.is_empty()).then(|| links.join(", "))
}

fn has_prev_cursor(params: &PaginationParams) -> bool {
    params.min_id.is_some() || params.since_id.is_some()
}

/// GET /api/v1/notifications
pub async fn get_notifications(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    raw_query: RawQuery,
) -> Result<impl IntoResponse, AppError> {
    use crate::api::dto::NotificationResponse;

    let (params, raw_include_types, raw_exclude_types, raw_grouped_types) =
        parse_notification_query(raw_query.0.as_deref())?;
    let limit = params.pagination.limit.unwrap_or(20).min(80);
    let include_types = raw_include_types
        .iter()
        .filter_map(|value| parse_notification_type_filter(value))
        .collect::<Vec<_>>();
    let exclude_types = raw_exclude_types
        .iter()
        .filter_map(|value| parse_notification_type_filter(value))
        .collect::<Vec<_>>();
    let (notifications, has_next) =
        load_notifications_page(&state, &params, &include_types, &exclude_types).await?;

    // Convert to API responses
    let mut responses = vec![];
    for notification in notifications {
        // Get status if present
        let status = if let Some(status_uri) = &notification.status_uri {
            get_notification_status(&state, status_uri).await
        } else {
            None
        };

        let status_response = if let Some(status) = status {
            Some(build_notification_status_response(&state, &status).await?)
        } else {
            None
        };

        let response = NotificationResponse {
            id: notification.id.clone(),
            notification_type: notification.notification_type.to_string(),
            group_key: notification_group_key(&notification),
            created_at: notification.created_at,
            account: build_notification_account_response(
                &state,
                &notification.origin_account_address,
            )
            .await?,
            status: status_response,
            report: None,
            event: None,
            moderation_warning: None,
        };

        responses.push(serde_json::to_value(response).unwrap());
    }
    let has_prev = has_prev_cursor(&params.pagination);

    let first_id = responses
        .first()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let last_id = responses
        .last()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let mut headers = HeaderMap::new();
    if let Some(link) = notification_link_header(
        "/api/v1/notifications",
        limit,
        first_id,
        last_id,
        has_prev,
        has_next,
        &raw_include_types,
        &raw_exclude_types,
        &raw_grouped_types,
        params.account_id.as_deref(),
        params.include_filtered,
    ) {
        headers.insert(
            LINK,
            link.parse()
                .map_err(|_| AppError::Validation("invalid Link header".to_string()))?,
        );
    }

    Ok((headers, Json(responses)))
}

/// GET /api/v2/notifications
pub async fn get_notifications_v2(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    raw_query: RawQuery,
) -> Result<impl IntoResponse, AppError> {
    let (params, raw_include_types, raw_exclude_types, raw_grouped_types) =
        parse_notification_query(raw_query.0.as_deref())?;
    let limit = params.pagination.limit.unwrap_or(20).min(80);
    let include_types = raw_include_types
        .iter()
        .filter_map(|value| parse_notification_type_filter(value))
        .collect::<Vec<_>>();
    let exclude_types = raw_exclude_types
        .iter()
        .filter_map(|value| parse_notification_type_filter(value))
        .collect::<Vec<_>>();
    let grouped_types = raw_grouped_types
        .iter()
        .filter_map(|value| parse_notification_type_filter(value))
        .collect::<Vec<_>>();
    let (notifications, has_next) =
        load_notifications_page(&state, &params, &include_types, &exclude_types).await?;
    let body = build_grouped_notifications_result(&state, notifications, &grouped_types).await?;
    let has_prev = has_prev_cursor(&params.pagination);

    let first_id = body["notification_groups"]
        .as_array()
        .and_then(|groups| groups.first())
        .and_then(|value| value.get("most_recent_notification_id"))
        .and_then(|value| value.as_str());
    let last_id = body["notification_groups"]
        .as_array()
        .and_then(|groups| groups.last())
        .and_then(|value| value.get("page_min_id"))
        .and_then(|value| value.as_str());
    let mut headers = HeaderMap::new();
    if let Some(link) = notification_link_header(
        "/api/v2/notifications",
        limit,
        first_id,
        last_id,
        has_prev,
        has_next,
        &raw_include_types,
        &raw_exclude_types,
        &raw_grouped_types,
        params.account_id.as_deref(),
        params.include_filtered,
    ) {
        headers.insert(
            LINK,
            link.parse()
                .map_err(|_| AppError::Validation("invalid Link header".to_string()))?,
        );
    }

    Ok((headers, Json(body)))
}

/// GET /api/v2/notifications/:group_key
pub async fn get_notification_group(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    Path(group_key): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let notifications = load_notification_group(&state, &group_key).await?;
    let body = build_grouped_notifications_result(&state, notifications, &[]).await?;
    Ok(Json(body))
}

/// POST /api/v2/notifications/:group_key/dismiss
pub async fn dismiss_notification_group(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    Path(group_key): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = delete_notification_group(&state, &group_key).await?;
    if deleted == 0 {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({})))
}

/// GET /api/v2/notifications/:group_key/accounts
pub async fn get_notification_group_accounts(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    Path(group_key): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let notifications = load_notification_group(&state, &group_key).await?;
    let body = build_grouped_notifications_result(&state, notifications, &[]).await?;
    Ok(Json(body["accounts"].clone()))
}

/// POST /api/v1/notifications/:id/dismiss
pub async fn dismiss_notification(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !state.db.delete_notification(&id).await? {
        return Err(AppError::NotFound);
    }

    Ok(Json(serde_json::json!({})))
}

/// POST /api/v1/notifications/clear
pub async fn clear_notifications(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    state.db.clear_notifications().await?;

    Ok(Json(serde_json::json!({})))
}

/// GET /api/v1/notifications/:id
/// Get a single notification by ID
pub async fn get_notification(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::NotificationResponse;

    let notification = state
        .db
        .get_notification(&id)
        .await?
        .ok_or(AppError::NotFound)?;

    // Get status if present
    let status = if let Some(status_uri) = &notification.status_uri {
        get_notification_status(&state, status_uri).await
    } else {
        None
    };

    let status_response = if let Some(status) = status {
        Some(build_notification_status_response(&state, &status).await?)
    } else {
        None
    };

    let response = NotificationResponse {
        id: notification.id.clone(),
        notification_type: notification.notification_type.to_string(),
        group_key: notification_group_key(&notification),
        created_at: notification.created_at,
        account: build_notification_account_response(&state, &notification.origin_account_address)
            .await?,
        status: status_response,
        report: None,
        event: None,
        moderation_warning: None,
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/notifications/unread_count
/// Get the count of unread notifications
pub async fn get_unread_count(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let count = state.db.count_unread_notifications().await?;

    Ok(Json(serde_json::json!({
        "count": count
    })))
}

/// GET /api/v2/notifications/unread_count
pub async fn get_unread_count_v2(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let count = state.db.count_unread_notifications().await?;
    Ok(Json(serde_json::json!({
        "count": count
    })))
}
