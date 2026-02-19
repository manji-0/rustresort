//! Polls endpoints

use axum::{
    extract::{Path, State},
    response::Json,
};
use serde::Deserialize;
use std::collections::HashSet;

use crate::{AppState, auth::CurrentUser, error::AppError};

#[derive(Debug, Deserialize)]
pub struct VoteParams {
    /// Array of option indices to vote for
    choices: Vec<usize>,
}

/// GET /api/v1/polls/:id - Get a poll
///
/// View a poll attached to a status.
pub async fn get_poll(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get poll details
    let poll = state.db.get_poll(&id).await?.ok_or(AppError::NotFound)?;

    // Get poll options
    let options = state.db.get_poll_options(&id).await?;

    // Get user's votes if authenticated
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_address = format!("{}@{}", account.username, state.config.server.domain);
    let user_votes = state.db.get_user_poll_votes(&id, &account_address).await?;

    // Convert option IDs to indices
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
                "votes_count": votes_count
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "id": poll.0,
        "expires_at": poll.1,
        "expired": poll.2,
        "multiple": poll.3,
        "votes_count": poll.4,
        "voters_count": poll.5,
        "voted": !own_votes.is_empty(),
        "own_votes": own_votes,
        "options": options_response,
        "emojis": []
    })))
}

/// POST /api/v1/polls/:id/votes - Vote in a poll
///
/// Vote on a poll attached to a status.
pub async fn vote_in_poll(
    State(state): State<AppState>,
    CurrentUser(session): CurrentUser,
    Path(id): Path<String>,
    Json(params): Json<VoteParams>,
) -> Result<Json<serde_json::Value>, AppError> {
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

    // Get user's account address
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_address = format!("{}@{}", account.username, state.config.server.domain);

    // Record vote
    state
        .db
        .vote_in_poll(&id, &account_address, &option_ids)
        .await?;

    // Return updated poll
    get_poll(State(state), CurrentUser(session), Path(id)).await
}

// Helper function to create poll response (for future use)
#[allow(dead_code)]
fn poll_to_response(
    poll_id: &str,
    options: Vec<String>,
    votes_count: Vec<i64>,
    voters_count: i64,
    expires_at: Option<String>,
    expired: bool,
    multiple: bool,
    voted: bool,
    own_votes: Vec<usize>,
) -> serde_json::Value {
    let total_votes: i64 = votes_count.iter().sum();

    let options_response: Vec<serde_json::Value> = options
        .iter()
        .zip(votes_count.iter())
        .map(|(title, votes)| {
            serde_json::json!({
                "title": title,
                "votes_count": votes
            })
        })
        .collect();

    serde_json::json!({
        "id": poll_id,
        "expires_at": expires_at,
        "expired": expired,
        "multiple": multiple,
        "votes_count": total_votes,
        "voters_count": voters_count,
        "voted": voted,
        "own_votes": own_votes,
        "options": options_response,
        "emojis": []
    })
}
