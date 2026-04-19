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
use axum_extra::extract::CookieJar;
use futures::{
    SinkExt, StreamExt,
    stream::{self, Stream},
};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, convert::Infallible};
use tokio::sync::broadcast;
use tokio::{sync::mpsc, task::JoinHandle};

use super::accounts::{
    build_remote_account_placeholder_response, resolve_account_response_for_identity,
};
use crate::StreamingApiState;
use crate::auth::CurrentUser;
use crate::data::{PersistedReason, Status, StatusVisibility};
use crate::error::AppError;
use crate::service::{EventReceiver, StreamEvent};

#[derive(Debug, Deserialize)]
pub struct StreamParams {
    stream: Option<String>,
    access_token: Option<String>,
    /// Only for hashtag stream
    tag: Option<String>,
    /// Whether the hashtag stream should be restricted to local statuses.
    local: Option<bool>,
    /// Only for list stream
    list: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WebSocketControlMessage {
    #[serde(rename = "type")]
    message_type: String,
    stream: serde_json::Value,
    tag: Option<String>,
    list: Option<String>,
}

#[derive(Clone, Debug)]
struct StreamSubscription {
    stream_name: String,
    tag: Option<String>,
    list: Option<String>,
    local: bool,
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn websocket_protocol_tokens(headers: &HeaderMap) -> Vec<&str> {
    headers
        .get("Sec-WebSocket-Protocol")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

#[derive(Clone)]
enum StreamAuth {
    Session,
    OAuth { scopes: Vec<String> },
}

fn scope_matches(granted: &str, required: &str) -> bool {
    granted == required
        || required
            .strip_prefix(granted)
            .is_some_and(|suffix| suffix.starts_with(':'))
}

fn oauth_scopes_satisfy(granted: &[String], required: &[&str]) -> bool {
    if required.is_empty() {
        return true;
    }
    required.iter().all(|required_scope| {
        granted
            .iter()
            .any(|granted_scope| scope_matches(granted_scope, required_scope))
    })
}

async fn authenticate_stream_request(
    state: &StreamingApiState,
    headers: &HeaderMap,
    jar: &CookieJar,
    query_access_token: Option<&str>,
) -> Result<Option<StreamAuth>, AppError> {
    let mut candidate_tokens = Vec::new();
    if let Some(token) = bearer_token(headers) {
        candidate_tokens.push(token);
    }
    if let Some(token) = query_access_token {
        candidate_tokens.push(token);
    }
    candidate_tokens.extend(websocket_protocol_tokens(headers));

    let had_candidate_token = !candidate_tokens.is_empty();
    for token in candidate_tokens {
        if let Some(oauth_token) = state.db.get_oauth_token(token).await?
            && matches!(
                oauth_token.grant_type.as_str(),
                "authorization_code" | "refresh_token"
            )
        {
            let scopes = oauth_token
                .scopes
                .split_whitespace()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            return Ok(Some(StreamAuth::OAuth { scopes }));
        }
        if crate::auth::verify_session_token(token, &state.config.auth.session_secret).is_ok() {
            return Ok(Some(StreamAuth::Session));
        }
    }

    if had_candidate_token {
        return Err(AppError::Unauthorized);
    }

    if jar
        .get("session")
        .map(|cookie| cookie.value())
        .is_some_and(|token| {
            crate::auth::verify_session_token(token, &state.config.auth.session_secret).is_ok()
        })
    {
        return Ok(Some(StreamAuth::Session));
    }

    Ok(None)
}

fn enforce_stream_auth(
    auth: Option<&StreamAuth>,
    required_scopes: &[&str],
) -> Result<(), AppError> {
    match auth {
        Some(StreamAuth::Session) => Ok(()),
        Some(StreamAuth::OAuth { scopes }) if oauth_scopes_satisfy(scopes, required_scopes) => {
            Ok(())
        }
        Some(StreamAuth::OAuth { .. }) => Err(AppError::Forbidden),
        None => Err(AppError::Unauthorized),
    }
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
    viewer_scoped: bool,
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

    let interactions = if viewer_scoped {
        let thread_uri = state.db.resolve_thread_root_uri(status).await?;
        crate::api::StatusInteractions::new(
            Some(state.db.is_favourited(&status.id).await?),
            Some(state.db.is_reposted(&status.id).await?),
            Some(state.db.is_thread_muted(&thread_uri).await?),
            Some(state.db.is_bookmarked(&status.id).await?),
            Some(state.db.is_status_pinned(&status.id).await?),
        )
    } else {
        crate::api::StatusInteractions::public()
    };

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
    viewer_scoped: bool,
) -> Result<crate::api::dto::StatusResponse, AppError> {
    let value = build_status_response_value(state, status, viewer_scoped).await?;
    serde_json::from_value(value)
        .map_err(|error| AppError::serialization("streaming status response decode", error))
}

async fn build_notification_response_value(
    state: &StreamingApiState,
    payload: &Value,
    viewer_scoped: bool,
) -> Result<Option<Value>, AppError> {
    let Some(notification_id) = payload.get("id").and_then(Value::as_str) else {
        return Ok(None);
    };

    let Some(notification) = state.db.get_notification(notification_id).await? else {
        return Ok(None);
    };

    let status_response = if let Some(status_uri) = notification.status_uri.as_deref() {
        if let Some(status) = get_notification_status(state, status_uri).await {
            Some(build_status_response(state, &status, viewer_scoped).await?)
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
        group_key: super::notifications::notification_group_key(&notification),
        created_at: notification.created_at,
        account: resolve_account_response_for_identity(
            state.config.as_ref(),
            state.db.as_ref(),
            state.profile_cache.as_ref(),
            None,
            &notification.origin_account_address,
        )
        .await
        .or_else(|| {
            build_remote_account_placeholder_response(
                &notification.origin_account_address,
                state.config.as_ref(),
                0,
            )
        })
        .ok_or(AppError::NotFound)?,
        status: status_response,
        report: None,
        event: None,
        moderation_warning: None,
    };

    serde_json::to_value(response)
        .map(Some)
        .map_err(|error| AppError::serialization("streaming notification payload", error))
}

async fn serialize_stream_event_data(
    state: &StreamingApiState,
    event: &StreamEvent,
    viewer_scoped: bool,
) -> String {
    match event {
        StreamEvent::Delete { payload, .. } => payload
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| json_value_to_string(payload)),
        StreamEvent::Update { payload, .. } => match load_stream_status(state, payload).await {
            Ok(Some(status)) => {
                match build_status_response_value(state, &status, viewer_scoped).await {
                    Ok(value) => json_value_to_string(&value),
                    Err(error) => {
                        tracing::warn!(%error, "failed to build streaming status payload");
                        json_value_to_string(payload)
                    }
                }
            }
            Ok(None) => json_value_to_string(payload),
            Err(error) => {
                tracing::warn!(%error, "failed to load status for streaming payload");
                json_value_to_string(payload)
            }
        },
        StreamEvent::Notification { payload, .. } => {
            match build_notification_response_value(state, payload, viewer_scoped).await {
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
    viewer_scoped: bool,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = stream::unfold(
        (state, receiver.into_inner()),
        move |(state, mut receiver)| async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let data = serialize_stream_event_data(&state, &event, viewer_scoped).await;
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
    viewer_scoped: bool,
) {
    let mut receiver = receiver.into_inner();
    loop {
        match receiver.recv().await {
            Ok(event) => {
                let payload = serialize_stream_event_data(&state, &event, viewer_scoped).await;
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

async fn subscribe_stream(
    state: &StreamingApiState,
    auth: Option<&StreamAuth>,
    subscription: &StreamSubscription,
) -> Result<(EventReceiver, String), AppError> {
    match subscription.stream_name.as_str() {
        "user" => {
            enforce_stream_auth(auth, &["read:statuses", "read:notifications"])?;
            let account_id = state.config.auth.username.clone();
            Ok((
                state.streaming_event_bus.subscribe_user(account_id.as_str()).await?,
                "user".to_string(),
            ))
        }
        "public" => {
            if let Some(auth) = auth {
                enforce_stream_auth(Some(auth), &["read:statuses"])?;
            }
            Ok((state.streaming_event_bus.subscribe_public().await?, "public".to_string()))
        }
        "public:local" => {
            if let Some(auth) = auth {
                enforce_stream_auth(Some(auth), &["read:statuses"])?;
            }
            Ok((
                state.streaming_event_bus.subscribe_public_local().await?,
                "public:local".to_string(),
            ))
        }
        "hashtag" | "hashtag:local" => {
            if let Some(auth) = auth {
                enforce_stream_auth(Some(auth), &["read:statuses"])?;
            }
            let tag = subscription
                .tag
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or(AppError::Validation("tag parameter required".to_string()))?;
            let local_hashtag = subscription.stream_name == "hashtag:local" || subscription.local;
            let receiver = if local_hashtag {
                state.streaming_event_bus.subscribe_hashtag_local(tag).await?
            } else {
                state.streaming_event_bus.subscribe_hashtag(tag).await?
            };
            Ok((
                receiver,
                if local_hashtag {
                    "hashtag:local".to_string()
                } else {
                    "hashtag".to_string()
                },
            ))
        }
        "list" => {
            enforce_stream_auth(auth, &["read:statuses"])?;
            let list_id = subscription
                .list
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or(AppError::Validation("list parameter required".to_string()))?;
            state.db.get_list(list_id).await?.ok_or(AppError::NotFound)?;
            Ok((
                state.streaming_event_bus.subscribe_list(list_id).await?,
                "list".to_string(),
            ))
        }
        "direct" => {
            enforce_stream_auth(auth, &["read:statuses"])?;
            let account_id = state.config.auth.username.clone();
            Ok((
                state
                    .streaming_event_bus
                    .subscribe_direct(account_id.as_str())
                    .await?,
                "direct".to_string(),
            ))
        }
        _ => Err(AppError::Validation(
            "stream parameter must be one of user, public, public:local, hashtag, hashtag:local, list, direct"
                .to_string(),
        )),
    }
}

fn parse_ws_subscriptions(
    message: WebSocketControlMessage,
) -> Result<Vec<StreamSubscription>, AppError> {
    let stream_names = match message.stream {
        serde_json::Value::String(value) => vec![value],
        serde_json::Value::Array(values) => values
            .into_iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    if stream_names.is_empty() {
        return Err(AppError::Validation(
            "stream parameter required".to_string(),
        ));
    }
    Ok(stream_names
        .into_iter()
        .map(|stream_name| StreamSubscription {
            local: stream_name == "hashtag:local",
            stream_name,
            tag: message.tag.clone(),
            list: message.list.clone(),
        })
        .collect())
}

fn subscription_key(subscription: &StreamSubscription) -> String {
    match subscription.stream_name.as_str() {
        "hashtag" | "hashtag:local" => format!(
            "{}:{}",
            subscription.stream_name,
            subscription.tag.clone().unwrap_or_default()
        ),
        "list" => format!("list:{}", subscription.list.clone().unwrap_or_default()),
        _ => subscription.stream_name.clone(),
    }
}

async fn multiplex_websocket_stream(
    state: StreamingApiState,
    socket: WebSocket,
    auth: Option<StreamAuth>,
) {
    let (mut sender, mut receiver) = socket.split();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<String>();
    let mut subscriptions: HashMap<String, JoinHandle<()>> = HashMap::new();

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let parsed: Result<WebSocketControlMessage, _> = serde_json::from_str(&text);
                        let Ok(message) = parsed else {
                            if sender.send(Message::Text("{\"error\":\"invalid_json\"}".to_string())).await.is_err() {
                                break;
                            }
                            continue;
                        };
                        let action = message.message_type.trim().to_ascii_lowercase();
                        let parsed_subscriptions = match parse_ws_subscriptions(message) {
                            Ok(value) => value,
                            Err(error) => {
                                if sender.send(Message::Text(serde_json::json!({"error": error.to_string()}).to_string())).await.is_err() {
                                    break;
                                }
                                continue;
                            }
                        };
                        match action.as_str() {
                            "subscribe" => {
                                for subscription in parsed_subscriptions {
                                    let key = subscription_key(&subscription);
                                    if subscriptions.contains_key(&key) {
                                        continue;
                                    }
                                    let subscribe_result = subscribe_stream(&state, auth.as_ref(), &subscription).await;
                                    let (event_receiver, stream_name) = match subscribe_result {
                                        Ok(value) => value,
                                        Err(error) => {
                                            if sender.send(Message::Text(serde_json::json!({"error": error.to_string()}).to_string())).await.is_err() {
                                                break;
                                            }
                                            continue;
                                        }
                                    };
                                    let tx = event_tx.clone();
                                    let state_for_task = state.clone();
                                    let viewer_scoped = auth.is_some()
                                        || matches!(stream_name.as_str(), "user" | "list" | "direct");
                                    let handle = tokio::spawn(async move {
                                        let mut event_receiver = event_receiver.into_inner();
                                        loop {
                                            match event_receiver.recv().await {
                                                Ok(event) => {
                                                    let payload = serialize_stream_event_data(
                                                        &state_for_task,
                                                        &event,
                                                        viewer_scoped,
                                                    )
                                                    .await;
                                                    let frame = serde_json::json!({
                                                        "stream": [stream_name],
                                                        "event": event.event_name(),
                                                        "payload": payload,
                                                    });
                                                    if tx.send(frame.to_string()).is_err() {
                                                        break;
                                                    }
                                                }
                                                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                                    tracing::warn!(skipped, "streaming receiver lagged; dropping old messages");
                                                }
                                                Err(broadcast::error::RecvError::Closed) => break,
                                            }
                                        }
                                    });
                                    subscriptions.insert(key, handle);
                                }
                            }
                            "unsubscribe" => {
                                for subscription in parsed_subscriptions {
                                    let key = subscription_key(&subscription);
                                    if let Some(handle) = subscriptions.remove(&key) {
                                        handle.abort();
                                    }
                                }
                            }
                            _ => {
                                if sender.send(Message::Text("{\"error\":\"unsupported_type\"}".to_string())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if sender.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            Some(frame) = event_rx.recv() => {
                if sender.send(Message::Text(frame)).await.is_err() {
                    break;
                }
            }
        }
    }

    for (_, handle) in subscriptions {
        handle.abort();
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
    Ok(build_sse_stream(state, receiver, true))
}

/// GET /api/v1/streaming/public
/// Stream public statuses
pub async fn stream_public(
    State(state): State<StreamingApiState>,
    headers: HeaderMap,
    jar: CookieJar,
    Query(_params): Query<StreamParams>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let receiver = state.streaming_event_bus.subscribe_public().await?;
    let auth = authenticate_stream_request(&state, &headers, &jar, None).await?;
    Ok(build_sse_stream(state, receiver, auth.is_some()))
}

/// GET /api/v1/streaming/public/local
/// Stream local public statuses
pub async fn stream_public_local(
    State(state): State<StreamingApiState>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let receiver = state.streaming_event_bus.subscribe_public_local().await?;
    let auth = authenticate_stream_request(&state, &headers, &jar, None).await?;
    Ok(build_sse_stream(state, receiver, auth.is_some()))
}

/// GET /api/v1/streaming/hashtag
/// Stream statuses with a specific hashtag
pub async fn stream_hashtag(
    State(state): State<StreamingApiState>,
    headers: HeaderMap,
    jar: CookieJar,
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
    let auth =
        authenticate_stream_request(&state, &headers, &jar, params.access_token.as_deref()).await?;
    Ok(build_sse_stream(state, receiver, auth.is_some()))
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
    Ok(build_sse_stream(state, receiver, true))
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
    Ok(build_sse_stream(state, receiver, true))
}

/// GET /api/v1/streaming
/// Mastodon-compatible streaming multiplexer using the `stream` query parameter.
pub async fn stream_root(
    State(state): State<StreamingApiState>,
    headers: HeaderMap,
    jar: CookieJar,
    ws: Option<WebSocketUpgrade>,
    Query(params): Query<StreamParams>,
) -> Result<impl IntoResponse, AppError> {
    let auth =
        authenticate_stream_request(&state, &headers, &jar, params.access_token.as_deref()).await?;
    if params.stream.is_none() {
        if let Some(ws) = ws {
            return Ok(ws
                .on_upgrade(move |socket| multiplex_websocket_stream(state, socket, auth))
                .into_response());
        }
        return Err(AppError::Validation(
            "stream parameter required for SSE connections".to_string(),
        ));
    }

    let subscription = StreamSubscription {
        stream_name: params.stream.clone().unwrap_or_default(),
        tag: params.tag.clone(),
        local: params.local.unwrap_or(false),
        list: params.list.clone(),
    };
    let (receiver, stream_name) = subscribe_stream(&state, auth.as_ref(), &subscription).await?;
    let viewer_scoped =
        auth.is_some() || matches!(stream_name.as_str(), "user" | "list" | "direct");
    if let Some(ws) = ws {
        return Ok(ws
            .on_upgrade(move |socket| {
                forward_websocket_stream(state, socket, receiver, stream_name, viewer_scoped)
            })
            .into_response());
    }
    Ok(build_sse_stream(state, receiver, viewer_scoped).into_response())
}
