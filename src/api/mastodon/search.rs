//! Search endpoints

use axum::{
    extract::{Query, State},
    response::Json,
};
use serde::Deserialize;

use super::accounts::resolve_remote_account_response;
use crate::{SearchApiState, auth::CurrentUser, error::AppError};

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
    #[serde(rename = "following")]
    following: bool,
    /// If provided, will only return statuses authored by this account
    #[serde(rename = "account_id")]
    account_id: Option<String>,
    /// Filter out unreviewed tags
    #[serde(default)]
    #[serde(rename = "exclude_unreviewed")]
    _exclude_unreviewed: bool,
    /// Maximum number of results to return (default 40)
    limit: Option<usize>,
    /// Offset in search results
    offset: Option<usize>,
}

fn canonical_account_identity(acct: &str, local_domain: &str) -> String {
    let normalized_acct = acct.trim().trim_start_matches('@').to_ascii_lowercase();
    if normalized_acct.contains('@') {
        normalized_acct
    } else {
        format!(
            "{normalized_acct}@{}",
            local_domain.trim().to_ascii_lowercase()
        )
    }
}

/// GET /api/v2/search - Search for content
///
/// Search for accounts, hashtags, and statuses.
pub async fn search_v2(
    State(state): State<SearchApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<SearchParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let query = params.q.trim();
    let local_domain = state.config.server.domain.to_ascii_lowercase();

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
    let account_limit = params.limit.unwrap_or(40).min(80);
    let following_identities = if params.following {
        state
            .db
            .get_all_follow_addresses()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|address| canonical_account_identity(&address, &local_domain))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

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
            let query_identity = canonical_account_identity(query, &local_domain);
            let mut local_account_identity = None;
            let mut matched_local_account = false;

            // For single-user instance, check if it's our account
            if let Ok(Some(account)) = state.db.get_account().await {
                let account_stats = crate::api::load_local_account_stats(state.db.as_ref())
                    .await
                    .unwrap_or_default();
                let account_address =
                    format!("{}@{}", account.username, state.config.server.domain);
                local_account_identity =
                    Some(canonical_account_identity(&account_address, &local_domain));
                if account_address
                    .to_lowercase()
                    .contains(&query.to_lowercase())
                    || local_account_identity.as_deref() == Some(query_identity.as_str())
                {
                    accounts.push(crate::api::account_to_response_with_stats(
                        &account,
                        &state.config,
                        account_stats,
                    ));
                    matched_local_account = true;
                }
            }

            if params.resolve {
                let should_skip_resolve = matched_local_account
                    && local_account_identity.as_deref() == Some(query_identity.as_str());
                if !should_skip_resolve
                    && let Some(remote_account) = resolve_remote_account_response(
                        state.config.as_ref(),
                        state.db.as_ref(),
                        state.profile_cache.as_ref(),
                        state.federation_fetch_client.as_ref(),
                        query,
                    )
                    .await
                {
                    let remote_identity =
                        canonical_account_identity(&remote_account.acct, &local_domain);
                    let already_present = accounts.iter().any(|account| {
                        canonical_account_identity(&account.acct, &local_domain) == remote_identity
                    });
                    if !already_present {
                        accounts.push(remote_account);
                    }
                }
            }
        } else {
            // Search by username
            if let Ok(Some(account)) = state.db.get_account().await {
                let account_stats = crate::api::load_local_account_stats(state.db.as_ref())
                    .await
                    .unwrap_or_default();
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
                    accounts.push(crate::api::account_to_response_with_stats(
                        &account,
                        &state.config,
                        account_stats,
                    ));
                }
            }
        }
        if params.following {
            accounts.retain(|account| {
                let identity = canonical_account_identity(&account.acct, &local_domain);
                following_identities
                    .iter()
                    .any(|candidate| candidate == &identity)
            });
        }
        accounts.truncate(account_limit);
    }

    // Search statuses
    if search_statuses {
        let limit = params.limit.unwrap_or(20).min(40);
        let offset = params.offset.unwrap_or(0);

        match state.db.search_statuses(query, limit, offset).await {
            Ok(found_statuses) => {
                // Get account for status responses
                if let Ok(Some(account)) = state.db.get_account().await {
                    let filtered_statuses = if let Some(account_id) = params.account_id.as_deref() {
                        if account.id == account_id {
                            found_statuses
                                .into_iter()
                                .filter(|status| status.is_local)
                                .collect::<Vec<_>>()
                        } else {
                            Vec::new()
                        }
                    } else {
                        found_statuses
                    };
                    let account_stats = crate::api::load_local_account_stats(state.db.as_ref())
                        .await
                        .unwrap_or_default();
                    let remote_account_stats = crate::api::load_remote_account_stats_map(
                        state.db.as_ref(),
                        state.profile_cache.as_ref(),
                        &state.config.server.protocol,
                        &filtered_statuses,
                    )
                    .await
                    .unwrap_or_default();
                    for status in filtered_statuses {
                        let remote_stats = remote_account_stats
                            .get(status.account_address.trim())
                            .copied();
                        let status_response =
                            crate::api::build_status_response_with_account_stats_and_remote_stats(
                                state.db.as_ref(),
                                &status,
                                &account,
                                &state.config,
                                account_stats,
                                remote_stats,
                                crate::api::StatusInteractions::new(
                                    Some(false),
                                    Some(false),
                                    Some(false),
                                    Some(false),
                                    Some(false),
                                ),
                            )
                            .await
                            .unwrap_or_else(|_| {
                                crate::api::status_to_response_with_account_stats_and_remote_stats(
                                    &status,
                                    &account,
                                    &state.config,
                                    account_stats,
                                    remote_stats,
                                    crate::api::StatusInteractions::new(
                                        Some(false),
                                        Some(false),
                                        Some(false),
                                        Some(false),
                                        Some(false),
                                    ),
                                )
                            });
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
    state: State<SearchApiState>,
    user: CurrentUser,
    params: Query<SearchParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    search_v2(state, user, params).await
}
