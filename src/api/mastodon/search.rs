//! Search endpoints

use axum::{
    extract::{Query, State},
    response::Json,
};
use serde::Deserialize;

use super::accounts::resolve_remote_account_response;
use crate::{AppState, auth::CurrentUser, error::AppError};

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    /// Search query
    q: String,
    /// Type of results to return (accounts, hashtags, statuses)
    #[serde(rename = "type")]
    search_type: Option<String>,
    /// Attempt WebFinger lookup
    #[serde(default)]
    resolve: bool,
    /// Only include accounts that the user is following
    #[serde(default)]
    following: bool,
    /// If provided, will only return statuses authored by this account
    account_id: Option<String>,
    /// Filter out unreviewed tags
    #[serde(default)]
    exclude_unreviewed: bool,
    /// Maximum number of results to return (default 40)
    limit: Option<usize>,
    /// Offset in search results
    offset: Option<usize>,
}

/// GET /api/v2/search - Search for content
///
/// Search for accounts, hashtags, and statuses.
pub async fn search_v2(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<SearchParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let query = params.q.trim();

    if query.is_empty() {
        return Ok(Json(serde_json::json!({
            "accounts": [],
            "statuses": [],
            "hashtags": []
        })));
    }

    let mut accounts = Vec::new();
    let mut statuses: Vec<serde_json::Value> = Vec::new();
    let mut hashtags = Vec::new();

    // Determine what to search based on type parameter
    let search_accounts =
        params.search_type.as_deref() == Some("accounts") || params.search_type.is_none();
    let search_statuses =
        params.search_type.as_deref() == Some("statuses") || params.search_type.is_none();
    let search_hashtags =
        params.search_type.as_deref() == Some("hashtags") || params.search_type.is_none();

    // Search accounts
    if search_accounts {
        // Check if query looks like an account address (contains @)
        if query.contains('@') {
            // For single-user instance, check if it's our account
            if let Ok(Some(account)) = state.db.get_account().await {
                let account_address =
                    format!("{}@{}", account.username, state.config.server.domain);
                if account_address
                    .to_lowercase()
                    .contains(&query.to_lowercase())
                {
                    accounts.push(crate::api::account_to_response(&account, &state.config));
                }
            }

            if params.resolve {
                if let Some(remote_account) = resolve_remote_account_response(&state, query).await {
                    let already_present = accounts
                        .iter()
                        .any(|account| account.id == remote_account.id);
                    if !already_present {
                        accounts.push(remote_account);
                    }
                }
            }
        } else {
            // Search by username
            if let Ok(Some(account)) = state.db.get_account().await {
                let display_name_matches = account
                    .display_name
                    .as_ref()
                    .map(|name| name.to_lowercase().contains(&query.to_lowercase()))
                    .unwrap_or(false);

                if account
                    .username
                    .to_lowercase()
                    .contains(&query.to_lowercase())
                    || display_name_matches
                {
                    accounts.push(crate::api::account_to_response(&account, &state.config));
                }
            }
        }
    }

    // Search statuses
    if search_statuses {
        let limit = params.limit.unwrap_or(20).min(40);
        let offset = params.offset.unwrap_or(0);

        match state.db.search_statuses(query, limit, offset).await {
            Ok(found_statuses) => {
                // Get account for status responses
                if let Ok(Some(account)) = state.db.get_account().await {
                    for status in found_statuses {
                        let status_response = crate::api::status_to_response(
                            &status,
                            &account,
                            &state.config,
                            Some(false), // favourited
                            Some(false), // reblogged
                            Some(false), // muted
                            Some(false), // bookmarked
                            Some(false), // pinned
                        );
                        statuses.push(serde_json::to_value(status_response).unwrap_or_default());
                    }
                }
            }
            Err(e) => {
                // Log error but don't fail the whole search
                eprintln!("Status search error: {}", e);
            }
        }
    }

    // Search hashtags
    if search_hashtags {
        // Extract hashtag from query
        let tag = query.trim_start_matches('#');
        if !tag.is_empty() {
            let limit = params.limit.unwrap_or(20).min(40);

            match state.db.search_hashtags(tag, limit).await {
                Ok(found_tags) => {
                    for (name, usage_count, _last_used) in found_tags {
                        hashtags.push(serde_json::json!({
                            "name": name,
                            "url": format!("https://{}/tags/{}", state.config.server.domain, name),
                            "history": [],
                            "following": false,
                            // Include usage stats for better UX
                            "uses": usage_count,
                        }));
                    }
                }
                Err(e) => {
                    // Log error but don't fail the whole search
                    eprintln!("Hashtag search error: {}", e);
                }
            }

            // If no results found, still return the searched tag if it looks valid
            if hashtags.is_empty() && tag.chars().all(|c| c.is_alphanumeric() || c == '_') {
                hashtags.push(serde_json::json!({
                    "name": tag,
                    "url": format!("https://{}/tags/{}", state.config.server.domain, tag),
                    "history": [],
                    "following": false,
                }));
            }
        }
    }

    Ok(Json(serde_json::json!({
        "accounts": accounts,
        "statuses": statuses,
        "hashtags": hashtags
    })))
}

/// GET /api/v1/search - Search for content (deprecated, v1)
///
/// Legacy search endpoint. Redirects to v2.
pub async fn search_v1(
    state: State<AppState>,
    user: CurrentUser,
    params: Query<SearchParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    search_v2(state, user, params).await
}
