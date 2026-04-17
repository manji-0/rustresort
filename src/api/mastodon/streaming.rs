//! Streaming API endpoints
//!
//! Provides real-time updates via Server-Sent Events (SSE)

use axum::{
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::IntoResponse,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::{self, Stream};
use serde::Deserialize;
use serde_json::Value;
use std::convert::Infallible;
use tokio::sync::broadcast;

use super::accounts::resolve_account_response_for_identity;
use crate::StreamingApiState;
use crate::auth::CurrentUser;
use crate::data::{PersistedReason, Status, StatusVisibility};
use crate::error::AppError;
use crate::service::{EventReceiver, StreamEvent};

#[derive(Debug, Deserialize)]
pub struct StreamParams {
    stream: Option<String>,
    /// Only for hashtag stream
    tag: Option<String>,
    /// Whether the hashtag stream should be restricted to local statuses.
    local: Option<bool>,
    /// Only for list stream
    list: Option<String>,
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// GET /api/v1/streaming/health
/// Health check for streaming endpoint
pub async fn streaming_health() -> impl IntoResponse {
    "OK"
}

fn json_value_to_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
}

fn cached_status_to_status(cached: &crate::data::CachedStatus) -> Status {
    Status {
        id: cached.id.clone(),
        uri: cached.uri.clone(),
        content: cached.content.clone(),
        content_warning: None,
        visibility: StatusVisibility::parse(&cached.visibility)
            .unwrap_or(StatusVisibility::Private),
        language: None,
        account_address: cached.account_address.clone(),
        is_local: false,
        in_reply_to_uri: cached.reply_to_uri.clone(),
        boost_of_uri: cached.boost_of_uri.clone(),
        quote_of_uri: cached.quote_of_uri.clone(),
        persisted_reason: PersistedReason::CacheOnly,
        created_at: cached.created_at,
        fetched_at: Some(chrono::Utc::now()),
    }
}

async fn get_notification_status(state: &StreamingApiState, status_uri: &str) -> Option<Status> {
    if let Ok(status) = state.db.get_status_by_uri(status_uri).await
        && status.is_some()
    {
        return status;
    }

    let cached = state.timeline_cache.get_by_uri(status_uri).await?;
    Some(cached_status_to_status(&cached))
}

async fn load_stream_status(
    state: &StreamingApiState,
    payload: &Value,
) -> Result<Option<Status>, AppError> {
    if let Some(status_id) = payload.get("id").and_then(Value::as_str)
        && let Some(status) = state.db.get_status(status_id).await?
    {
        return Ok(Some(status));
    }
    if let Some(status_id) = payload.get("id").and_then(Value::as_str)
        && let Some(status) = state.timeline_cache.get(status_id).await
    {
        return Ok(Some(cached_status_to_status(&status)));
    }

    if let Some(status_uri) = payload.get("uri").and_then(Value::as_str)
        && let Some(status) = state.db.get_status_by_uri(status_uri).await?
    {
        return Ok(Some(status));
    }
    if let Some(status_uri) = payload.get("uri").and_then(Value::as_str)
        && let Some(status) = state.timeline_cache.get_by_uri(status_uri).await
    {
        return Ok(Some(cached_status_to_status(&status)));
    }

    Ok(None)
}

async fn build_status_response_value(
    state: &StreamingApiState,
    status: &Status,
) -> Result<Value, AppError> {
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

    let response = crate::api::build_status_response_with_account_stats_and_remote_stats(
        state.db.as_ref(),
        status,
        &account,
        &state.config,
        account_stats,
        remote_stats,
        interactions,
    )
    .await?;

    serde_json::to_value(response)
        .map_err(|error| AppError::serialization("streaming status payload", error))
}

async fn build_status_response(
    state: &StreamingApiState,
    status: &Status,
) -> Result<crate::api::dto::StatusResponse, AppError> {
    let value = build_status_response_value(state, status).await?;
    serde_json::from_value(value)
        .map_err(|error| AppError::serialization("streaming status response decode", error))
}

async fn build_notification_response_value(
    state: &StreamingApiState,
    payload: &Value,
) -> Result<Option<Value>, AppError> {
    let Some(notification_id) = payload.get("id").and_then(Value::as_str) else {
        return Ok(None);
    };

    let Some(notification) = state.db.get_notification(notification_id).await? else {
        return Ok(None);
    };

    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let status_response = if let Some(status_uri) = notification.status_uri.as_deref() {
        if let Some(status) = get_notification_status(state, status_uri).await {
            Some(build_status_response(state, &status).await?)
        } else {
            None
        }
    } else {
        None
    };

    let notification_id = notification.id.clone();
    let response = crate::api::NotificationResponse {
        id: notification_id.clone(),
        notification_type: notification.notification_type.to_string(),
        group_key: format!("ungrouped-{}", notification_id),
        created_at: notification.created_at,
        account: resolve_account_response_for_identity(
            state.config.as_ref(),
            state.db.as_ref(),
            state.profile_cache.as_ref(),
            None,
            &notification.origin_account_address,
        )
        .await
        .unwrap_or_else(|| {
            crate::api::account_to_response_with_stats(&account, &state.config, account_stats)
        }),
        status: status_response,
        report: None,
        event: None,
        moderation_warning: None,
    };

    serde_json::to_value(response)
        .map(Some)
        .map_err(|error| AppError::serialization("streaming notification payload", error))
}

async fn serialize_stream_event_data(state: &StreamingApiState, event: &StreamEvent) -> String {
    match event {
        StreamEvent::Delete { payload, .. } => payload
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| json_value_to_string(payload)),
        StreamEvent::Update { payload, .. } => match load_stream_status(state, payload).await {
            Ok(Some(status)) => match build_status_response_value(state, &status).await {
                Ok(value) => json_value_to_string(&value),
                Err(error) => {
                    tracing::warn!(%error, "failed to build streaming status payload");
                    json_value_to_string(payload)
                }
            },
            Ok(None) => json_value_to_string(payload),
            Err(error) => {
                tracing::warn!(%error, "failed to load status for streaming payload");
                json_value_to_string(payload)
            }
        },
        StreamEvent::Notification { payload, .. } => {
            match build_notification_response_value(state, payload).await {
                Ok(Some(value)) => json_value_to_string(&value),
                Ok(None) => json_value_to_string(payload),
                Err(error) => {
                    tracing::warn!(%error, "failed to build streaming notification payload");
                    json_value_to_string(payload)
                }
            }
        }
        _ => json_value_to_string(event.payload()),
    }
}

fn build_sse_stream(
    state: StreamingApiState,
    receiver: EventReceiver,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = stream::unfold(
        (state, receiver.into_inner()),
        |(state, mut receiver)| async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let data = serialize_stream_event_data(&state, &event).await;
                        let sse_event = Event::default().event(event.event_name()).data(data);
                        return Some((Ok(sse_event), (state, receiver)));
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "streaming receiver lagged; dropping old messages");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    );

    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn forward_websocket_stream(
    state: StreamingApiState,
    mut socket: WebSocket,
    receiver: EventReceiver,
    stream_name: String,
) {
    let mut receiver = receiver.into_inner();
    loop {
        match receiver.recv().await {
            Ok(event) => {
                let payload = serialize_stream_event_data(&state, &event).await;
                let frame = serde_json::json!({
                    "stream": [stream_name],
                    "event": event.event_name(),
                    "payload": payload,
                });
                if socket.send(Message::Text(frame.to_string())).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "streaming receiver lagged; dropping old messages");
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

/// GET /api/v1/streaming/user
/// Stream events for the authenticated user
pub async fn stream_user(
    State(state): State<StreamingApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let account_id = state.config.auth.username.as_str();
    let receiver = state.streaming_event_bus.subscribe_user(account_id).await?;
    Ok(build_sse_stream(state, receiver))
}

/// GET /api/v1/streaming/public
/// Stream public statuses
pub async fn stream_public(
    State(state): State<StreamingApiState>,
    Query(_params): Query<StreamParams>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let receiver = state.streaming_event_bus.subscribe_public().await?;
    Ok(build_sse_stream(state, receiver))
}

/// GET /api/v1/streaming/public/local
/// Stream local public statuses
pub async fn stream_public_local(
    State(state): State<StreamingApiState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let receiver = state.streaming_event_bus.subscribe_public_local().await?;
    Ok(build_sse_stream(state, receiver))
}

/// GET /api/v1/streaming/hashtag
/// Stream statuses with a specific hashtag
pub async fn stream_hashtag(
    State(state): State<StreamingApiState>,
    Query(params): Query<StreamParams>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let tag = params
        .tag
        .ok_or(AppError::Validation("tag parameter required".to_string()))?;

    let receiver = if params.local.unwrap_or(false) {
        state
            .streaming_event_bus
            .subscribe_hashtag_local(&tag)
            .await?
    } else {
        state.streaming_event_bus.subscribe_hashtag(&tag).await?
    };
    Ok(build_sse_stream(state, receiver))
}

/// GET /api/v1/streaming/list
/// Stream statuses from a specific list
pub async fn stream_list(
    State(state): State<StreamingApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<StreamParams>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let list_id = params
        .list
        .ok_or(AppError::Validation("list parameter required".to_string()))?;

    state
        .db
        .get_list(&list_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let receiver = state.streaming_event_bus.subscribe_list(&list_id).await?;
    Ok(build_sse_stream(state, receiver))
}

/// GET /api/v1/streaming/direct
/// Stream direct messages
pub async fn stream_direct(
    State(state): State<StreamingApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let account_id = state.config.auth.username.as_str();
    let receiver = state
        .streaming_event_bus
        .subscribe_direct(account_id)
        .await?;
    Ok(build_sse_stream(state, receiver))
}

async fn enforce_root_stream_scopes(
    state: &StreamingApiState,
    headers: &HeaderMap,
    required_scopes: &[&str],
) -> Result<(), AppError> {
    let Some(token) = bearer_token(headers) else {
        return Ok(());
    };
    let oauth_token = state
        .db
        .get_oauth_token(token)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let granted = oauth_token.scopes.split_whitespace().collect::<Vec<_>>();
    let granted_matches = |required: &&str| {
        granted.iter().any(|scope| {
            scope == required
                || required
                    .split_once(':')
                    .map(|(prefix, _)| scope == &prefix)
                    .unwrap_or(false)
        })
    };
    if required_scopes.iter().all(granted_matches) {
        return Ok(());
    }
    Err(AppError::Forbidden)
}

/// GET /api/v1/streaming
/// Mastodon-compatible streaming multiplexer using the `stream` query parameter.
pub async fn stream_root(
    State(state): State<StreamingApiState>,
    session: CurrentUser,
    headers: HeaderMap,
    ws: Option<WebSocketUpgrade>,
    Query(params): Query<StreamParams>,
) -> Result<impl IntoResponse, AppError> {
    let stream_name = params.stream.clone().unwrap_or_default();
    match stream_name.as_str() {
        "user" => {
            enforce_root_stream_scopes(&state, &headers, &["read:statuses", "read:notifications"])
                .await?;
            let account_id = state.config.auth.username.clone();
            let receiver = state
                .streaming_event_bus
                .subscribe_user(account_id.as_str())
                .await?;
            if let Some(ws) = ws {
                return Ok(ws
                    .on_upgrade(move |socket| {
                        forward_websocket_stream(state, socket, receiver, stream_name)
                    })
                    .into_response());
            }
            stream_user(State(state), session)
                .await
                .map(IntoResponse::into_response)
        }
        "public" => {
            enforce_root_stream_scopes(&state, &headers, &["read:statuses"]).await?;
            let receiver = state.streaming_event_bus.subscribe_public().await?;
            if let Some(ws) = ws {
                return Ok(ws
                    .on_upgrade(move |socket| {
                        forward_websocket_stream(state, socket, receiver, stream_name)
                    })
                    .into_response());
            }
            stream_public(State(state), Query(params))
                .await
                .map(IntoResponse::into_response)
        }
        "public:local" => {
            enforce_root_stream_scopes(&state, &headers, &["read:statuses"]).await?;
            let receiver = state.streaming_event_bus.subscribe_public_local().await?;
            if let Some(ws) = ws {
                return Ok(ws
                    .on_upgrade(move |socket| {
                        forward_websocket_stream(state, socket, receiver, stream_name)
                    })
                    .into_response());
            }
            stream_public_local(State(state))
                .await
                .map(IntoResponse::into_response)
        }
        "hashtag" | "hashtag:local" => {
            enforce_root_stream_scopes(&state, &headers, &["read:statuses"]).await?;
            let tag = params
                .tag
                .clone()
                .ok_or(AppError::Validation("tag parameter required".to_string()))?;
            let local_hashtag = stream_name == "hashtag:local" || params.local.unwrap_or(false);
            let receiver = if local_hashtag {
                state.streaming_event_bus.subscribe_hashtag_local(&tag).await?
            } else {
                state.streaming_event_bus.subscribe_hashtag(&tag).await?
            };
            if let Some(ws) = ws {
                let ws_stream_name = if local_hashtag {
                    "hashtag:local".to_string()
                } else {
                    stream_name.clone()
                };
                return Ok(ws
                    .on_upgrade(move |socket| {
                        forward_websocket_stream(state, socket, receiver, ws_stream_name)
                    })
                    .into_response());
            }
            stream_hashtag(State(state), Query(params))
                .await
                .map(IntoResponse::into_response)
        }
        "list" => {
            enforce_root_stream_scopes(&state, &headers, &["read:statuses"]).await?;
            let list_id = params
                .list
                .clone()
                .ok_or(AppError::Validation("list parameter required".to_string()))?;
            state
                .db
                .get_list(&list_id)
                .await?
                .ok_or(AppError::NotFound)?;
            let receiver = state.streaming_event_bus.subscribe_list(&list_id).await?;
            if let Some(ws) = ws {
                return Ok(ws
                    .on_upgrade(move |socket| {
                        forward_websocket_stream(state, socket, receiver, stream_name)
                    })
                    .into_response());
            }
            stream_list(State(state), session, Query(params))
                .await
                .map(IntoResponse::into_response)
        }
        "direct" => {
            enforce_root_stream_scopes(&state, &headers, &["read:statuses"]).await?;
            let account_id = state.config.auth.username.clone();
            let receiver = state
                .streaming_event_bus
                .subscribe_direct(account_id.as_str())
                .await?;
            if let Some(ws) = ws {
                return Ok(ws
                    .on_upgrade(move |socket| {
                        forward_websocket_stream(state, socket, receiver, stream_name)
                    })
                    .into_response());
            }
            stream_direct(State(state), session)
                .await
                .map(IntoResponse::into_response)
        }
        _ => Err(AppError::Validation(
            "stream parameter must be one of user, public, public:local, hashtag, hashtag:local, list, direct"
                .to_string(),
        )),
    }
}
