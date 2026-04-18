//! Conversations endpoints (Direct Messages)

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, header::LINK},
    response::{IntoResponse, Json},
};
use serde::Deserialize;

use super::accounts::{
    build_remote_account_placeholder_response, resolve_account_response_for_identity,
};
use crate::{
    ConversationsApiState,
    auth::CurrentUser,
    error::AppError,
    service::{StreamEvent, StreamTarget},
};

#[derive(Debug, Deserialize)]
pub struct ConversationsParams {
    /// Maximum number of results to return (default 20)
    limit: Option<usize>,
    /// Return results older than this ID
    #[serde(rename = "max_id")]
    max_id: Option<String>,
    /// Return results newer than this ID
    #[serde(rename = "since_id")]
    since_id: Option<String>,
    /// Return results immediately newer than this ID
    #[serde(rename = "min_id")]
    min_id: Option<String>,
}

async fn build_conversation_response(
    state: &ConversationsApiState,
    account: &crate::data::Account,
    account_stats: crate::api::AccountStats,
    conversation_id: String,
    last_status_id: Option<String>,
    unread: bool,
) -> Result<serde_json::Value, AppError> {
    let local_address = format!("{}@{}", account.username, state.config.server.domain);
    let participant_addresses = state
        .db
        .get_conversation_participants(&conversation_id)
        .await?;

    let mut accounts = Vec::new();
    for address in participant_addresses
        .iter()
        .filter(|address| !address.eq_ignore_ascii_case(local_address.as_str()))
    {
        if let Some(account_response) = resolve_account_response_for_identity(
            state.config.as_ref(),
            state.db.as_ref(),
            state.profile_cache.as_ref(),
            None,
            address,
        )
        .await
        {
            accounts.push(serde_json::to_value(account_response).unwrap());
        } else if let Some(account_response) =
            build_remote_account_placeholder_response(address, state.config.as_ref(), 0)
        {
            accounts.push(serde_json::to_value(account_response).unwrap());
        }
    }

    if accounts.is_empty() && participant_addresses.len() <= 1 {
        accounts.push(
            serde_json::to_value(crate::api::account_to_response_with_stats(
                account,
                &state.config,
                account_stats,
            ))
            .unwrap(),
        );
    }

    let last_status = if let Some(status_id) = last_status_id {
        if let Some(status) = state.db.get_status(&status_id).await? {
            let remote_stats = if status.is_local {
                None
            } else {
                crate::api::load_remote_account_stats_map(
                    state.db.as_ref(),
                    state.profile_cache.as_ref(),
                    &state.config.server.protocol,
                    std::slice::from_ref(&status),
                )
                .await
                .ok()
                .and_then(|stats| stats.get(status.account_address.trim()).cloned())
            };
            Some(
                serde_json::to_value(
                    crate::api::build_status_response_with_account_stats_and_remote_stats(
                        state.db.as_ref(),
                        &status,
                        account,
                        &state.config,
                        account_stats,
                        remote_stats,
                        crate::api::StatusInteractions::new(
                            Some(state.db.is_favourited(&status.id).await?),
                            Some(state.db.is_reposted(&status.id).await?),
                            Some(
                                state
                                    .db
                                    .is_thread_muted(
                                        &state.db.resolve_thread_root_uri(&status).await?,
                                    )
                                    .await?,
                            ),
                            Some(state.db.is_bookmarked(&status.id).await?),
                            Some(state.db.is_status_pinned(&status.id).await?),
                        ),
                    )
                    .await?,
                )
                .unwrap(),
            )
        } else {
            None
        }
    } else {
        None
    };

    Ok(serde_json::json!({
        "id": conversation_id,
        "unread": unread,
        "accounts": accounts,
        "last_status": last_status,
    }))
}

fn conversation_link_header(
    limit: usize,
    first_id: Option<&str>,
    last_id: Option<&str>,
) -> Option<String> {
    let mut links = Vec::new();
    if let Some(last_id) = last_id.filter(|value| !value.is_empty()) {
        links.push(format!(
            "</api/v1/conversations?limit={limit}&max_id={}>; rel=\"next\"",
            urlencoding::encode(last_id)
        ));
    }
    if let Some(first_id) = first_id.filter(|value| !value.is_empty()) {
        links.push(format!(
            "</api/v1/conversations?limit={limit}&min_id={}>; rel=\"prev\"",
            urlencoding::encode(first_id)
        ));
    }
    (!links.is_empty()).then(|| links.join(", "))
}

async fn publish_conversation_event(
    state: &ConversationsApiState,
    payload: serde_json::Value,
) -> Result<(), AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    state
        .streaming_event_bus
        .publish(StreamEvent::Conversation {
            payload,
            targets: vec![
                StreamTarget::Direct {
                    account_id: account.username.clone(),
                },
                StreamTarget::User {
                    account_id: account.username,
                },
            ],
        })
        .await
}

/// GET /api/v1/conversations - Get conversations
///
/// View all conversations (direct message threads).
pub async fn get_conversations(
    State(state): State<ConversationsApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<ConversationsParams>,
) -> Result<impl IntoResponse, AppError> {
    let limit = params.limit.unwrap_or(20).min(40);
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let conversations = state
        .db
        .get_conversations(
            limit,
            params.max_id.as_deref(),
            params.min_id.as_deref().or(params.since_id.as_deref()),
        )
        .await?;

    let mut response = Vec::new();
    for (conversation_id, last_status_id, unread) in conversations {
        response.push(
            build_conversation_response(
                &state,
                &account,
                account_stats,
                conversation_id,
                last_status_id,
                unread,
            )
            .await?,
        );
    }

    let first_id = response
        .first()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let last_id = response
        .last()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let mut headers = HeaderMap::new();
    if let Some(link) = conversation_link_header(limit, first_id, last_id) {
        headers.insert(LINK, link.parse().expect("valid link header"));
    }

    Ok((headers, Json(serde_json::json!(response))))
}

/// DELETE /api/v1/conversations/:id - Remove a conversation
///
/// Remove a conversation from the list.
pub async fn delete_conversation(
    State(state): State<ConversationsApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = state.db.delete_conversation(&id).await?;

    if !deleted {
        return Err(AppError::NotFound);
    }

    publish_conversation_event(
        &state,
        serde_json::json!({
            "id": id,
            "_deleted": true,
        }),
    )
    .await?;

    Ok(Json(serde_json::json!({})))
}

/// POST /api/v1/conversations/:id/read - Mark as read
///
/// Mark a conversation as read.
pub async fn mark_conversation_read(
    State(state): State<ConversationsApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let updated = state.db.mark_conversation_read(&id).await?;

    if !updated {
        return Err(AppError::NotFound);
    }

    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    let (conversation_id, last_status_id, unread) = state
        .db
        .get_conversation(&id)
        .await?
        .ok_or(AppError::NotFound)?;
    let updated_conversation = build_conversation_response(
        &state,
        &account,
        account_stats,
        conversation_id,
        last_status_id,
        unread,
    )
    .await?;

    publish_conversation_event(&state, updated_conversation.clone()).await?;

    Ok(Json(updated_conversation))
}
