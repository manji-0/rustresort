//! Polls endpoints

use axum::{
    body::to_bytes,
    extract::Request,
    extract::{Path, State},
    http::{HeaderMap, header::CONTENT_TYPE},
    response::Json,
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use std::collections::HashSet;

use crate::{
    PollsApiState,
    api::mastodon::federation_delivery::resolve_remote_actor_and_inbox_with_dependencies,
    auth::CurrentUser, error::AppError,
};

fn build_delivery(
    state: &PollsApiState,
    account: &crate::data::Account,
) -> crate::federation::ActivityDelivery {
    crate::federation::build_local_delivery(
        state.http_client.clone(),
        &state.config.server.base_url(),
        account,
    )
}

#[derive(Debug, Deserialize)]
pub struct VoteParams {
    /// Array of option indices to vote for
    choices: Vec<usize>,
}

fn parse_vote_params(headers: &HeaderMap, body: &[u8]) -> Result<VoteParams, AppError> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if content_type.starts_with("application/json") || content_type.is_empty() {
        return serde_json::from_slice(body)
            .map_err(|error| AppError::Validation(format!("invalid JSON body: {error}")));
    }

    if content_type.starts_with("application/x-www-form-urlencoded") {
        let mut choices = Vec::new();
        for (key, value) in url::form_urlencoded::parse(body).into_owned() {
            if matches!(key.as_str(), "choices[]" | "choices") {
                choices.push(value.parse::<usize>().map_err(|_| {
                    AppError::Validation("choices must be integer indices".to_string())
                })?);
            }
        }
        return Ok(VoteParams { choices });
    }

    Err(AppError::Validation(
        "unsupported content type for poll vote payload".to_string(),
    ))
}

async fn request_is_authenticated(
    state: &PollsApiState,
    headers: &HeaderMap,
    jar: &CookieJar,
) -> bool {
    if let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && state
            .db
            .get_oauth_token(token)
            .await
            .ok()
            .flatten()
            .is_some()
    {
        return true;
    }

    jar.get("session")
        .map(|cookie| cookie.value())
        .is_some_and(|token| {
            crate::auth::verify_session_token(token, &state.config.auth.session_secret).is_ok()
        })
}

async fn render_poll(
    state: &PollsApiState,
    id: &str,
    include_user_votes: bool,
) -> Result<Json<serde_json::Value>, AppError> {
    let poll = state.db.get_poll(id).await?.ok_or(AppError::NotFound)?;
    if let Some(status_id) = state.db.get_status_id_by_poll_id(id).await?
        && let Some(status) = state.db.get_status(&status_id).await?
        && !include_user_votes
        && !matches!(
            status.visibility,
            crate::data::StatusVisibility::Public | crate::data::StatusVisibility::Unlisted
        )
    {
        return Err(AppError::NotFound);
    }
    let options = state.db.get_poll_options(id).await?;

    let user_votes = if include_user_votes {
        let Some(account) = state.db.get_account().await? else {
            return Err(AppError::NotFound);
        };
        let account_address = format!("{}@{}", account.username, state.config.server.domain);
        state.db.get_user_poll_votes(id, &account_address).await?
    } else {
        Vec::new()
    };

    let own_votes: Vec<usize> = user_votes
        .iter()
        .filter_map(|vote_option_id| {
            options
                .iter()
                .position(|(option_id, _, _)| option_id == vote_option_id)
        })
        .collect();

    let options_response: Vec<serde_json::Value> = options
        .into_iter()
        .map(|(_, title, votes_count)| {
            serde_json::json!({
                "title": title,
                "votes_count": if poll.4 && !poll.2 && own_votes.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::json!(votes_count)
                }
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "id": poll.0,
        "expires_at": poll.1,
        "expired": poll.2,
        "multiple": poll.3,
        "hide_totals": poll.4,
        "votes_count": if poll.4 && !poll.2 && own_votes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::json!(poll.5)
        },
        "voters_count": if poll.4 && !poll.2 && own_votes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::json!(poll.6)
        },
        "voted": !own_votes.is_empty(),
        "own_votes": own_votes,
        "options": options_response,
        "emojis": []
    })))
}

/// GET /api/v1/polls/:id - Get a poll
///
/// View a poll attached to a status.
pub async fn get_poll(
    State(state): State<PollsApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<serde_json::Value>, AppError> {
    render_poll(
        &state,
        &id,
        request_is_authenticated(&state, &headers, &jar).await,
    )
    .await
}

/// POST /api/v1/polls/:id/votes - Vote in a poll
///
/// Vote on a poll attached to a status.
pub async fn vote_in_poll(
    State(state): State<PollsApiState>,
    CurrentUser(session): CurrentUser,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<serde_json::Value>, AppError> {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 64 * 1024)
        .await
        .map_err(|error| AppError::Validation(format!("failed to read request body: {error}")))?;
    let params = parse_vote_params(&parts.headers, &body)?;

    // Validate choices
    if params.choices.is_empty() {
        return Err(AppError::Validation(
            "At least one choice is required".to_string(),
        ));
    }
    let mut seen_choice_indices = HashSet::new();
    for choice in &params.choices {
        if !seen_choice_indices.insert(*choice) {
            return Err(AppError::Validation(
                "Duplicate choices are not allowed".to_string(),
            ));
        }
    }

    // Get poll to validate
    let poll = state.db.get_poll(&id).await?.ok_or(AppError::NotFound)?;

    // Check if poll is expired
    if poll.2 {
        return Err(AppError::Validation("Poll has expired".to_string()));
    }

    // Get poll options to convert indices to IDs
    let options = state.db.get_poll_options(&id).await?;

    // Validate choice indices and convert to option IDs
    let mut option_ids = Vec::new();
    for choice_index in &params.choices {
        if *choice_index >= options.len() {
            return Err(AppError::Validation(format!(
                "Invalid choice index: {}",
                choice_index
            )));
        }
        option_ids.push(options[*choice_index].0.clone());
    }

    let selected_titles = params
        .choices
        .iter()
        .map(|choice_index| options[*choice_index].1.clone())
        .collect::<Vec<_>>();

    // Get user's account address
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_address = format!("{}@{}", account.username, state.config.server.domain);

    if let Some(status_id) = state.db.get_status_id_by_poll_id(&id).await?
        && let Some(status) = state.db.get_status(&status_id).await?
        && !status.is_local
        && !status.account_address.is_empty()
    {
        let poll_uri = status.uri.clone();
        let remote_account_address = status.account_address.clone();
        let (target_actor_uri, target_inbox_uri) =
            resolve_remote_actor_and_inbox_with_dependencies(
                state.db.as_ref(),
                state.profile_cache.as_ref(),
                state.federation_fetch_client.as_ref(),
                &remote_account_address,
            )
            .await?;
        let delivery = build_delivery(&state, &account);
        delivery
            .send_poll_vote(
                &poll_uri,
                &selected_titles,
                &target_actor_uri,
                &target_inbox_uri,
            )
            .await?;
    }

    // Record vote after remote delivery succeeds, or immediately for local polls.
    state
        .db
        .vote_in_poll(&id, &account_address, &option_ids)
        .await?;

    // Return updated poll
    let _ = session;
    render_poll(&state, &id, true).await
}
