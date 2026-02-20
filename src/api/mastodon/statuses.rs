//! Status endpoints

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::accounts::PaginationParams;
use super::federation_delivery::{
    build_delivery, local_actor_uri, resolve_remote_actor_and_inbox,
    spawn_best_effort_batch_delivery, spawn_best_effort_delivery,
};
use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::metrics::{
    DB_QUERIES_TOTAL, DB_QUERY_DURATION_SECONDS, HTTP_REQUEST_DURATION_SECONDS,
    HTTP_REQUESTS_TOTAL, POSTS_TOTAL,
};
use crate::service::{AccountService, StatusService};

const DEFAULT_VISIBILITY: &str = "public";
const CREATE_STATUS_IDEMPOTENCY_ENDPOINT: &str = "/api/v1/statuses";
const MAX_IDEMPOTENCY_KEY_LENGTH: usize = 256;
const MIN_POLL_OPTIONS: usize = 2;
const MAX_POLL_OPTIONS: usize = 4;
const MAX_POLL_OPTION_CHARS: usize = 50;
const MIN_POLL_EXPIRES_IN_SECONDS: i64 = 300;
const MAX_POLL_EXPIRES_IN_SECONDS: i64 = 2_629_746;
const IDEMPOTENCY_PENDING_WAIT_TIMEOUT_MS: u64 = 5_000;
const IDEMPOTENCY_PENDING_RETRY_DELAY_MS: u64 = 50;

#[derive(Debug, Deserialize)]
pub struct CreateStatusPollRequest {
    pub options: Vec<String>,
    pub expires_in: i64,
    #[serde(default)]
    pub multiple: bool,
    #[serde(default, rename = "hide_totals")]
    pub _hide_totals: bool,
}

/// Status creation request
#[derive(Debug, Deserialize)]
pub struct CreateStatusRequest {
    pub status: Option<String>,
    pub media_ids: Option<Vec<String>>,
    pub in_reply_to_id: Option<String>,
    pub quoted_status_id: Option<String>,
    pub poll: Option<CreateStatusPollRequest>,
    pub scheduled_at: Option<String>,
    pub sensitive: Option<bool>,
    pub spoiler_text: Option<String>,
    pub visibility: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct StatusActionParams {
    pub uri: Option<String>,
}

fn resolve_action_uri<'a>(
    id: &'a str,
    params: &'a StatusActionParams,
) -> Result<Option<&'a str>, AppError> {
    if let Some(uri) = params.uri.as_deref() {
        let trimmed = uri.trim();
        if trimmed.is_empty() {
            return Err(AppError::Validation(
                "uri query parameter cannot be empty".to_string(),
            ));
        }
        return Ok(Some(trimmed));
    }

    if id.starts_with("http://") || id.starts_with("https://") {
        return Ok(Some(id));
    }

    Ok(None)
}

fn should_federate_to_followers(visibility: &str) -> bool {
    visibility == "public" || visibility == "unlisted"
}

fn ensure_public_visibility_for_public_endpoint(visibility: &str) -> Result<(), AppError> {
    if should_federate_to_followers(visibility) {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}

fn normalize_visibility_input(raw_visibility: Option<String>) -> Result<String, AppError> {
    let visibility = raw_visibility
        .unwrap_or_else(|| DEFAULT_VISIBILITY.to_string())
        .trim()
        .to_ascii_lowercase();

    match visibility.as_str() {
        "public" | "unlisted" | "private" | "direct" => Ok(visibility),
        _ => Err(AppError::Validation(
            "visibility must be one of: public, unlisted, private, direct".to_string(),
        )),
    }
}

#[derive(Debug)]
struct NormalizedCreatePoll {
    options: Vec<String>,
    expires_in: i64,
    multiple: bool,
}

fn normalize_poll_input(
    raw_poll: Option<CreateStatusPollRequest>,
) -> Result<Option<NormalizedCreatePoll>, AppError> {
    let Some(poll) = raw_poll else {
        return Ok(None);
    };

    let options: Vec<String> = poll
        .options
        .into_iter()
        .map(|option| option.trim().to_string())
        .collect();
    if !(MIN_POLL_OPTIONS..=MAX_POLL_OPTIONS).contains(&options.len()) {
        return Err(AppError::Validation(format!(
            "poll options must be between {} and {}",
            MIN_POLL_OPTIONS, MAX_POLL_OPTIONS
        )));
    }
    if options.iter().any(|option| option.is_empty()) {
        return Err(AppError::Validation(
            "poll options must not be empty".to_string(),
        ));
    }
    if options
        .iter()
        .any(|option| option.chars().count() > MAX_POLL_OPTION_CHARS)
    {
        return Err(AppError::Validation(format!(
            "poll option must be at most {} characters",
            MAX_POLL_OPTION_CHARS
        )));
    }
    if poll.expires_in < MIN_POLL_EXPIRES_IN_SECONDS {
        return Err(AppError::Validation(format!(
            "poll expires_in must be at least {} seconds",
            MIN_POLL_EXPIRES_IN_SECONDS
        )));
    }
    if poll.expires_in > MAX_POLL_EXPIRES_IN_SECONDS {
        return Err(AppError::Validation(format!(
            "poll expires_in must be at most {} seconds",
            MAX_POLL_EXPIRES_IN_SECONDS
        )));
    }

    Ok(Some(NormalizedCreatePoll {
        options,
        expires_in: poll.expires_in,
        multiple: poll.multiple,
    }))
}

fn normalize_scheduled_at(raw_scheduled_at: Option<String>) -> Result<Option<String>, AppError> {
    let Some(raw_scheduled_at) = raw_scheduled_at else {
        return Ok(None);
    };

    let scheduled_at = chrono::DateTime::parse_from_rfc3339(raw_scheduled_at.trim())
        .map_err(|_| AppError::Validation("scheduled_at must be RFC3339".to_string()))?
        .with_timezone(&Utc);
    if scheduled_at <= Utc::now() {
        return Err(AppError::Unprocessable(
            "scheduled_at must be in the future".to_string(),
        ));
    }

    Ok(Some(scheduled_at.to_rfc3339()))
}

fn extract_idempotency_key(headers: &HeaderMap) -> Result<Option<String>, AppError> {
    let Some(raw) = headers.get("Idempotency-Key") else {
        return Ok(None);
    };

    let key = raw
        .to_str()
        .map_err(|_| AppError::Validation("Idempotency-Key must be ASCII".to_string()))?
        .trim();

    if key.is_empty() {
        return Err(AppError::Validation(
            "Idempotency-Key must not be empty".to_string(),
        ));
    }
    if key.len() > MAX_IDEMPOTENCY_KEY_LENGTH {
        return Err(AppError::Validation(format!(
            "Idempotency-Key must be at most {} characters",
            MAX_IDEMPOTENCY_KEY_LENGTH
        )));
    }

    Ok(Some(key.to_string()))
}

fn media_type_from_content_type(content_type: &str) -> &'static str {
    if content_type.starts_with("image/") {
        "image"
    } else if content_type.starts_with("video/") {
        "video"
    } else if content_type.starts_with("audio/") {
        "audio"
    } else {
        "unknown"
    }
}

fn build_status_service(state: &AppState) -> StatusService {
    StatusService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.storage.clone(),
        state.config.server.base_url().to_string(),
    )
}

fn build_account_service(state: &AppState) -> AccountService {
    AccountService::new(state.db.clone(), state.storage.clone())
}

/// Status source response
#[derive(Debug, Serialize)]
struct StatusSourceResponse {
    id: String,
    text: String,
    spoiler_text: String,
}

/// POST /api/v1/statuses
pub async fn create_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    headers: HeaderMap,
    Json(req): Json<CreateStatusRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::data::{EntityId, Status};

    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["POST", "/api/v1/statuses"])
        .start_timer();
    let status_service = build_status_service(&state);

    let idempotency_key = extract_idempotency_key(&headers)?;
    let mut reserved_idempotency_key: Option<String> = None;
    if let Some(key) = idempotency_key.as_deref() {
        if let Some(cached_response) = status_service
            .get_idempotency_response(CREATE_STATUS_IDEMPOTENCY_ENDPOINT, key)
            .await?
        {
            HTTP_REQUESTS_TOTAL
                .with_label_values(&["POST", "/api/v1/statuses", "200"])
                .inc();
            return Ok(Json(cached_response));
        }

        if status_service
            .reserve_idempotency_key(CREATE_STATUS_IDEMPOTENCY_ENDPOINT, key)
            .await?
        {
            reserved_idempotency_key = Some(key.to_string());
        } else {
            let wait_deadline = tokio::time::Instant::now()
                + tokio::time::Duration::from_millis(IDEMPOTENCY_PENDING_WAIT_TIMEOUT_MS);
            loop {
                if let Some(cached_response) = status_service
                    .get_idempotency_response(CREATE_STATUS_IDEMPOTENCY_ENDPOINT, key)
                    .await?
                {
                    HTTP_REQUESTS_TOTAL
                        .with_label_values(&["POST", "/api/v1/statuses", "200"])
                        .inc();
                    return Ok(Json(cached_response));
                }
                if status_service
                    .reserve_idempotency_key(CREATE_STATUS_IDEMPOTENCY_ENDPOINT, key)
                    .await?
                {
                    reserved_idempotency_key = Some(key.to_string());
                    break;
                }
                if tokio::time::Instant::now() >= wait_deadline {
                    return Err(AppError::Unprocessable(
                        "request with the same Idempotency-Key is still being processed"
                            .to_string(),
                    ));
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(
                    IDEMPOTENCY_PENDING_RETRY_DELAY_MS,
                ))
                .await;
            }
        }
    }

    let response_result: Result<serde_json::Value, AppError> = async {
        let account_service = build_account_service(&state);
        let status_service = build_status_service(&state);

        // Get account
        let db_timer = DB_QUERY_DURATION_SECONDS
            .with_label_values(&["SELECT", "accounts"])
            .start_timer();
        let account = account_service.get_account().await?;
        DB_QUERIES_TOTAL
            .with_label_values(&["SELECT", "accounts"])
            .inc();
        db_timer.observe_duration();

        let CreateStatusRequest {
            status,
            media_ids,
            in_reply_to_id,
            quoted_status_id,
            poll,
            scheduled_at,
            sensitive: _,
            spoiler_text,
            visibility,
            language,
        } = req;

        let visibility = normalize_visibility_input(visibility)?;
        let poll = normalize_poll_input(poll)?;
        let scheduled_at = normalize_scheduled_at(scheduled_at)?;
        let media_ids = media_ids.unwrap_or_default();

        if poll.is_some() && !media_ids.is_empty() {
            return Err(AppError::Unprocessable(
                "poll and media_ids cannot be used together".to_string(),
            ));
        }

        if quoted_status_id
            .as_deref()
            .is_some_and(|quoted_status_id| !quoted_status_id.trim().is_empty())
        {
            return Err(AppError::Validation(
                "quoted_status_id parameter is not supported".to_string(),
            ));
        }

        let content = status.unwrap_or_default().trim().to_string();
        let has_textual_payload = !content.is_empty();
        if !has_textual_payload && media_ids.is_empty() && poll.is_none() {
            return Err(AppError::Validation(
                "one of status, media_ids, or poll is required".to_string(),
            ));
        }

        // Resolve reply target if provided.
        let mut in_reply_to_uri = None;
        let mut reply_target_account_address = None;
        let mut persisted_reason = "own".to_string();
        if let Some(in_reply_to_id) = in_reply_to_id.as_deref() {
            if let Some(reply_target) = status_service.find(in_reply_to_id).await? {
                in_reply_to_uri = Some(reply_target.uri.clone());
                if reply_target.is_local {
                    persisted_reason = "reply_to_own".to_string();
                } else if !reply_target.account_address.is_empty() {
                    reply_target_account_address = Some(reply_target.account_address);
                }
            } else if let Some(reply_target) = status_service.find_by_uri(in_reply_to_id).await? {
                in_reply_to_uri = Some(reply_target.uri.clone());
                if reply_target.is_local {
                    persisted_reason = "reply_to_own".to_string();
                } else if !reply_target.account_address.is_empty() {
                    reply_target_account_address = Some(reply_target.account_address);
                }
            } else if let Some(cached_target) =
                state.timeline_cache.get_by_uri(in_reply_to_id).await
            {
                in_reply_to_uri = Some(cached_target.uri.clone());
                if !cached_target.account_address.is_empty() {
                    reply_target_account_address = Some(cached_target.account_address.clone());
                }
            } else {
                return Err(AppError::Validation(
                    "in_reply_to_id does not exist".to_string(),
                ));
            }
        }

        if let Some(scheduled_at) = scheduled_at {
            let media_ids_json = if media_ids.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&media_ids).map_err(|error| {
                    AppError::Internal(anyhow::anyhow!(
                        "failed to serialize scheduled media_ids: {error}"
                    ))
                })?)
            };
            let poll_options_json = match &poll {
                Some(poll) => Some(serde_json::to_string(&poll.options).map_err(|error| {
                    AppError::Internal(anyhow::anyhow!(
                        "failed to serialize scheduled poll options: {error}"
                    ))
                })?),
                None => None,
            };

            let scheduled_id = status_service
                .create_scheduled_status(
                    &scheduled_at,
                    &content,
                    &visibility,
                    spoiler_text.as_deref(),
                    in_reply_to_id.as_deref(),
                    media_ids_json.as_deref(),
                    poll_options_json.as_deref(),
                    poll.as_ref().map(|poll| poll.expires_in),
                    poll.as_ref().is_some_and(|poll| poll.multiple),
                )
                .await?;
            return status_service
                .get_scheduled_status(&scheduled_id)
                .await?
                .ok_or(AppError::NotFound);
        }

        let status_id = EntityId::new().0;
        let uri = format!(
            "{}/users/{}/statuses/{}",
            state.config.server.base_url(),
            account.username,
            status_id
        );

        let status = Status {
            id: status_id.clone(),
            uri: uri.clone(),
            content: format!("<p>{}</p>", html_escape::encode_text(&content)),
            content_warning: spoiler_text.clone(),
            visibility: visibility.clone(),
            language: language.or(Some("en".to_string())),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri,
            boost_of_uri: None,
            persisted_reason,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let should_federate_create = should_federate_to_followers(&status.visibility);
        let create_delivery_targets = if should_federate_create {
            match account_service.get_follower_inboxes().await {
                Ok(follower_inboxes) => follower_inboxes,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "Skipping follower fan-out prefetch for Create delivery"
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        // Save to database
        let db_timer = DB_QUERY_DURATION_SECONDS
            .with_label_values(&["INSERT", "statuses"])
            .start_timer();
        status_service
            .persist_local_status_with_media_and_poll(
                &status,
                &media_ids,
                poll.as_ref()
                    .map(|poll| (poll.options.as_slice(), poll.expires_in, poll.multiple)),
            )
            .await?;
        DB_QUERIES_TOTAL
            .with_label_values(&["INSERT", "statuses"])
            .inc();
        db_timer.observe_duration();

        // Update posts total metric
        POSTS_TOTAL.inc();

        if should_federate_create {
            let delivery = build_delivery(&state, &account);
            let state_for_delivery = state.clone();
            let status_for_delivery = status.clone();
            let reply_target_account_address_for_delivery = reply_target_account_address;
            spawn_best_effort_batch_delivery("create_status", async move {
                let mut delivery_targets = create_delivery_targets;

                if let Some(reply_target_account_address) =
                    reply_target_account_address_for_delivery
                {
                    match resolve_remote_actor_and_inbox(
                        &state_for_delivery,
                        &reply_target_account_address,
                    )
                    .await
                    {
                        Ok((_, reply_target_inbox_uri)) => {
                            delivery_targets.push(reply_target_inbox_uri);
                        }
                        Err(error) => {
                            tracing::warn!(
                                reply_target_account_address = %reply_target_account_address,
                                %error,
                                "Failed to resolve reply target inbox for Create delivery"
                            );
                        }
                    }
                }

                if delivery_targets.is_empty() {
                    tracing::debug!(
                        "Skipping outbound Create delivery because no targets were found"
                    );
                    return Vec::new();
                }

                delivery
                    .send_create(&status_for_delivery, delivery_targets)
                    .await
            });
        } else if !should_federate_create {
            tracing::debug!(
                visibility = %status.visibility,
                "Skipping outbound Create delivery for non-public visibility"
            );
        }

        let media_attachments_value = if !media_ids.is_empty() {
            let media_attachments = status_service.get_media_by_status(&status.id).await?;
            let values: Vec<serde_json::Value> = media_attachments
                .into_iter()
                .map(|attachment| {
                    let media_url = format!(
                        "{}/{}",
                        state.config.storage.media.public_url, attachment.s3_key
                    );
                    let preview_url = attachment
                        .thumbnail_s3_key
                        .as_ref()
                        .map(|key| format!("{}/{}", state.config.storage.media.public_url, key))
                        .unwrap_or_else(|| media_url.clone());
                    serde_json::json!({
                        "id": attachment.id,
                        "type": media_type_from_content_type(&attachment.content_type),
                        "url": media_url,
                        "preview_url": preview_url,
                        "remote_url": serde_json::Value::Null,
                        "text_url": serde_json::Value::Null,
                        "meta": serde_json::Value::Null,
                        "description": attachment.description,
                        "blurhash": attachment.blurhash,
                    })
                })
                .collect();
            Some(serde_json::Value::Array(values))
        } else {
            None
        };

        let poll_value = if poll.is_some() {
            if let Some((poll_id, expires_at, expired, multiple, votes_count, voters_count)) =
                status_service.get_poll_by_status_id(&status.id).await?
            {
                let options = status_service.get_poll_options(&poll_id).await?;
                let options_response: Vec<serde_json::Value> = options
                    .into_iter()
                    .map(|(_, title, option_votes_count)| {
                        serde_json::json!({
                            "title": title,
                            "votes_count": option_votes_count,
                        })
                    })
                    .collect();
                Some(serde_json::json!({
                    "id": poll_id,
                    "expires_at": expires_at,
                    "expired": expired,
                    "multiple": multiple,
                    "votes_count": votes_count,
                    "voters_count": voters_count,
                    "voted": false,
                    "own_votes": [],
                    "options": options_response,
                    "emojis": [],
                }))
            } else {
                None
            }
        } else {
            None
        };

        let response = crate::api::status_to_response(
            &status,
            &account,
            &state.config,
            Some(false),
            Some(false),
            Some(false),
        );
        let mut response_value = serde_json::to_value(response).map_err(|error| {
            AppError::Internal(anyhow::anyhow!(
                "failed to serialize status response: {error}"
            ))
        })?;
        if let Some(obj) = response_value.as_object_mut() {
            if let Some(media_attachments_value) = media_attachments_value {
                obj.insert("media_attachments".to_string(), media_attachments_value);
            }
            if let Some(poll_value) = poll_value {
                obj.insert("poll".to_string(), poll_value);
            }
        }
        Ok(response_value)
    }
    .await;

    let response_value = match response_result {
        Ok(response_value) => response_value,
        Err(error) => {
            if let Some(key) = reserved_idempotency_key.as_deref() {
                let _ = status_service
                    .clear_pending_idempotency_key(CREATE_STATUS_IDEMPOTENCY_ENDPOINT, key)
                    .await;
            }
            return Err(error);
        }
    };

    if let Some(key) = reserved_idempotency_key.as_deref() {
        status_service
            .store_idempotency_response(CREATE_STATUS_IDEMPOTENCY_ENDPOINT, key, &response_value)
            .await?;
    }

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["POST", "/api/v1/statuses", "200"])
        .inc();

    Ok(Json(response_value))
}

/// GET /api/v1/statuses/:id
pub async fn get_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["GET", "/api/v1/statuses/:id"])
        .start_timer();

    let status_service = build_status_service(&state);

    // Get status from database
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "statuses"])
        .start_timer();
    let status = status_service.get(&id).await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "statuses"])
        .inc();
    db_timer.observe_duration();
    ensure_public_visibility_for_public_endpoint(&status.visibility)?;

    // Get account
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "accounts"])
        .start_timer();
    let account = build_account_service(&state).get_account().await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "accounts"])
        .inc();
    db_timer.observe_duration();

    // Check if favourited/reblogged/bookmarked
    let favourited = status_service.is_favourited(&id).await.ok();
    let reblogged = status_service.is_reposted(&id).await.ok();
    let bookmarked = status_service.is_bookmarked(&id).await.ok();

    // Convert to API response
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        favourited,
        reblogged,
        bookmarked,
    );

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/v1/statuses/:id", "200"])
        .inc();

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// DELETE /api/v1/statuses/:id
pub async fn delete_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["DELETE", "/api/v1/statuses/:id"])
        .start_timer();

    let status_service = build_status_service(&state);

    // Get status to verify it exists and is local
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "statuses"])
        .start_timer();
    let status = status_service.get(&id).await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "statuses"])
        .inc();
    db_timer.observe_duration();

    // Get account for response
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "accounts"])
        .start_timer();
    let account = build_account_service(&state).get_account().await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "accounts"])
        .inc();
    db_timer.observe_duration();

    // Delete the status
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["DELETE", "statuses"])
        .start_timer();
    status_service.delete_loaded(&status).await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["DELETE", "statuses"])
        .inc();
    db_timer.observe_duration();

    let should_federate_delete = should_federate_to_followers(&status.visibility);
    if should_federate_delete {
        match build_account_service(&state).get_follower_inboxes().await {
            Ok(follower_inboxes) if !follower_inboxes.is_empty() => {
                let delivery = build_delivery(&state, &account);
                let status_uri = status.uri.clone();
                let status_visibility = status.visibility.clone();
                spawn_best_effort_batch_delivery("delete_status", async move {
                    delivery
                        .send_delete(&status_uri, &status_visibility, follower_inboxes)
                        .await
                });
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    %error,
                    "Skipping outbound Delete delivery because follower inbox lookup failed"
                );
            }
        }
    } else if !should_federate_delete {
        tracing::debug!(
            visibility = %status.visibility,
            "Skipping outbound Delete delivery for non-public visibility"
        );
    }

    // Update posts total metric
    POSTS_TOTAL.dec();

    // Return the deleted status
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        Some(false),
        Some(false),
        Some(false),
    );

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["DELETE", "/api/v1/statuses/:id", "200"])
        .inc();

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/statuses/:id/context
pub async fn get_status_context(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::ContextResponse;
    let status_service = build_status_service(&state);

    // Get the status to verify it exists
    let status = status_service.get(&id).await?;
    ensure_public_visibility_for_public_endpoint(&status.visibility)?;

    // Get account
    let _account = build_account_service(&state).get_account().await?;

    // TODO: Implement proper reply tree traversal
    // For now, return empty ancestors and descendants
    // In a full implementation, we would:
    // 1. Traverse up the reply chain to get ancestors
    // 2. Query for statuses that reply to this one for descendants

    let context = ContextResponse {
        ancestors: vec![],
        descendants: vec![],
    };

    Ok(Json(serde_json::to_value(context).unwrap()))
}

/// GET /api/v1/statuses/:id/reblogged_by
pub async fn get_reblogged_by(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(_params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let status_service = build_status_service(&state);

    // Get the status to verify it exists
    let status = status_service.get(&id).await?;
    ensure_public_visibility_for_public_endpoint(&status.visibility)?;

    // For single-user instance, only the owner can reblog
    // In a full implementation, we would query the reposts table
    // and fetch account information for each user who reblogged

    // For now, return empty array as we don't track individual rebloggers
    Ok(Json(vec![]))
}

/// GET /api/v1/statuses/:id/favourited_by
pub async fn get_favourited_by(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(_params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let status_service = build_status_service(&state);

    // Get the status to verify it exists
    let status = status_service.get(&id).await?;
    ensure_public_visibility_for_public_endpoint(&status.visibility)?;

    // For single-user instance, only the owner can favourite
    // Check if the status is favourited by the owner
    let is_favourited = status_service.is_favourited(&id).await?;

    if is_favourited {
        // Return the owner's account
        let account = build_account_service(&state).get_account().await?;

        let account_response = crate::api::account_to_response(&account, &state.config);
        Ok(Json(vec![serde_json::to_value(account_response).unwrap()]))
    } else {
        // Not favourited, return empty array
        Ok(Json(vec![]))
    }
}

/// GET /api/v1/statuses/:id/source
pub async fn get_status_source(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get the status
    let status = status_service.get(&id).await?;

    // Only allow getting source for local statuses
    if !status.is_local {
        return Err(AppError::Forbidden);
    }

    // Return the source
    let source = StatusSourceResponse {
        id: status.id.clone(),
        text: status.content.clone(),
        spoiler_text: status.content_warning.unwrap_or_default(),
    };

    Ok(Json(serde_json::to_value(source).unwrap()))
}

/// POST /api/v1/statuses/:id/favourite
pub async fn favourite_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<StatusActionParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get account
    let account = build_account_service(&state).get_account().await?;

    // Get status and add favourite.
    let (status, favourite_id) = if let Some(uri) = resolve_action_uri(&id, &params)? {
        status_service.favourite_with_id(uri).await?
    } else {
        status_service.favourite_by_id_with_id(&id).await?
    };
    let status_id = status.id.clone();

    if !status.is_local && !status.account_address.is_empty() {
        let state_for_delivery = state.clone();
        let account_for_delivery = account.clone();
        let account_address_for_delivery = status.account_address.clone();
        let like_activity_uri = format!(
            "{}/like/{}",
            local_actor_uri(&state, &account.username),
            favourite_id
        );
        let status_uri = status.uri.clone();
        spawn_best_effort_delivery("favourite_status", async move {
            let (_, target_inbox_uri) =
                resolve_remote_actor_and_inbox(&state_for_delivery, &account_address_for_delivery)
                    .await?;
            let delivery = build_delivery(&state_for_delivery, &account_for_delivery);
            delivery
                .send_like_with_id(&like_activity_uri, &status_uri, &target_inbox_uri)
                .await
        });
    }

    // Return status with favourited=true
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        Some(true),
        Some(false),
        status_service.is_bookmarked(&status_id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unfavourite
pub async fn unfavourite_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<StatusActionParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get account
    let account = build_account_service(&state).get_account().await?;

    let status = if let Some(uri) = resolve_action_uri(&id, &params)? {
        status_service.get_by_uri(uri).await?
    } else {
        status_service.get(&id).await?
    };
    let like_activity_uri =
        status_service
            .get_favourite_id(&status.id)
            .await?
            .map(|favourite_id| {
                format!(
                    "{}/like/{}",
                    local_actor_uri(&state, &account.username),
                    favourite_id
                )
            });
    status_service.unfavourite_loaded(&status).await?;
    let status_id = status.id.clone();

    if let Some(like_activity_uri) = like_activity_uri {
        if !status.is_local && !status.account_address.is_empty() {
            let state_for_delivery = state.clone();
            let account_for_delivery = account.clone();
            let account_address_for_delivery = status.account_address.clone();
            spawn_best_effort_delivery("unfavourite_status", async move {
                let (_, target_inbox_uri) = resolve_remote_actor_and_inbox(
                    &state_for_delivery,
                    &account_address_for_delivery,
                )
                .await?;
                let delivery = build_delivery(&state_for_delivery, &account_for_delivery);
                delivery
                    .send_undo_to_inbox_with_type(
                        &like_activity_uri,
                        Some("Like"),
                        &target_inbox_uri,
                    )
                    .await
            });
        }
    }

    // Return status with favourited=false
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        Some(false),
        Some(false),
        status_service.is_bookmarked(&status_id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/reblog
pub async fn reblog_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<StatusActionParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::data::EntityId;

    let status_service = build_status_service(&state);

    // Get account
    let account = build_account_service(&state).get_account().await?;

    let action_uri = resolve_action_uri(&id, &params)?;
    let (status, repost_uri) = if let Some(uri) = action_uri {
        let status = status_service.repost(uri).await?;
        let repost_uri = status_service
            .get_repost_uri(&status.id)
            .await?
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "repost URI missing after creating repost activity"
                ))
            })?;
        (status, repost_uri)
    } else {
        // Create repost record
        let repost_id = EntityId::new().0;
        let repost_uri = format!(
            "{}/users/{}/statuses/{}/activity",
            state.config.server.base_url(),
            account.username,
            repost_id
        );
        let status = status_service.repost_by_id(&id, &repost_uri).await?;
        (status, repost_uri)
    };
    let status_id = status.id.clone();

    let should_federate_reblog = should_federate_to_followers(&status.visibility);
    if should_federate_reblog {
        match build_account_service(&state).get_follower_inboxes().await {
            Ok(follower_inboxes) if !follower_inboxes.is_empty() => {
                let delivery = build_delivery(&state, &account);
                let announce_activity_uri = repost_uri.clone();
                let announced_status_uri = status.uri.clone();
                let announced_status_visibility = status.visibility.clone();
                spawn_best_effort_batch_delivery("reblog_status", async move {
                    delivery
                        .send_announce_with_id(
                            &announce_activity_uri,
                            &announced_status_uri,
                            &announced_status_visibility,
                            follower_inboxes,
                        )
                        .await
                });
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    %error,
                    "Skipping outbound Announce delivery because follower inbox lookup failed"
                );
            }
        }
    } else if !should_federate_reblog {
        tracing::debug!(
            visibility = %status.visibility,
            "Skipping outbound Announce delivery for non-public visibility"
        );
    }

    // Return the original status with reblogged=true
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        status_service.is_favourited(&status_id).await.ok(),
        Some(true),
        status_service.is_bookmarked(&status_id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unreblog
pub async fn unreblog_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<StatusActionParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get account
    let account = build_account_service(&state).get_account().await?;

    let action_uri = resolve_action_uri(&id, &params)?;
    let status = if let Some(uri) = action_uri {
        status_service.get_by_uri(uri).await?
    } else {
        status_service.get(&id).await?
    };
    let repost_uri = status_service.get_repost_uri(&status.id).await?;
    if let Some(uri) = action_uri {
        status_service.unrepost(uri).await?;
    } else {
        status_service.unrepost_by_id(&id).await?;
    }
    let status_id = status.id.clone();

    if let Some(repost_uri) = repost_uri {
        let should_federate_unreblog = should_federate_to_followers(&status.visibility);
        if should_federate_unreblog {
            match build_account_service(&state).get_follower_inboxes().await {
                Ok(follower_inboxes) if !follower_inboxes.is_empty() => {
                    let delivery = build_delivery(&state, &account);
                    spawn_best_effort_batch_delivery("unreblog_status", async move {
                        delivery
                            .send_undo_with_type(&repost_uri, Some("Announce"), follower_inboxes)
                            .await
                    });
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "Skipping outbound Undo(Announce) delivery because follower inbox lookup failed"
                    );
                }
            }
        } else if !should_federate_unreblog {
            tracing::debug!(
                visibility = %status.visibility,
                "Skipping outbound Undo(Announce) delivery for non-public visibility"
            );
        }
    }

    // Return status with reblogged=false
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        status_service.is_favourited(&status_id).await.ok(),
        Some(false),
        status_service.is_bookmarked(&status_id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/bookmark
pub async fn bookmark_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<StatusActionParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get account
    let account = build_account_service(&state).get_account().await?;

    // Get status and add bookmark.
    let status = if let Some(uri) = resolve_action_uri(&id, &params)? {
        status_service.bookmark(uri).await?
    } else {
        status_service.bookmark_by_id(&id).await?
    };
    let status_id = status.id.clone();

    // Return status with bookmarked=true
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        status_service.is_favourited(&status_id).await.ok(),
        Some(false),
        Some(true),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unbookmark
pub async fn unbookmark_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<StatusActionParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get account
    let account = build_account_service(&state).get_account().await?;

    // Get status and remove bookmark.
    let status = if let Some(uri) = resolve_action_uri(&id, &params)? {
        let status = status_service.get_by_uri(uri).await?;
        status_service.unbookmark_loaded(&status).await?;
        status
    } else {
        status_service.unbookmark_by_id(&id).await?
    };
    let status_id = status.id.clone();

    // Return status with bookmarked=false
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        status_service.is_favourited(&status_id).await.ok(),
        Some(false),
        Some(false),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// Update status request
#[derive(Debug, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: Option<String>,
    pub spoiler_text: Option<String>,
    pub sensitive: Option<bool>,
    pub media_ids: Option<Vec<String>>,
}

/// PUT /api/v1/statuses/:id
/// Edit an existing status
///
/// Note: For simplicity in single-user instance, this creates a new version
/// without preserving full edit history.
pub async fn update_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateStatusRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get the status
    let mut status = status_service.get(&id).await?;

    // Only allow editing local statuses
    if !status.is_local {
        return Err(AppError::Forbidden);
    }

    // Get account
    let account = build_account_service(&state).get_account().await?;

    // Update fields if provided
    if let Some(content) = req.status {
        if !content.is_empty() {
            status.content = format!("<p>{}</p>", html_escape::encode_text(&content));
        }
    }

    if let Some(spoiler_text) = req.spoiler_text {
        status.content_warning = Some(spoiler_text);
    }

    // TODO: Handle media_ids updates
    // For now, we skip media updates as it requires more complex logic

    // Save updated status
    // Note: In a full implementation, we would create a new version in an edit_history table.
    status_service.update_loaded(&status).await?;

    // Return updated status
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        status_service.is_favourited(&id).await.ok(),
        Some(false),
        status_service.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/statuses/:id/history
/// Get edit history for a status
///
/// For single-user instance without full edit history tracking,
/// this returns only the current version.
pub async fn get_status_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let status_service = build_status_service(&state);

    // Get the status
    let status = status_service.get(&id).await?;

    // Get account
    let account = build_account_service(&state).get_account().await?;

    // For now, return only the current version
    // In a full implementation, we would query an edit_history table
    let current_version = serde_json::json!({
        "content": status.content,
        "spoiler_text": status.content_warning.unwrap_or_default(),
        "sensitive": false,
        "created_at": status.created_at.to_rfc3339(),
        "account": crate::api::account_to_response(&account, &state.config),
    });

    Ok(Json(vec![current_version]))
}

/// POST /api/v1/statuses/:id/pin
/// Pin a status to profile
///
/// For single-user instance, this is a no-op that returns success.
/// Pinned statuses are not currently tracked in the database.
pub async fn pin_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get status
    let status = status_service.get(&id).await?;

    // Only allow pinning local statuses
    if !status.is_local {
        return Err(AppError::Validation(
            "Can only pin own statuses".to_string(),
        ));
    }

    // Get account
    let account = build_account_service(&state).get_account().await?;

    // TODO: Store pinned status in database
    // For now, just return the status with pinned=true
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        status_service.is_favourited(&id).await.ok(),
        Some(false),
        status_service.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unpin
/// Unpin a status from profile
///
/// For single-user instance, this is a no-op that returns success.
pub async fn unpin_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get status
    let status = status_service.get(&id).await?;

    // Get account
    let account = build_account_service(&state).get_account().await?;

    // TODO: Remove pinned status from database
    // For now, just return the status with pinned=false
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        status_service.is_favourited(&id).await.ok(),
        Some(false),
        status_service.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/mute
/// Mute notifications from a conversation
///
/// For single-user instance, this is a no-op that returns success.
/// Conversation muting is not currently tracked.
pub async fn mute_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get status
    let status = status_service.get(&id).await?;

    // Get account
    let account = build_account_service(&state).get_account().await?;

    // TODO: Store muted conversation in database
    // For now, just return the status with muted=true
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        status_service.is_favourited(&id).await.ok(),
        Some(false),
        status_service.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unmute
/// Unmute notifications from a conversation
///
/// For single-user instance, this is a no-op that returns success.
pub async fn unmute_status(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get status
    let status = status_service.get(&id).await?;

    // Get account
    let account = build_account_service(&state).get_account().await?;

    // TODO: Remove muted conversation from database
    // For now, just return the status with muted=false
    let response = crate::api::status_to_response(
        &status,
        &account,
        &state.config,
        status_service.is_favourited(&id).await.ok(),
        Some(false),
        status_service.is_bookmarked(&id).await.ok(),
    );

    Ok(Json(serde_json::to_value(response).unwrap()))
}
