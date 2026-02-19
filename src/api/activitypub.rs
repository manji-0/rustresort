//! ActivityPub endpoints
//!
//! - Actor profile
//! - Inbox (activity receiving)
//! - Outbox
//! - Followers/Following collections

use axum::body::Bytes;
use axum::{
    Router,
    extract::{Path, State},
    response::Json,
    routing::{get, post},
};
use http::HeaderMap;
use std::sync::Arc;

use crate::AppState;
use crate::data::Account;
use crate::error::AppError;
use crate::metrics::{
    ACTIVITYPUB_ACTIVITIES_RECEIVED, FEDERATION_REQUEST_DURATION_SECONDS,
    FEDERATION_REQUESTS_TOTAL, HTTP_REQUEST_DURATION_SECONDS, HTTP_REQUESTS_TOTAL,
};

fn extract_signature_key_id(headers: &HeaderMap) -> Result<String, AppError> {
    let signature = headers
        .get("signature")
        .ok_or(AppError::Unauthorized)?
        .to_str()
        .map_err(|_| AppError::Unauthorized)?;

    let parsed = crate::federation::parse_signature_header(signature)?;
    Ok(parsed.key_id)
}

fn build_activity_processor(
    state: &AppState,
    account: &Account,
) -> crate::federation::ActivityProcessor {
    let local_address = format!("{}@{}", account.username, state.config.server.domain);
    let delivery = Arc::new(crate::federation::build_local_delivery(
        state.http_client.clone(),
        &state.config.server.base_url(),
        account,
    ));

    crate::federation::ActivityProcessor::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
        state.http_client.clone(),
        local_address,
        state.config.server.protocol.clone(),
    )
    .with_delivery(delivery)
}

/// Create ActivityPub router
///
/// Routes:
/// - GET /users/:username - Actor profile
/// - POST /users/:username/inbox - Personal inbox
/// - POST /inbox - Shared inbox
/// - GET /users/:username/outbox - Outbox
/// - GET /users/:username/statuses/:id - Note object
/// - GET /users/:username/followers - Followers collection
/// - GET /users/:username/following - Following collection
pub fn activitypub_router() -> Router<AppState> {
    Router::new()
        .route("/users/:username", get(actor))
        .route("/users/:username/inbox", post(inbox))
        .route("/inbox", post(shared_inbox))
        .route("/users/:username/outbox", get(outbox))
        .route("/users/:username/statuses/:id", get(status_object))
        .route("/users/:username/followers", get(followers))
        .route("/users/:username/following", get(following))
}

/// GET /users/:username
///
/// Returns ActivityPub Actor document.
///
/// Content-Type: application/activity+json
async fn actor(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["GET", "/users/:username"])
        .start_timer();

    // Get account from database
    let account = state.db.get_account().await?;

    match account {
        Some(acc) if acc.username == username => {
            let base_url = state.config.server.base_url();
            let actor_url = format!("{}/users/{}", base_url, username);

            // Build Actor document according to ActivityPub spec
            let response = Json(serde_json::json!({
                "@context": [
                    "https://www.w3.org/ns/activitystreams",
                    "https://w3id.org/security/v1"
                ],
                "type": "Person",
                "id": actor_url.clone(),
                "preferredUsername": acc.username,
                "name": acc.display_name.unwrap_or_else(|| acc.username.clone()),
                "summary": acc.note.unwrap_or_default(),
                "inbox": format!("{}/inbox", actor_url),
                "outbox": format!("{}/outbox", actor_url),
                "followers": format!("{}/followers", actor_url),
                "following": format!("{}/following", actor_url),
                "url": actor_url.clone(),
                "publicKey": {
                    "id": format!("{}#main-key", actor_url),
                    "owner": actor_url,
                    "publicKeyPem": acc.public_key_pem
                },
                "icon": acc.avatar_s3_key.map(|key| serde_json::json!({
                    "type": "Image",
                    "mediaType": "image/webp",
                    "url": state.storage.get_public_url(&key)
                })),
                "image": acc.header_s3_key.map(|key| serde_json::json!({
                    "type": "Image",
                    "mediaType": "image/webp",
                    "url": state.storage.get_public_url(&key)
                }))
            }));

            // Record successful request
            HTTP_REQUESTS_TOTAL
                .with_label_values(&["GET", "/users/:username", "200"])
                .inc();

            Ok(response)
        }
        _ => Err(AppError::NotFound),
    }
}

/// POST /users/:username/inbox
///
/// Receives incoming ActivityPub activities.
///
/// # Steps
/// 1. Verify HTTP Signature
/// 2. Parse activity
/// 3. Process based on type
async fn inbox(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(), AppError> {
    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["POST", "/users/:username/inbox"])
        .start_timer();
    let _fed_timer = FEDERATION_REQUEST_DURATION_SECONDS
        .with_label_values(&["inbound"])
        .start_timer();

    // Verify username exists
    let account = state.db.get_account().await?;
    let account = match account {
        Some(account) if account.username == username => account,
        _ => return Err(AppError::NotFound),
    };

    // Check for Signature header first (reject unsigned requests immediately)
    if headers.get("signature").is_none() {
        FEDERATION_REQUESTS_TOTAL
            .with_label_values(&["inbound", "unauthorized"])
            .inc();
        return Err(AppError::Unauthorized);
    }

    // Parse the activity to get the actor
    let activity: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::Validation(format!("Invalid JSON: {}", e)))?;

    let actor_id = activity
        .get("actor")
        .and_then(|a: &serde_json::Value| a.as_str())
        .ok_or_else(|| AppError::Validation("Missing actor field".to_string()))?
        .to_string(); // Clone the string to avoid borrow issues;

    let signature_key_id = extract_signature_key_id(&headers)?;
    if !crate::federation::key_id_matches_actor(&signature_key_id, &actor_id)? {
        FEDERATION_REQUESTS_TOTAL
            .with_label_values(&["inbound", "unauthorized"])
            .inc();
        return Err(AppError::Unauthorized);
    }

    // Fetch the actor's public key from signature keyId.
    let public_key_pem =
        crate::federation::fetch_public_key(&signature_key_id, state.http_client.as_ref()).await?;

    // Get the request path
    let path = format!("/users/{}/inbox", username);

    // Verify the HTTP signature
    crate::federation::verify_signature("POST", &path, &headers, Some(&body), &public_key_pem)?;

    // Apply inbound federation rate limiting only after signature verification
    // to avoid unauthenticated quota poisoning.
    let actor_domain = crate::federation::extract_domain(&signature_key_id);
    state
        .federation_rate_limiter
        .check_and_increment(&actor_domain)
        .await?;

    // Record activity type
    if let Some(activity_type) = activity.get("type").and_then(|t| t.as_str()) {
        ACTIVITYPUB_ACTIVITIES_RECEIVED
            .with_label_values(&[activity_type])
            .inc();
    }

    // Process the activity
    let processor = build_activity_processor(&state, &account);

    processor.process(activity, &actor_id).await?;

    // Record successful federation request
    FEDERATION_REQUESTS_TOTAL
        .with_label_values(&["inbound", "success"])
        .inc();
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["POST", "/users/:username/inbox", "200"])
        .inc();

    Ok(())
}

/// POST /inbox
///
/// Shared inbox for all users on this instance.
/// More efficient for remote servers to deliver to multiple users.
///
/// # Steps
/// 1. Verify HTTP Signature
/// 2. Parse activity
/// 3. Route to appropriate user(s)
async fn shared_inbox(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(), AppError> {
    // Check for Signature header first (reject unsigned requests immediately)
    if headers.get("signature").is_none() {
        return Err(AppError::Unauthorized);
    }

    // Parse the activity to get the actor
    let activity: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::Validation(format!("Invalid JSON: {}", e)))?;

    let actor_id = activity
        .get("actor")
        .and_then(|a: &serde_json::Value| a.as_str())
        .ok_or_else(|| AppError::Validation("Missing actor field".to_string()))?
        .to_string(); // Clone the string to avoid borrow issues;

    let signature_key_id = extract_signature_key_id(&headers)?;
    if !crate::federation::key_id_matches_actor(&signature_key_id, &actor_id)? {
        return Err(AppError::Unauthorized);
    }

    // Fetch the actor's public key from signature keyId.
    let public_key_pem =
        crate::federation::fetch_public_key(&signature_key_id, state.http_client.as_ref()).await?;

    // Get the request path
    let path = "/inbox";

    // Verify the HTTP signature
    crate::federation::verify_signature("POST", path, &headers, Some(&body), &public_key_pem)?;

    // Apply inbound federation rate limiting only after signature verification
    // to avoid unauthenticated quota poisoning.
    let actor_domain = crate::federation::extract_domain(&signature_key_id);
    state
        .federation_rate_limiter
        .check_and_increment(&actor_domain)
        .await?;

    // Verify we have at least one account on this instance
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Process the activity
    let processor = build_activity_processor(&state, &account);

    processor.process(activity, &actor_id).await?;

    Ok(())
}

/// GET /users/:username/outbox
///
/// Returns Outbox collection (paginated).
///
/// Only public activities are included.
async fn outbox(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify username matches local account
    let account = state.db.get_account().await?;

    match account {
        Some(acc) if acc.username == username => {
            // Get outbox-safe statuses from database.
            // ActivityPub outbox must not expose private/direct posts.
            let statuses = state.db.get_local_outbox_statuses(20, None).await?;

            let base_url = state.config.server.base_url();
            let outbox_url = format!("{}/users/{}/outbox", base_url, username);
            let followers_url = format!("{}/users/{}/followers", base_url, username);
            let public_audience = "https://www.w3.org/ns/activitystreams#Public";

            // Build OrderedCollection
            let items: Vec<serde_json::Value> = statuses
                .iter()
                .map(|status| {
                    let (to, cc) = match status.visibility.as_str() {
                        "unlisted" => (
                            serde_json::json!([followers_url.clone()]),
                            serde_json::json!([public_audience]),
                        ),
                        _ => (
                            serde_json::json!([public_audience]),
                            serde_json::json!([followers_url.clone()]),
                        ),
                    };
                    let mut object = serde_json::json!({
                        "type": "Note",
                        "id": status.uri.clone(),
                        "attributedTo": format!("{}/users/{}", base_url, username),
                        "content": status.content.clone(),
                        "published": status.created_at.to_rfc3339(),
                        "to": to,
                        "cc": cc
                    });
                    if let Some(summary) = &status.content_warning {
                        object["summary"] = serde_json::json!(summary);
                        object["sensitive"] = serde_json::json!(true);
                    }
                    if let Some(in_reply_to) = &status.in_reply_to_uri {
                        object["inReplyTo"] = serde_json::json!(in_reply_to);
                    }
                    serde_json::json!({
                        "type": "Create",
                        "id": format!("{}/activity", status.uri),
                        "actor": format!("{}/users/{}", base_url, username),
                        "published": status.created_at.to_rfc3339(),
                        "to": object["to"].clone(),
                        "cc": object["cc"].clone(),
                        "object": object
                    })
                })
                .collect();

            Ok(Json(serde_json::json!({
                "@context": "https://www.w3.org/ns/activitystreams",
                "type": "OrderedCollection",
                "id": outbox_url,
                "totalItems": items.len(),
                "orderedItems": items
            })))
        }
        _ => Err(AppError::NotFound),
    }
}

/// GET /users/:username/statuses/:id
///
/// Returns a Note object for a local status URI.
async fn status_object(
    State(state): State<AppState>,
    Path((username, id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["GET", "/users/:username/statuses/:id"])
        .start_timer();

    let account = state.db.get_account().await?;
    match account {
        Some(acc) if acc.username == username => {
            let base_url = state.config.server.base_url();
            let actor_url = format!("{}/users/{}", base_url, username);
            let status_uri = format!("{}/statuses/{}", actor_url, id);
            let followers_url = format!("{}/followers", actor_url);

            let status = state
                .db
                .get_status_by_uri(&status_uri)
                .await?
                .ok_or(AppError::NotFound)?;

            if status.visibility != "public" && status.visibility != "unlisted" {
                return Err(AppError::NotFound);
            }

            let public_audience = "https://www.w3.org/ns/activitystreams#Public";
            let (to_audience, cc_audience) = match status.visibility.as_str() {
                "unlisted" => (
                    serde_json::json!([followers_url.clone()]),
                    serde_json::json!([public_audience]),
                ),
                _ => (
                    serde_json::json!([public_audience]),
                    serde_json::json!([followers_url.clone()]),
                ),
            };

            let mut note = serde_json::json!({
                "@context": "https://www.w3.org/ns/activitystreams",
                "type": "Note",
                "id": status.uri,
                "attributedTo": actor_url,
                "content": status.content,
                "published": status.created_at.to_rfc3339(),
                "to": to_audience,
                "cc": cc_audience
            });

            if let Some(summary) = status.content_warning {
                note["summary"] = serde_json::json!(summary);
                note["sensitive"] = serde_json::json!(true);
            }

            if let Some(in_reply_to) = status.in_reply_to_uri {
                note["inReplyTo"] = serde_json::json!(in_reply_to);
            }

            HTTP_REQUESTS_TOTAL
                .with_label_values(&["GET", "/users/:username/statuses/:id", "200"])
                .inc();

            Ok(Json(note))
        }
        _ => Err(AppError::NotFound),
    }
}

/// GET /users/:username/followers
///
/// Returns Followers collection.
async fn followers(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify username
    let account = state.db.get_account().await?;

    match account {
        Some(acc) if acc.username == username => {
            // Get follower addresses from database
            let follower_addresses = state.db.get_all_follower_addresses().await?;

            let base_url = state.config.server.base_url();
            let followers_url = format!("{}/users/{}/followers", base_url, username);

            // Build OrderedCollection of follower URIs
            // Note: In a real implementation, these should be actor URIs, not addresses
            // For now, we'll use placeholder URIs
            let items: Vec<String> = follower_addresses
                .iter()
                .map(|addr| {
                    format!(
                        "https://{}/users/{}",
                        addr.split('@').nth(1).unwrap_or("unknown.example"),
                        addr.split('@').next().unwrap_or("unknown")
                    )
                })
                .collect();

            Ok(Json(serde_json::json!({
                "@context": "https://www.w3.org/ns/activitystreams",
                "type": "OrderedCollection",
                "id": followers_url,
                "totalItems": items.len(),
                "orderedItems": items
            })))
        }
        _ => Err(AppError::NotFound),
    }
}

/// GET /users/:username/following
///
/// Returns Following collection.
async fn following(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify username
    let account = state.db.get_account().await?;

    match account {
        Some(acc) if acc.username == username => {
            // Get follow addresses from database
            let follow_addresses = state.db.get_all_follow_addresses().await?;

            let base_url = state.config.server.base_url();
            let following_url = format!("{}/users/{}/following", base_url, username);

            // Build OrderedCollection of following URIs
            let items: Vec<String> = follow_addresses
                .iter()
                .map(|addr| {
                    format!(
                        "https://{}/users/{}",
                        addr.split('@').nth(1).unwrap_or("unknown.example"),
                        addr.split('@').next().unwrap_or("unknown")
                    )
                })
                .collect();

            Ok(Json(serde_json::json!({
                "@context": "https://www.w3.org/ns/activitystreams",
                "type": "OrderedCollection",
                "id": following_url,
                "totalItems": items.len(),
                "orderedItems": items
            })))
        }
        _ => Err(AppError::NotFound),
    }
}
