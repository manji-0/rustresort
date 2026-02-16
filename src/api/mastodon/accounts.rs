//! Account endpoints

use axum::{
    extract::{Path, Query, RawQuery, State},
    response::Json,
};
use serde::Deserialize;

use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::metrics::{
    DB_QUERIES_TOTAL, DB_QUERY_DURATION_SECONDS, FOLLOWERS_TOTAL, FOLLOWING_TOTAL,
    HTTP_REQUEST_DURATION_SECONDS, HTTP_REQUESTS_TOTAL,
};

/// Pagination parameters
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct PaginationParams {
    pub max_id: Option<String>,
    pub min_id: Option<String>,
    pub limit: Option<usize>,
}

/// Update credentials request
#[derive(Debug, Deserialize)]
pub struct UpdateCredentialsRequest {
    pub display_name: Option<String>,
    pub note: Option<String>,
    pub avatar: Option<String>, // Base64 encoded image
    pub header: Option<String>, // Base64 encoded image
    pub locked: Option<bool>,
    pub bot: Option<bool>,
    pub discoverable: Option<bool>,
}

/// Search query parameters
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: String,
    pub limit: Option<usize>,
    pub resolve: Option<bool>,
    pub following: Option<bool>,
}

fn default_port_for_protocol(protocol: &str) -> Option<u16> {
    if protocol.eq_ignore_ascii_case("http") {
        Some(80)
    } else if protocol.eq_ignore_ascii_case("https") {
        Some(443)
    } else {
        None
    }
}

fn extract_explicit_port(authority: &str) -> Option<u16> {
    let authority = authority.trim();

    if let Some(rest) = authority.strip_prefix('[') {
        let (_, tail) = rest.split_once(']')?;
        let port_str = tail.strip_prefix(':')?;
        if port_str.is_empty() || !port_str.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        return port_str.parse::<u16>().ok();
    }

    let (host_part, port_str) = authority.rsplit_once(':')?;
    if host_part.is_empty()
        || host_part.contains(':')
        || port_str.is_empty()
        || !port_str.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }

    port_str.parse::<u16>().ok()
}

fn parse_host_and_port(authority: &str) -> Result<(String, Option<u16>), AppError> {
    let parsed = url::Url::parse(&format!("http://{}", authority))
        .map_err(|_| AppError::Validation("Invalid account ID format".to_string()))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::Validation("Invalid account ID format".to_string()))?;
    let normalized_host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();

    Ok((normalized_host, extract_explicit_port(authority)))
}

fn format_authority_host(host: &str) -> String {
    let bare_host = host.trim_start_matches('[').trim_end_matches(']');
    if bare_host.contains(':') {
        format!("[{}]", bare_host)
    } else {
        bare_host.to_string()
    }
}

fn is_same_local_account(target_address: &str, local_address: &str, local_protocol: &str) -> bool {
    let Some((target_user, target_domain)) = target_address.split_once('@') else {
        return false;
    };
    let Some((local_user, local_domain)) = local_address.split_once('@') else {
        return false;
    };

    if !target_user.eq_ignore_ascii_case(local_user) {
        return false;
    }

    let Ok((target_host, target_port)) = parse_host_and_port(target_domain) else {
        return false;
    };
    let Ok((local_host, local_port)) = parse_host_and_port(local_domain) else {
        return false;
    };
    if !target_host.eq_ignore_ascii_case(&local_host) {
        return false;
    }

    let Some(default_port) = default_port_for_protocol(local_protocol) else {
        return target_port == local_port;
    };
    let target_effective_port = target_port.unwrap_or(default_port);
    let local_effective_port = local_port.unwrap_or(default_port);
    target_effective_port == local_effective_port
}

fn normalize_account_address(raw: &str) -> Result<String, AppError> {
    fn normalize_domain(raw: &str) -> Result<String, AppError> {
        let parsed = url::Url::parse(&format!("https://{}", raw))
            .map_err(|_| AppError::Validation("Invalid account ID format".to_string()))?;
        if parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(AppError::Validation(
                "Invalid account ID format".to_string(),
            ));
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| AppError::Validation("Invalid account ID format".to_string()))?;
        let normalized_host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        let authority_host = format_authority_host(&normalized_host);
        let normalized_port = extract_explicit_port(raw);

        Ok(match normalized_port {
            Some(port) => format!("{}:{}", authority_host, port),
            None => authority_host,
        })
    }

    let trimmed = raw.trim();
    let without_leading_at = trimmed.strip_prefix('@').unwrap_or(trimmed);
    let (username, domain) = without_leading_at
        .split_once('@')
        .ok_or_else(|| AppError::Validation("Invalid account ID format".to_string()))?;

    if username.is_empty() || domain.is_empty() || username.contains('@') || domain.contains('@') {
        return Err(AppError::Validation(
            "Invalid account ID format".to_string(),
        ));
    }

    Ok(format!(
        "{}@{}",
        username.to_ascii_lowercase(),
        normalize_domain(domain)?
    ))
}

async fn resolve_target_address(state: &AppState, id: &str) -> Result<String, AppError> {
    if id.starts_with("http://") || id.starts_with("https://") {
        return Err(AppError::Validation(
            "Account URI is not yet supported".to_string(),
        ));
    }

    if id.contains('@') {
        return normalize_account_address(id);
    }

    if let Some(account) = state.db.get_account().await? {
        if account.id == id {
            return normalize_account_address(&format!(
                "{}@{}",
                account.username, state.config.server.domain
            ));
        }
    }

    Err(AppError::Validation(
        "Invalid account ID format".to_string(),
    ))
}

/// GET /api/v1/accounts/verify_credentials
pub async fn verify_credentials(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["GET", "/api/v1/accounts/verify_credentials"])
        .start_timer();

    // Get the account from database
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "accounts"])
        .start_timer();
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "accounts"])
        .inc();
    db_timer.observe_duration();

    // Convert to API response
    let mut response = crate::api::account_to_response(&account, &state.config);

    // Get counts
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "followers"])
        .start_timer();
    let followers_count = state.db.get_all_follower_addresses().await?.len() as i32;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "followers"])
        .inc();
    db_timer.observe_duration();

    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "follows"])
        .start_timer();
    let following_count = state.db.get_all_follow_addresses().await?.len() as i32;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "follows"])
        .inc();
    db_timer.observe_duration();

    response.followers_count = followers_count;
    response.following_count = following_count;

    // Update metrics
    FOLLOWERS_TOTAL.set(followers_count as i64);
    FOLLOWING_TOTAL.set(following_count as i64);

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/v1/accounts/verify_credentials", "200"])
        .inc();

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// PATCH /api/v1/accounts/update_credentials
pub async fn update_credentials(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Json(req): Json<UpdateCredentialsRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    use chrono::Utc;

    // Get current account
    let mut account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Update fields if provided
    if let Some(display_name) = req.display_name {
        account.display_name = Some(display_name);
    }

    if let Some(note) = req.note {
        account.note = Some(note);
    }

    // TODO: Handle avatar and header uploads
    // For now, we skip image processing as it requires multipart/form-data handling
    // and S3 upload integration

    account.updated_at = Utc::now();

    // Save to database
    state.db.upsert_account(&account).await?;

    // Return updated account
    let mut response = crate::api::account_to_response(&account, &state.config);

    // Get counts
    let followers_count = state.db.get_all_follower_addresses().await?.len() as i32;
    let following_count = state.db.get_all_follow_addresses().await?.len() as i32;

    response.followers_count = followers_count;
    response.following_count = following_count;

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/accounts/:id
pub async fn get_account(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get the account from database
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Check if ID matches
    if account.id != id {
        return Err(AppError::NotFound);
    }

    // Convert to API response
    let mut response = crate::api::account_to_response(&account, &state.config);

    // Get counts
    let followers_count = state.db.get_all_follower_addresses().await?.len() as i32;
    let following_count = state.db.get_all_follow_addresses().await?.len() as i32;

    response.followers_count = followers_count;
    response.following_count = following_count;

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/accounts/:id/statuses
pub async fn account_statuses(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get the account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    if account.id != id {
        return Err(AppError::NotFound);
    }

    // Get local statuses
    let limit = params.limit.unwrap_or(20).min(40);
    let statuses = state
        .db
        .get_local_statuses(limit, params.max_id.as_deref())
        .await?;

    // Convert to API responses
    let responses: Vec<_> = statuses
        .iter()
        .map(|status| {
            let response =
                crate::api::status_to_response(status, &account, &state.config, None, None, None);
            serde_json::to_value(response).unwrap()
        })
        .collect();

    Ok(Json(responses))
}

/// GET /api/v1/accounts/:id/followers
pub async fn get_account_followers(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(_params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get the account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    if account.id != id {
        return Err(AppError::NotFound);
    }

    // Get follower addresses
    let _follower_addresses = state.db.get_all_follower_addresses().await?;

    // TODO: Fetch full account info for each follower from cache/federation
    // For now, return empty array as we don't have remote account info
    Ok(Json(vec![]))
}

/// GET /api/v1/accounts/:id/following
pub async fn get_account_following(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(_params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get the account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    if account.id != id {
        return Err(AppError::NotFound);
    }

    // Get following addresses
    let _following_addresses = state.db.get_all_follow_addresses().await?;

    // TODO: Fetch full account info for each followed account from cache/federation
    // For now, return empty array as we don't have remote account info
    Ok(Json(vec![]))
}

/// POST /api/v1/accounts/:id/follow
pub async fn follow_account(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::RelationshipResponse;
    use crate::data::{EntityId, Follow};
    use chrono::Utc;

    // Accept account addresses and local account IDs.
    let target_address = resolve_target_address(&state, &id).await?;

    // Get our account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let local_address = normalize_account_address(&format!(
        "{}@{}",
        account.username, state.config.server.domain
    ))?;

    if is_same_local_account(
        &target_address,
        &local_address,
        &state.config.server.protocol,
    ) {
        return Err(AppError::Validation("cannot follow yourself".to_string()));
    }

    // Persist follow relationship if not already present.
    let follow_id = EntityId::new().0;
    let follow = Follow {
        id: follow_id.clone(),
        target_address: target_address.clone(),
        uri: format!(
            "{}/users/{}/follow/{}",
            state.config.server.base_url(),
            account.username,
            follow_id
        ),
        created_at: Utc::now(),
    };
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    state
        .db
        .insert_follow_if_absent(&follow, default_port)
        .await?;

    // TODO: Actually send Follow activity via federation delivery.
    // state.federation.send_follow(&account, &target_address).await?;

    // Return relationship response
    let relationship = RelationshipResponse {
        id: id.clone(),
        following: true, // We just followed
        followed_by: false,
        blocking: false,
        blocked_by: false,
        muting: false,
        muting_notifications: false,
        requested: false,
        domain_blocking: false,
        showing_reblogs: true,
        endorsed: false,
        notifying: false,
        note: String::new(),
    };

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

#[cfg(test)]
mod tests {
    use super::normalize_account_address;

    #[test]
    fn normalize_account_address_preserves_ipv6_brackets_without_port() {
        let normalized = normalize_account_address("Alice@[2001:DB8::1]").unwrap();
        assert_eq!(normalized, "alice@[2001:db8::1]");
    }

    #[test]
    fn normalize_account_address_preserves_ipv6_brackets_with_port() {
        let normalized = normalize_account_address("Alice@[2001:DB8::1]:443").unwrap();
        assert_eq!(normalized, "alice@[2001:db8::1]:443");
    }
}

/// POST /api/v1/accounts/:id/unfollow
pub async fn unfollow_account(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::RelationshipResponse;

    // Accept account addresses and local account IDs.
    let target_address = resolve_target_address(&state, &id).await?;

    // Get our account
    let _account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Remove follow relationship from DB.
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    state
        .db
        .delete_follow(&target_address, default_port)
        .await?;

    // Send Undo Follow activity via ActivityPub
    // TODO: Actually send Undo activity and remove the follow relationship
    // state.federation.send_unfollow(&account, &target_address).await?;

    // Return relationship response
    let relationship = RelationshipResponse {
        id: id.clone(),
        following: false, // We just unfollowed
        followed_by: false,
        blocking: false,
        blocked_by: false,
        muting: false,
        muting_notifications: false,
        requested: false,
        domain_blocking: false,
        showing_reblogs: true,
        endorsed: false,
        notifying: false,
        note: String::new(),
    };

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

/// GET /api/v1/accounts/relationships
pub async fn get_relationships(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    use crate::api::dto::RelationshipResponse;

    // Get following and follower addresses
    let _following_addresses = state.db.get_all_follow_addresses().await?;
    let _follower_addresses = state.db.get_all_follower_addresses().await?;

    let ids: Vec<String> = raw_query
        .as_deref()
        .map(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .filter_map(|(key, value)| {
                    if key == "id[]" || key == "id" {
                        Some(value.into_owned())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Create relationship responses for each requested ID
    let mut relationships = vec![];
    for id in ids {
        // For single-user instance, we check if the ID matches our account
        let _account = state.db.get_account().await?;

        let relationship = RelationshipResponse {
            id: id.clone(),
            following: false,   // TODO: Check if we follow this account
            followed_by: false, // TODO: Check if this account follows us
            blocking: false,
            blocked_by: false,
            muting: false,
            muting_notifications: false,
            requested: false,
            domain_blocking: false,
            showing_reblogs: true,
            endorsed: false,
            notifying: false,
            note: String::new(),
        };

        relationships.push(serde_json::to_value(relationship).unwrap());
    }

    Ok(Json(relationships))
}

/// GET /api/v1/accounts/search
pub async fn search_accounts(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // For single-user instance, we can only search for:
    // 1. Our own account (by username)
    // 2. Remote accounts (by address like user@domain.com)

    let query = params.q.trim().to_lowercase();
    let mut results = vec![];

    // Get our account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Check if query matches our username
    if account.username.to_lowercase().contains(&query)
        || account
            .display_name
            .as_ref()
            .map(|d| d.to_lowercase().contains(&query))
            .unwrap_or(false)
    {
        let account_response = crate::api::account_to_response(&account, &state.config);
        results.push(serde_json::to_value(account_response).unwrap());
    }

    // If resolve=true and query looks like an account address, try WebFinger
    if params.resolve.unwrap_or(false) && query.contains('@') {
        // TODO: Implement WebFinger lookup for remote accounts
        // This would:
        // 1. Parse the account address
        // 2. Perform WebFinger lookup
        // 3. Fetch the actor profile
        // 4. Convert to AccountResponse
        // 5. Add to results
    }

    // Apply limit
    let limit = params.limit.unwrap_or(40).min(80);
    results.truncate(limit);

    Ok(Json(results))
}

/// Create account request
#[derive(Debug, Deserialize)]
pub struct CreateAccountRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    pub agreement: Option<bool>,
    pub locale: Option<String>,
}

/// POST /api/v1/accounts
/// Create a new account
///
/// For single-user instance, this endpoint returns an error
/// as account creation is not supported.
pub async fn create_account(
    State(_state): State<AppState>,
    Json(_req): Json<CreateAccountRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Single-user instance doesn't support account creation via API
    Err(AppError::Unprocessable(
        "Account creation is not supported on this single-user instance".to_string(),
    ))
}

/// GET /api/v1/accounts/:id/lists
/// Get lists that contain the specified account
///
/// For single-user instance, this returns an empty array
/// as list functionality is not yet implemented.
pub async fn get_account_lists(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Verify account exists
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    if account.id != id {
        return Err(AppError::NotFound);
    }

    // Lists not yet implemented, return empty array
    Ok(Json(vec![]))
}

/// GET /api/v1/accounts/:id/identity_proofs
/// Get identity proofs for the specified account
///
/// Identity proofs (e.g., Keybase) are not supported,
/// so this always returns an empty array.
pub async fn get_account_identity_proofs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Verify account exists
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    if account.id != id {
        return Err(AppError::NotFound);
    }

    // Identity proofs not supported, return empty array
    Ok(Json(vec![]))
}

/// Mute account request
#[derive(Debug, Deserialize)]
pub struct MuteAccountRequest {
    pub notifications: Option<bool>,
    pub duration: Option<i64>, // Duration in seconds, 0 = indefinite
}

/// POST /api/v1/accounts/:id/block
/// Block an account
pub async fn block_account(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::RelationshipResponse;

    // Accept account addresses and local account IDs.
    let target_address = resolve_target_address(&state, &id).await?;

    // Store block in database
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    state
        .db
        .block_account(&target_address, default_port)
        .await?;

    // Return relationship response
    let relationship = RelationshipResponse {
        id: id.clone(),
        following: false,
        followed_by: false,
        blocking: true, // Now blocking
        blocked_by: false,
        muting: false,
        muting_notifications: false,
        requested: false,
        domain_blocking: false,
        showing_reblogs: false,
        endorsed: false,
        notifying: false,
        note: String::new(),
    };

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

/// POST /api/v1/accounts/:id/unblock
/// Unblock an account
pub async fn unblock_account(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::RelationshipResponse;

    // Accept account addresses and local account IDs.
    let target_address = resolve_target_address(&state, &id).await?;

    // Remove block from database
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    state
        .db
        .unblock_account(&target_address, default_port)
        .await?;

    // Return relationship response
    let relationship = RelationshipResponse {
        id: id.clone(),
        following: false,
        followed_by: false,
        blocking: false, // No longer blocking
        blocked_by: false,
        muting: false,
        muting_notifications: false,
        requested: false,
        domain_blocking: false,
        showing_reblogs: true,
        endorsed: false,
        notifying: false,
        note: String::new(),
    };

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

/// POST /api/v1/accounts/:id/mute
/// Mute an account
pub async fn mute_account(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    req: Option<Json<MuteAccountRequest>>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::RelationshipResponse;

    // Accept account addresses and local account IDs.
    let target_address = resolve_target_address(&state, &id).await?;

    let req = req
        .map(|Json(payload)| payload)
        .unwrap_or(MuteAccountRequest {
            notifications: None,
            duration: None,
        });

    let mute_notifications = req.notifications.unwrap_or(true);
    let duration = req.duration;

    // Store mute in database
    state
        .db
        .mute_account(
            &target_address,
            mute_notifications,
            duration,
            default_port_for_protocol(&state.config.server.protocol),
        )
        .await?;

    // Return relationship response
    let relationship = RelationshipResponse {
        id: id.clone(),
        following: false,
        followed_by: false,
        blocking: false,
        blocked_by: false,
        muting: true, // Now muting
        muting_notifications: mute_notifications,
        requested: false,
        domain_blocking: false,
        showing_reblogs: true,
        endorsed: false,
        notifying: false,
        note: String::new(),
    };

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

/// POST /api/v1/accounts/:id/unmute
/// Unmute an account
pub async fn unmute_account(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::RelationshipResponse;

    // Accept account addresses and local account IDs.
    let target_address = resolve_target_address(&state, &id).await?;

    // Remove mute from database
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    state
        .db
        .unmute_account(&target_address, default_port)
        .await?;

    // Return relationship response
    let relationship = RelationshipResponse {
        id: id.clone(),
        following: false,
        followed_by: false,
        blocking: false,
        blocked_by: false,
        muting: false, // No longer muting
        muting_notifications: false,
        requested: false,
        domain_blocking: false,
        showing_reblogs: true,
        endorsed: false,
        notifying: false,
        note: String::new(),
    };

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

/// GET /api/v1/blocks
/// Get list of blocked accounts
pub async fn get_blocks(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get blocked account addresses from database
    let limit = params.limit.unwrap_or(40).min(80);

    let _addresses = state.db.get_blocked_accounts(limit).await?;

    // For now, return empty array as we don't have remote account info
    // TODO: Fetch full account info for each blocked account
    Ok(Json(vec![]))
}

/// GET /api/v1/mutes
/// Get list of muted accounts
pub async fn get_mutes(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get muted account addresses from database
    let limit = params.limit.unwrap_or(40).min(80);

    let _addresses = state.db.get_muted_accounts(limit).await?;

    // For now, return empty array as we don't have remote account info
    // TODO: Fetch full account info for each muted account
    Ok(Json(vec![]))
}

/// GET /api/v1/follow_requests
/// Get list of pending follow requests
pub async fn get_follow_requests(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get follow requests from database
    let limit = params.limit.unwrap_or(40).min(80);

    let _addresses = state.db.get_follow_request_addresses(limit).await?;

    // For now, return empty array as we don't have remote account info
    // TODO: Fetch full account info for each requester
    Ok(Json(vec![]))
}

/// GET /api/v1/follow_requests/:id
/// Get a specific follow request
pub async fn get_follow_request(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let requester_address = id.clone();

    // Check if follow request exists
    if !state.db.has_follow_request(&requester_address).await? {
        return Err(AppError::NotFound);
    }

    // TODO: Fetch full account info for the requester
    // For now, return a minimal account object
    Ok(Json(serde_json::json!({
        "id": id,
        "username": requester_address.split('@').next().unwrap_or(&requester_address),
        "acct": requester_address,
    })))
}

/// POST /api/v1/follow_requests/:id/authorize
/// Accept a follow request
pub async fn authorize_follow_request(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::RelationshipResponse;

    let requester_address = id.clone();

    // Accept the follow request (moves to followers table)
    state.db.accept_follow_request(&requester_address).await?;

    // TODO: Send Accept activity via ActivityPub

    // Return relationship response
    let relationship = RelationshipResponse {
        id: id.clone(),
        following: false,
        followed_by: true, // Now following us
        blocking: false,
        blocked_by: false,
        muting: false,
        muting_notifications: false,
        requested: false,
        domain_blocking: false,
        showing_reblogs: true,
        endorsed: false,
        notifying: false,
        note: String::new(),
    };

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

/// POST /api/v1/follow_requests/:id/reject
/// Reject a follow request
pub async fn reject_follow_request(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::RelationshipResponse;

    let requester_address = id.clone();

    // Remove from follow_requests
    if !state.db.reject_follow_request(&requester_address).await? {
        return Err(AppError::NotFound);
    }

    // TODO: Send Reject activity via ActivityPub

    // Return relationship response
    let relationship = RelationshipResponse {
        id: id.clone(),
        following: false,
        followed_by: false,
        blocking: false,
        blocked_by: false,
        muting: false,
        muting_notifications: false,
        requested: false,
        domain_blocking: false,
        showing_reblogs: true,
        endorsed: false,
        notifying: false,
        note: String::new(),
    };

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}
