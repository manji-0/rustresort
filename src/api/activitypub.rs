//! ActivityPub endpoints
//!
//! - Actor profile
//! - Inbox (activity receiving)
//! - Outbox
//! - Followers/Following collections

use axum::body::Bytes;
use axum::{
    Router,
    extract::{FromRef, Path, Query, State},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use http::{
    HeaderMap, HeaderValue,
    header::{CONTENT_TYPE, HeaderName},
};
use serde::Deserialize;
use std::{future::Future, sync::Arc};

use crate::ActivityPubState;
use crate::data::{Account, Repost, Status, StatusVisibility};
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

fn extract_actor_id(activity: &serde_json::Value) -> Result<String, AppError> {
    activity
        .get("actor")
        .and_then(|actor| {
            actor
                .as_str()
                .or_else(|| actor.get("id").and_then(|id| id.as_str()))
        })
        .map(str::to_string)
        .ok_or_else(|| AppError::Validation("Missing actor field".to_string()))
}

fn build_activity_processor(
    state: &ActivityPubState,
    account: &Account,
) -> crate::federation::ActivityProcessor {
    let local_address = format!("{}@{}", account.username, state.config.server.domain);
    let delivery = Arc::new(
        crate::federation::build_local_delivery(
            state.http_client.clone(),
            &state.config.server.base_url(),
            account,
        )
        .with_media_storage(state.storage.clone()),
    );

    crate::federation::ActivityProcessor::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
        local_address,
        state.config.server.protocol.clone(),
    )
    .with_federation_fetch_client(state.federation_fetch_client.clone())
    .with_delivery(delivery)
    .with_streaming_event_bus(state.streaming_event_bus.clone())
    .with_web_push_sender(state.web_push_sender.clone())
}

async fn process_inbound_activity_with_public_key_resolver<F, Fut>(
    state: &ActivityPubState,
    account: &Account,
    headers: &HeaderMap,
    body: &[u8],
    request_path: &str,
    resolve_public_key: F,
) -> Result<(), AppError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String, AppError>>,
{
    // Check for Signature header first (reject unsigned requests immediately)
    if headers.get("signature").is_none() {
        FEDERATION_REQUESTS_TOTAL
            .with_label_values(&["inbound", "unauthorized"])
            .inc();
        return Err(AppError::Unauthorized);
    }

    let activity: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| AppError::Validation(format!("Invalid JSON: {}", e)))?;
    let actor_id = extract_actor_id(&activity)?;

    let signature_key_id = extract_signature_key_id(headers)?;
    if !crate::federation::key_id_matches_actor(&signature_key_id, &actor_id)? {
        FEDERATION_REQUESTS_TOTAL
            .with_label_values(&["inbound", "unauthorized"])
            .inc();
        return Err(AppError::Unauthorized);
    }

    let public_key_pem = if let Some(override_pem) = state
        .inbound_public_key_overrides
        .read()
        .ok()
        .and_then(|overrides| overrides.get(&signature_key_id).cloned())
    {
        override_pem
    } else {
        resolve_public_key(signature_key_id.clone()).await?
    };
    crate::federation::verify_signature(
        "POST",
        request_path,
        headers,
        Some(body),
        &public_key_pem,
    )?;

    // Apply inbound federation rate limiting only after signature verification
    // to avoid unauthenticated quota poisoning.
    let actor_domain = crate::federation::extract_domain(&signature_key_id);
    state
        .federation_rate_limiter
        .check_and_increment(&actor_domain)
        .await?;

    if let Some(activity_type) = activity.get("type").and_then(|t| t.as_str()) {
        ACTIVITYPUB_ACTIVITIES_RECEIVED
            .with_label_values(&[activity_type])
            .inc();
    }

    let processor = build_activity_processor(state, account);
    processor.process(activity, &actor_id).await?;
    Ok(())
}

fn fallback_actor_uri_from_address(protocol: &str, address: &str) -> String {
    let Some((username, domain)) = address.split_once('@') else {
        return address.to_string();
    };
    format!(
        "{}://{}/users/{}",
        if protocol.eq_ignore_ascii_case("http") {
            "http"
        } else {
            "https"
        },
        domain,
        username
    )
}

#[derive(Debug, Default, Deserialize)]
struct OutboxQuery {
    page: Option<bool>,
    offset: Option<usize>,
}

enum OutboxItem {
    Create(Status),
    Announce { repost: Repost, status: Status },
}

fn activitypub_content_type() -> HeaderValue {
    HeaderValue::from_static("application/activity+json")
}

fn activitypub_status_context() -> serde_json::Value {
    serde_json::json!([
        "https://www.w3.org/ns/activitystreams",
        {
            "Hashtag": "https://www.w3.org/ns/activitystreams#Hashtag",
            "votersCount": "http://joinmastodon.org/ns#votersCount"
        }
    ])
}

fn activitypub_actor_context() -> serde_json::Value {
    serde_json::json!([
        "https://www.w3.org/ns/activitystreams",
        "https://w3id.org/security/v1",
        {
            "schema": "http://schema.org#",
            "PropertyValue": "schema:PropertyValue",
            "value": "schema:value",
            "featured": {
                "@id": "http://joinmastodon.org/ns#featured",
                "@type": "@id"
            },
            "featuredTags": {
                "@id": "http://joinmastodon.org/ns#featuredTags",
                "@type": "@id"
            },
            "discoverable": "http://joinmastodon.org/ns#discoverable",
            "indexable": "http://joinmastodon.org/ns#indexable"
        }
    ])
}

fn activitypub_json_response(value: serde_json::Value) -> Response {
    (
        [(
            HeaderName::from_static(CONTENT_TYPE.as_str()),
            activitypub_content_type(),
        )],
        Json(value),
    )
        .into_response()
}

fn ensure_public_activity_visibility(visibility: StatusVisibility) -> Result<(), AppError> {
    if matches!(
        visibility,
        StatusVisibility::Public | StatusVisibility::Unlisted
    ) {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}

fn activitypub_audience(
    followers_url: &str,
    visibility: StatusVisibility,
) -> (serde_json::Value, serde_json::Value) {
    let public_audience = "https://www.w3.org/ns/activitystreams#Public";
    match visibility {
        StatusVisibility::Unlisted => (
            serde_json::json!([followers_url]),
            serde_json::json!([public_audience]),
        ),
        _ => (
            serde_json::json!([public_audience]),
            serde_json::json!([followers_url]),
        ),
    }
}

fn activitypub_attachment_type(content_type: &str) -> &'static str {
    if content_type.starts_with("image/") {
        "Image"
    } else if content_type.starts_with("video/") {
        "Video"
    } else if content_type.starts_with("audio/") {
        "Audio"
    } else {
        "Document"
    }
}

async fn build_status_tags(
    state: &ActivityPubState,
    content: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    let mut tags = crate::data::extract_hashtags_from_content(content)
        .into_iter()
        .map(|name| {
            serde_json::json!({
                "type": "Hashtag",
                "href": format!("{}/tagged/{}", state.config.server.base_url(), name),
                "name": format!("#{}", name),
            })
        })
        .collect::<Vec<_>>();
    tags.extend(
        crate::api::mastodon::federation_delivery::extract_mentions_from_content(content)
            .into_iter()
            .filter(|mention| {
                mention.split_once('@').is_some_and(|(_, domain)| {
                    domain.eq_ignore_ascii_case(&state.config.server.domain)
                })
            })
            .map(|mention| {
                let username = mention
                    .split_once('@')
                    .map(|(username, _)| username)
                    .unwrap_or_default();
                serde_json::json!({
                    "type": "Mention",
                    "href": format!("{}/users/{}", state.config.server.base_url(), username),
                    "name": format!("@{}", mention),
                })
            }),
    );
    let recipients =
        crate::api::mastodon::federation_delivery::resolve_remote_recipients_with_dependencies(
            state.db.as_ref(),
            state.profile_cache.as_ref(),
            state.federation_fetch_client.as_ref(),
            crate::api::mastodon::federation_delivery::extract_remote_mentions_from_content(
                content,
                &state.config.server.domain,
            ),
        )
        .await;
    tags.extend(recipients.into_iter().map(|recipient| {
        serde_json::json!({
            "type": "Mention",
            "href": recipient.actor_uri,
            "name": format!("@{}", recipient.address),
        })
    }));
    Ok(tags)
}

async fn build_note_object(
    state: &ActivityPubState,
    actor_url: &str,
    followers_url: &str,
    status: &Status,
) -> Result<serde_json::Value, AppError> {
    let (to_audience, cc_audience) = activitypub_audience(followers_url, status.visibility);
    let poll = state.db.get_poll_by_status_id(&status.id).await?;
    let mut note =
        if let Some((poll_id, expires_at, expired, multiple, _votes_count, voters_count)) = poll {
            let options = state
                .db
                .get_poll_options(&poll_id)
                .await?
                .into_iter()
                .map(|(_, title, votes_count)| {
                    serde_json::json!({
                        "type": "Note",
                        "name": title,
                        "replies": {
                            "type": "Collection",
                            "totalItems": votes_count,
                        }
                    })
                })
                .collect::<Vec<_>>();
            let mut object = serde_json::json!({
                "type": "Question",
                "id": status.uri.clone(),
                "attributedTo": actor_url,
                "content": status.content.clone(),
                "published": status.created_at.to_rfc3339(),
                "to": to_audience,
                "cc": cc_audience,
                "endTime": expires_at,
                "votersCount": voters_count,
            });
            if expired {
                object["closed"] = serde_json::json!(expires_at);
            }
            if multiple {
                object["anyOf"] = serde_json::json!(options);
            } else {
                object["oneOf"] = serde_json::json!(options);
            }
            object
        } else {
            serde_json::json!({
                "type": "Note",
                "id": status.uri.clone(),
                "attributedTo": actor_url,
                "content": status.content.clone(),
                "published": status.created_at.to_rfc3339(),
                "to": to_audience,
                "cc": cc_audience
            })
        };

    if let Some(summary) = &status.content_warning {
        note["summary"] = serde_json::json!(summary);
        note["sensitive"] = serde_json::json!(true);
    }
    if let Some(in_reply_to) = &status.in_reply_to_uri {
        note["inReplyTo"] = serde_json::json!(in_reply_to);
    }
    if let Some(quote_of_uri) = &status.quote_of_uri {
        note["quoteUri"] = serde_json::json!(quote_of_uri);
        note["quoteUrl"] = serde_json::json!(quote_of_uri);
    }

    if let Some(language) = &status.language {
        let mut content_map = serde_json::Map::new();
        content_map.insert(language.clone(), serde_json::json!(status.content.clone()));
        note["contentMap"] = serde_json::Value::Object(content_map);
    }
    let tags = build_status_tags(state, &status.content).await?;
    if !tags.is_empty() {
        note["tag"] = serde_json::json!(tags);
    }

    let attachments = state
        .db
        .get_media_by_status(&status.id)
        .await?
        .into_iter()
        .map(|attachment| {
            let url = state.storage.get_public_url(&attachment.s3_key);
            let preview_url = attachment
                .thumbnail_s3_key
                .as_ref()
                .map(|key| state.storage.get_public_url(key));
            let content_type = attachment.content_type.clone();
            let mut object = serde_json::json!({
                "type": activitypub_attachment_type(&content_type),
                "mediaType": content_type,
                "url": url,
            });
            if let Some(description) = attachment.description {
                object["name"] = serde_json::json!(description);
            }
            if let Some(blurhash) = attachment.blurhash {
                object["blurhash"] = serde_json::json!(blurhash);
            }
            if let Some(width) = attachment.width {
                object["width"] = serde_json::json!(width);
            }
            if let Some(height) = attachment.height {
                object["height"] = serde_json::json!(height);
            }
            if let Some(preview_url) = preview_url {
                object["icon"] = serde_json::json!({
                    "type": "Image",
                    "mediaType": attachment.content_type,
                    "url": preview_url,
                });
            }
            object
        })
        .collect::<Vec<_>>();
    if !attachments.is_empty() {
        note["attachment"] = serde_json::json!(attachments);
    }

    Ok(note)
}

pub(crate) fn build_local_actor_document(
    storage: &dyn crate::storage::MediaStorageRepository,
    base_url: &str,
    account: &Account,
) -> serde_json::Value {
    let actor_url = format!("{}/users/{}", base_url, account.username);
    let attachments = crate::profile_fields::activitypub_profile_attachments(
        account.profile_fields_json.as_deref(),
    );
    let mut actor = serde_json::json!({
        "@context": activitypub_actor_context(),
        "type": "Person",
        "id": actor_url.clone(),
        "preferredUsername": account.username.clone(),
        "name": account.display_name.clone().unwrap_or_else(|| account.username.clone()),
        "summary": account.note.clone().unwrap_or_default(),
        "inbox": format!("{}/inbox", actor_url),
        "outbox": format!("{}/outbox", actor_url),
        "followers": format!("{}/followers", actor_url),
        "following": format!("{}/following", actor_url),
        "featured": format!("{}/collections/featured", actor_url),
        "featuredTags": format!("{}/collections/tags", actor_url),
        "manuallyApprovesFollowers": account.locked,
        "endpoints": {
            "sharedInbox": format!("{}/inbox", base_url)
        },
        "discoverable": account.discoverable,
        "indexable": account.indexable,
        "bot": account.bot,
        "url": actor_url.clone(),
        "publicKey": {
            "id": format!("{}#main-key", actor_url),
            "owner": actor_url,
            "publicKeyPem": account.public_key_pem.clone()
        },
        "icon": account.avatar_s3_key.as_ref().map(|key| serde_json::json!({
            "type": "Image",
            "mediaType": "image/webp",
            "url": storage.get_public_url(key)
        })),
        "image": account.header_s3_key.as_ref().map(|key| serde_json::json!({
            "type": "Image",
            "mediaType": "image/webp",
            "url": storage.get_public_url(key)
        }))
    });

    if !attachments.is_empty() {
        actor["attachment"] = serde_json::json!(attachments);
    }

    if let Some(also_known_as) = account.also_known_as.as_deref() {
        actor["alsoKnownAs"] = serde_json::json!([also_known_as]);
    }
    if let Some(moved_to_uri) = account.moved_to_uri.as_deref() {
        actor["movedTo"] = serde_json::json!(moved_to_uri);
    }

    actor
}

async fn build_create_activity(
    state: &ActivityPubState,
    actor_url: &str,
    followers_url: &str,
    status: &Status,
) -> Result<serde_json::Value, AppError> {
    let object = build_note_object(state, actor_url, followers_url, status).await?;
    Ok(serde_json::json!({
        "@context": activitypub_status_context(),
        "type": "Create",
        "id": format!("{}/activity", status.uri),
        "actor": actor_url,
        "published": status.created_at.to_rfc3339(),
        "to": object["to"].clone(),
        "cc": object["cc"].clone(),
        "object": object
    }))
}

fn build_announce_activity(
    actor_url: &str,
    followers_url: &str,
    repost: &Repost,
    status: &Status,
) -> serde_json::Value {
    let (to_audience, cc_audience) = activitypub_audience(followers_url, status.visibility);
    serde_json::json!({
        "@context": activitypub_status_context(),
        "type": "Announce",
        "id": repost.uri,
        "actor": actor_url,
        "published": repost.created_at.to_rfc3339(),
        "to": to_audience,
        "cc": cc_audience,
        "object": status.uri
    })
}

fn outbox_item_created_at(item: &OutboxItem) -> chrono::DateTime<chrono::Utc> {
    match item {
        OutboxItem::Create(status) => status.created_at,
        OutboxItem::Announce { repost, .. } => repost.created_at,
    }
}

fn outbox_item_id(item: &OutboxItem) -> &str {
    match item {
        OutboxItem::Create(status) => &status.id,
        OutboxItem::Announce { repost, .. } => &repost.id,
    }
}

async fn load_outbox_items(
    state: &ActivityPubState,
    fetch_limit: usize,
) -> Result<Vec<OutboxItem>, AppError> {
    let mut items = Vec::new();

    for status in state
        .db
        .get_local_outbox_statuses(fetch_limit, None)
        .await?
    {
        items.push(OutboxItem::Create(status));
    }

    for repost in state.db.get_local_outbox_reposts(fetch_limit).await? {
        if let Some(status) = state.db.get_status(&repost.status_id).await?
            && matches!(
                status.visibility,
                StatusVisibility::Public | StatusVisibility::Unlisted
            )
        {
            items.push(OutboxItem::Announce { repost, status });
        }
    }

    items.sort_by(|left, right| {
        outbox_item_created_at(right)
            .cmp(&outbox_item_created_at(left))
            .then_with(|| outbox_item_id(right).cmp(outbox_item_id(left)))
    });
    items.truncate(fetch_limit);
    Ok(items)
}

/// Create ActivityPub router
///
/// Routes:
/// - GET /users/:username - Actor profile
/// - POST /users/:username/inbox - Personal inbox
/// - POST /inbox - Shared inbox
/// - GET /users/:username/outbox - Outbox
/// - GET /users/:username/statuses/:id - Note object
/// - GET /users/:username/statuses/:id/activity - Create or Announce activity
/// - GET /users/:username/followers - Followers collection
/// - GET /users/:username/following - Following collection
pub fn activitypub_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    ActivityPubState: FromRef<S>,
{
    Router::new()
        .route("/users/:username", get(actor))
        .route("/users/:username/inbox", post(inbox))
        .route("/inbox", post(shared_inbox))
        .route("/users/:username/outbox", get(outbox))
        .route("/users/:username/statuses/:id", get(status_object))
        .route(
            "/users/:username/statuses/:id/activity",
            get(status_activity),
        )
        .route("/users/:username/collections/featured", get(featured))
        .route("/users/:username/collections/tags", get(featured_tags))
        .route(
            "/users/:username/tagged/:hashtag",
            get(actor_tag_collection),
        )
        .route("/users/:username/followers", get(followers))
        .route("/users/:username/following", get(following))
        .route("/tags/:hashtag", get(tag_collection))
        .route("/tagged/:hashtag", get(tag_collection))
}

/// GET /users/:username
///
/// Returns ActivityPub Actor document.
///
/// Content-Type: application/activity+json
async fn actor(
    State(state): State<ActivityPubState>,
    Path(username): Path<String>,
) -> Result<Response, AppError> {
    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["GET", "/users/:username"])
        .start_timer();

    // Get account from database
    let account = state.db.get_account().await?;

    match account {
        Some(acc) if acc.username == username => {
            let base_url = state.config.server.base_url();
            let actor = build_local_actor_document(state.storage.as_ref(), &base_url, &acc);

            // Build Actor document according to ActivityPub spec
            let response = activitypub_json_response(actor);

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
    State(state): State<ActivityPubState>,
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

    let path = format!("/users/{}/inbox", username);
    let public_key_cache = state.public_key_cache.clone();
    process_inbound_activity_with_public_key_resolver(
        &state,
        &account,
        &headers,
        &body,
        &path,
        move |key_id| {
            let public_key_cache = public_key_cache.clone();
            async move { public_key_cache.get(&key_id).await }
        },
    )
    .await?;

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
    State(state): State<ActivityPubState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(), AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let public_key_cache = state.public_key_cache.clone();
    process_inbound_activity_with_public_key_resolver(
        &state,
        &account,
        &headers,
        &body,
        "/inbox",
        move |key_id| {
            let public_key_cache = public_key_cache.clone();
            async move { public_key_cache.get(&key_id).await }
        },
    )
    .await?;

    Ok(())
}

/// GET /users/:username/outbox
///
/// Returns Outbox collection (paginated).
///
/// Only public activities are included.
async fn outbox(
    State(state): State<ActivityPubState>,
    Path(username): Path<String>,
    Query(query): Query<OutboxQuery>,
) -> Result<Response, AppError> {
    // Verify username matches local account
    let account = state.db.get_account().await?;

    match account {
        Some(acc) if acc.username == username => {
            let page_size = 20usize;
            let offset = query.offset.unwrap_or(0);
            let fetch_limit = offset.saturating_add(page_size);
            let items = load_outbox_items(&state, fetch_limit).await?;
            let total_items = state.db.count_local_outbox_statuses().await?
                + state.db.count_local_outbox_reposts().await?;
            let base_url = state.config.server.base_url();
            let outbox_url = format!("{}/users/{}/outbox", base_url, username);
            let actor_url = format!("{}/users/{}", base_url, username);
            let followers_url = format!("{}/users/{}/followers", base_url, username);
            let mut ordered_items = Vec::new();
            for item in items.iter().skip(offset).take(page_size) {
                let value = match item {
                    OutboxItem::Create(status) => {
                        build_create_activity(&state, &actor_url, &followers_url, status).await?
                    }
                    OutboxItem::Announce { repost, status } => {
                        build_announce_activity(&actor_url, &followers_url, repost, status)
                    }
                };
                ordered_items.push(value);
            }

            if query.page.unwrap_or(false) {
                let next_offset = offset.saturating_add(page_size);
                let next = (next_offset < total_items as usize)
                    .then(|| format!("{outbox_url}?page=true&offset={next_offset}"));
                return Ok(activitypub_json_response(serde_json::json!({
                    "@context": "https://www.w3.org/ns/activitystreams",
                    "type": "OrderedCollectionPage",
                    "id": format!("{outbox_url}?page=true&offset={offset}"),
                    "partOf": outbox_url,
                    "orderedItems": ordered_items,
                    "next": next
                })));
            }

            Ok(activitypub_json_response(serde_json::json!({
                "@context": "https://www.w3.org/ns/activitystreams",
                "type": "OrderedCollection",
                "id": outbox_url,
                "totalItems": total_items,
                "first": format!("{outbox_url}?page=true"),
                "orderedItems": ordered_items
            })))
        }
        _ => Err(AppError::NotFound),
    }
}

/// GET /users/:username/statuses/:id
///
/// Returns a Note object for a local status URI.
async fn status_object(
    State(state): State<ActivityPubState>,
    Path((username, id)): Path<(String, String)>,
) -> Result<Response, AppError> {
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
            ensure_public_activity_visibility(status.visibility)?;
            let mut note = build_note_object(&state, &actor_url, &followers_url, &status).await?;
            note["@context"] = activitypub_status_context();

            HTTP_REQUESTS_TOTAL
                .with_label_values(&["GET", "/users/:username/statuses/:id", "200"])
                .inc();

            Ok(activitypub_json_response(note))
        }
        _ => Err(AppError::NotFound),
    }
}

async fn status_activity(
    State(state): State<ActivityPubState>,
    Path((username, id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let account = state.db.get_account().await?;
    match account {
        Some(acc) if acc.username == username => {
            let base_url = state.config.server.base_url();
            let actor_url = format!("{}/users/{}", base_url, username);
            let followers_url = format!("{}/followers", actor_url);
            let activity_uri = format!("{actor_url}/statuses/{id}/activity");

            if let Some(repost) = state.db.get_repost_by_uri(&activity_uri).await? {
                let status = state
                    .db
                    .get_status(&repost.status_id)
                    .await?
                    .ok_or(AppError::NotFound)?;
                ensure_public_activity_visibility(status.visibility)?;
                return Ok(activitypub_json_response(build_announce_activity(
                    &actor_url,
                    &followers_url,
                    &repost,
                    &status,
                )));
            }

            let status_uri = format!("{actor_url}/statuses/{id}");
            let status = state
                .db
                .get_status_by_uri(&status_uri)
                .await?
                .ok_or(AppError::NotFound)?;
            ensure_public_activity_visibility(status.visibility)?;
            Ok(activitypub_json_response(
                build_create_activity(&state, &actor_url, &followers_url, &status).await?,
            ))
        }
        _ => Err(AppError::NotFound),
    }
}

async fn featured(
    State(state): State<ActivityPubState>,
    Path(username): Path<String>,
) -> Result<Response, AppError> {
    let account = state.db.get_account().await?;
    match account {
        Some(acc) if acc.username == username => {
            let actor_url = format!("{}/users/{}", state.config.server.base_url(), username);
            let followers_url = format!("{}/followers", actor_url);
            let timeline_service = crate::service::TimelineService::new(
                state.db.clone(),
                state.timeline_cache.clone(),
                state.profile_cache.clone(),
            );
            let pinned_statuses = timeline_service
                .account_timeline(None, None, 40, None, None, false, false, false, true)
                .await?
                .into_iter()
                .map(|item| item.status)
                .collect::<Vec<_>>();

            let mut ordered_items = Vec::with_capacity(pinned_statuses.len());
            for status in pinned_statuses {
                ordered_items
                    .push(build_note_object(&state, &actor_url, &followers_url, &status).await?);
            }

            Ok(activitypub_json_response(serde_json::json!({
                "@context": activitypub_status_context(),
                "type": "OrderedCollection",
                "id": format!("{}/collections/featured", actor_url),
                "totalItems": ordered_items.len(),
                "orderedItems": ordered_items
            })))
        }
        _ => Err(AppError::NotFound),
    }
}

async fn featured_tags(
    State(state): State<ActivityPubState>,
    Path(username): Path<String>,
) -> Result<Response, AppError> {
    let account = state.db.get_account().await?;
    match account {
        Some(acc) if acc.username == username => {
            let actor_url = format!("{}/users/{}", state.config.server.base_url(), username);
            let ordered_items = state
                .db
                .list_featured_tags()
                .await?
                .into_iter()
                .map(|(_, name, _, _)| {
                    serde_json::json!({
                        "type": "Hashtag",
                        "href": format!("{}/tagged/{}", actor_url, name),
                        "name": format!("#{}", name),
                    })
                })
                .collect::<Vec<_>>();
            Ok(activitypub_json_response(serde_json::json!({
                "@context": activitypub_status_context(),
                "type": "OrderedCollection",
                "id": format!("{}/collections/tags", actor_url),
                "totalItems": ordered_items.len(),
                "orderedItems": ordered_items
            })))
        }
        _ => Err(AppError::NotFound),
    }
}

async fn tag_collection(
    State(state): State<ActivityPubState>,
    Path(hashtag): Path<String>,
) -> Result<Response, AppError> {
    build_tag_collection_response(&state, None, &hashtag).await
}

async fn actor_tag_collection(
    State(state): State<ActivityPubState>,
    Path((username, hashtag)): Path<(String, String)>,
) -> Result<Response, AppError> {
    build_tag_collection_response(&state, Some(&username), &hashtag).await
}

async fn build_tag_collection_response(
    state: &ActivityPubState,
    username: Option<&str>,
    hashtag: &str,
) -> Result<Response, AppError> {
    if let Some(username) = username {
        let account = state.db.get_account().await?;
        match account {
            Some(acc) if acc.username == username => {}
            _ => return Err(AppError::NotFound),
        }
    }
    let normalized = hashtag.trim().trim_start_matches('#');
    let statuses = state
        .db
        .get_statuses_by_hashtag_in_window(normalized, 40, None, None)
        .await?;
    let ordered_items = statuses
        .into_iter()
        .map(|status| serde_json::json!(status.uri))
        .collect::<Vec<_>>();
    let base_url = state.config.server.base_url();

    Ok(activitypub_json_response(serde_json::json!({
        "@context": activitypub_status_context(),
        "type": "OrderedCollection",
        "id": username
            .map(|username| format!("{}/users/{}/tagged/{}", base_url, username, normalized))
            .unwrap_or_else(|| format!("{}/tagged/{}", base_url, normalized)),
        "totalItems": ordered_items.len(),
        "orderedItems": ordered_items
    })))
}

/// GET /users/:username/followers
///
/// Returns Followers collection.
async fn followers(
    State(state): State<ActivityPubState>,
    Path(username): Path<String>,
) -> Result<Response, AppError> {
    // Verify username
    let account = state.db.get_account().await?;

    match account {
        Some(acc) if acc.username == username => {
            let followers = state.db.get_all_followers().await?;

            let base_url = state.config.server.base_url();
            let followers_url = format!("{}/users/{}/followers", base_url, username);

            let items: Vec<String> = followers
                .iter()
                .map(|follower| {
                    follower.actor_uri.clone().unwrap_or_else(|| {
                        fallback_actor_uri_from_address(
                            &state.config.server.protocol,
                            &follower.follower_address,
                        )
                    })
                })
                .collect();

            Ok(activitypub_json_response(serde_json::json!({
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
    State(state): State<ActivityPubState>,
    Path(username): Path<String>,
) -> Result<Response, AppError> {
    // Verify username
    let account = state.db.get_account().await?;

    match account {
        Some(acc) if acc.username == username => {
            let follows = state.db.get_all_follows().await?;

            let base_url = state.config.server.base_url();
            let following_url = format!("{}/users/{}/following", base_url, username);

            let items: Vec<String> = follows
                .iter()
                .map(|follow| {
                    follow.actor_uri.clone().unwrap_or_else(|| {
                        fallback_actor_uri_from_address(
                            &state.config.server.protocol,
                            &follow.target_address,
                        )
                    })
                })
                .collect();

            Ok(activitypub_json_response(serde_json::json!({
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

#[cfg(test)]
mod tests {
    use super::{extract_actor_id, process_inbound_activity_with_public_key_resolver};
    use crate::{
        ActivityPubState, AppState, config,
        data::{Account, Database, EntityId, NotificationType},
        federation::sign_request,
    };
    use axum::extract::FromRef;
    use chrono::Utc;
    use http::{HeaderMap, HeaderValue};
    use serde_json::json;
    use tempfile::TempDir;

    const TEST_PRIVATE_KEY_PEM: &str = include_str!("../../tests/fixtures/test_private_key.pem");
    const TEST_PUBLIC_KEY_PEM: &str = include_str!("../../tests/fixtures/test_public_key.pem");

    async fn create_test_activitypub_state() -> (ActivityPubState, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let config = config::AppConfig {
            server: config::ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 0,
                domain: "test.example.com".to_string(),
                protocol: "https".to_string(),
                trusted_proxy_ips: Vec::new(),
            },
            database: config::DatabaseConfig {
                path: db_path.clone(),
                sync: config::DatabaseSyncConfig::default(),
            },
            storage: config::StorageConfig {
                media: config::MediaStorageConfig {
                    bucket: "test-media".to_string(),
                    public_url: "https://media.test.example.com".to_string(),
                },
                backup: config::BackupStorageConfig {
                    enabled: false,
                    bucket: "test-backup".to_string(),
                    interval_seconds: 86400,
                    retention_count: 7,
                    encryption: config::BackupEncryptionConfig::default(),
                },
            },
            cloudflare: config::CloudflareConfig {
                account_id: "test-account".to_string(),
                r2_access_key_id: "test-key".to_string(),
                r2_secret_access_key: "test-secret".to_string(),
            },
            auth: config::AuthConfig {
                username: "testuser".to_string(),
                password: Some("test-password".to_string()),
                session_secret: "test-secret-key-32-bytes-long!!".to_string(),
                session_max_age: 604800,
            },
            instance: config::InstanceConfig {
                title: "Test Instance".to_string(),
                description: "Test RustResort Instance".to_string(),
                contact_email: "test@example.com".to_string(),
            },
            admin: config::AdminConfig {
                display_name: "Test User".to_string(),
                email: Some("testuser@test.example.com".to_string()),
                note: Some("Test account".to_string()),
            },
            cache: config::CacheConfig {
                timeline_max_items: 2000,
                profile_ttl: 86400,
            },
            ui: config::UiConfig::default(),
            metrics: config::MetricsConfig::default(),
            logging: config::LoggingConfig {
                level: "info".to_string(),
                format: "pretty".to_string(),
            },
        };

        // Pre-seed the singleton account to avoid RSA generation in AppState::new.
        let db = Database::connect(&db_path).await.unwrap();
        let now = Utc::now();
        db.upsert_account(&Account {
            id: EntityId::new_string(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: Some("Test account".to_string()),
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: TEST_PRIVATE_KEY_PEM.to_string(),
            public_key_pem: TEST_PUBLIC_KEY_PEM.to_string(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

        let app_state = AppState::new(config).await.unwrap();
        (ActivityPubState::from_ref(&app_state), temp_dir)
    }

    fn build_signed_headers(url: &str, body: &[u8], key_id: &str) -> HeaderMap {
        let signed = sign_request("POST", url, Some(body), TEST_PRIVATE_KEY_PEM, key_id).unwrap();
        let parsed_url = url::Url::parse(url).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::HOST,
            HeaderValue::from_str(parsed_url.host_str().unwrap()).unwrap(),
        );
        headers.insert(
            http::header::DATE,
            HeaderValue::from_str(&signed.date).unwrap(),
        );
        headers.insert(
            "signature",
            HeaderValue::from_str(&signed.signature).unwrap(),
        );
        headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/activity+json"),
        );
        if let Some(digest) = signed.digest {
            headers.insert("digest", HeaderValue::from_str(&digest).unwrap());
        }
        headers
    }

    #[test]
    fn extract_actor_id_accepts_string_actor() {
        let activity = json!({
            "actor": "https://remote.example/users/alice"
        });

        assert_eq!(
            extract_actor_id(&activity).unwrap(),
            "https://remote.example/users/alice"
        );
    }

    #[test]
    fn extract_actor_id_accepts_embedded_actor_object() {
        let activity = json!({
            "actor": {
                "id": "https://remote.example/@alice",
                "inbox": "https://remote.example/inbox"
            }
        });

        assert_eq!(
            extract_actor_id(&activity).unwrap(),
            "https://remote.example/@alice"
        );
    }

    #[tokio::test]
    async fn signed_personal_inbox_follow_persists_follower_and_notification() {
        let (state, _temp_dir) = create_test_activitypub_state().await;
        let account = state.db.get_account().await.unwrap().unwrap();
        let activity = json!({
            "id": "https://remote.example/follows/1",
            "type": "Follow",
            "actor": {
                "id": "https://remote.example/users/alice",
                "inbox": "https://remote.example/users/alice/inbox"
            },
            "object": "https://test.example.com/users/testuser"
        });
        let body = serde_json::to_vec(&activity).unwrap();
        let headers = build_signed_headers(
            "https://test.example.com/users/testuser/inbox",
            &body,
            "https://remote.example/users/alice#main-key",
        );

        process_inbound_activity_with_public_key_resolver(
            &state,
            &account,
            &headers,
            &body,
            "/users/testuser/inbox",
            |_| async { Ok(TEST_PUBLIC_KEY_PEM.to_string()) },
        )
        .await
        .unwrap();

        let followers = state.db.get_all_followers().await.unwrap();
        assert_eq!(followers.len(), 1);
        assert_eq!(followers[0].follower_address, "alice@remote.example");
        let notifications = state.db.get_notifications(10, None, false).await.unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].notification_type, NotificationType::Follow);
    }

    #[tokio::test]
    async fn signed_shared_inbox_create_with_embedded_actor_persists_mention() {
        let (state, _temp_dir) = create_test_activitypub_state().await;
        let account = state.db.get_account().await.unwrap().unwrap();
        let status_uri = "https://remote.example/users/alice/statuses/1";
        let activity = json!({
            "id": "https://remote.example/activities/create-1",
            "type": "Create",
            "actor": {
                "id": "https://remote.example/users/alice",
                "inbox": "https://remote.example/inbox"
            },
            "object": {
                "type": "Note",
                "id": status_uri,
                "attributedTo": "https://remote.example/users/alice",
                "content": "<p>Hello</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": "https://test.example.com/users/testuser/"
            }
        });
        let body = serde_json::to_vec(&activity).unwrap();
        let headers = build_signed_headers(
            "https://test.example.com/inbox",
            &body,
            "https://remote.example/users/alice#main-key",
        );

        process_inbound_activity_with_public_key_resolver(
            &state,
            &account,
            &headers,
            &body,
            "/inbox",
            |_| async { Ok(TEST_PUBLIC_KEY_PEM.to_string()) },
        )
        .await
        .unwrap();

        let status = state.db.get_status_by_uri(status_uri).await.unwrap();
        assert!(status.is_some());
        let notifications = state.db.get_notifications(10, None, false).await.unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(
            notifications[0].notification_type,
            NotificationType::Mention
        );
        assert_eq!(notifications[0].status_uri.as_deref(), Some(status_uri));
    }
}
