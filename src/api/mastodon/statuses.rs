//! Status endpoints

use axum::{
    body::to_bytes,
    extract::Request,
    extract::{Path, Query, State},
    http::{
        HeaderMap,
        header::{CONTENT_TYPE, LINK},
    },
    response::{IntoResponse, Json},
};
use axum_extra::extract::CookieJar;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

use super::accounts::PaginationParams;
use super::federation_delivery::{
    ResolvedRemoteRecipient, extract_mentions_from_content, extract_remote_mentions_from_content,
    resolve_remote_actor_and_inbox_with_dependencies, resolve_remote_recipients_with_dependencies,
    spawn_best_effort_batch_delivery, spawn_best_effort_delivery,
};
use crate::StatusApiState;
use crate::auth::CurrentUser;
use crate::data::{Account, PersistedReason, ScheduledStatusInsert, StatusVisibility};
use crate::error::AppError;
use crate::metrics::{
    DB_QUERIES_TOTAL, DB_QUERY_DURATION_SECONDS, HTTP_REQUEST_DURATION_SECONDS,
    HTTP_REQUESTS_TOTAL, POSTS_TOTAL,
};
use crate::service::{AccountService, StatusService};

const DEFAULT_VISIBILITY: StatusVisibility = StatusVisibility::Public;
const CREATE_STATUS_IDEMPOTENCY_ENDPOINT: &str = "/api/v1/statuses";
const MAX_IDEMPOTENCY_KEY_LENGTH: usize = 256;
const MIN_POLL_OPTIONS: usize = 2;
const MAX_POLL_OPTIONS: usize = 4;
const MAX_POLL_OPTION_CHARS: usize = 50;
const MIN_POLL_EXPIRES_IN_SECONDS: i64 = 300;
const MAX_POLL_EXPIRES_IN_SECONDS: i64 = 2_629_746;
const IDEMPOTENCY_PENDING_WAIT_TIMEOUT_MS: u64 = 5_000;
const IDEMPOTENCY_PENDING_RETRY_DELAY_MS: u64 = 50;
const MAX_STATUS_CONTEXT_ANCESTORS: usize = 40;
const MAX_STATUS_CONTEXT_DESCENDANTS: usize = 40;

fn paginate_account_values(
    mut accounts: Vec<serde_json::Value>,
    params: &PaginationParams,
    limit: usize,
) -> Vec<serde_json::Value> {
    accounts.sort_by(|left, right| {
        let left_id = left
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let right_id = right
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        right_id.cmp(left_id)
    });
    accounts.dedup_by(|left, right| {
        left.get("id").and_then(|value| value.as_str())
            == right.get("id").and_then(|value| value.as_str())
    });

    let mut accounts = accounts
        .into_iter()
        .filter(|account| {
            let id = account
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            params
                .max_id
                .as_deref()
                .map(|cursor| id < cursor)
                .unwrap_or(true)
                && params
                    .min_id
                    .as_deref()
                    .map(|cursor| id > cursor)
                    .unwrap_or(true)
                && params
                    .since_id
                    .as_deref()
                    .map(|cursor| id > cursor)
                    .unwrap_or(true)
        })
        .take(limit)
        .collect::<Vec<_>>();
    if params.min_id.is_some() {
        accounts.reverse();
    }
    accounts
}

fn interaction_account_link_header(
    path: &str,
    limit: usize,
    first_id: Option<&str>,
    last_id: Option<&str>,
) -> Option<String> {
    let build_path = |cursor_key: &str, cursor_value: &str| {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        serializer.append_pair("limit", &limit.to_string());
        serializer.append_pair(cursor_key, cursor_value);
        format!("{path}?{}", serializer.finish())
    };

    let mut links = Vec::new();
    if let Some(last_id) = last_id.filter(|value| !value.is_empty()) {
        links.push(format!("<{}>; rel=\"next\"", build_path("max_id", last_id)));
    }
    if let Some(first_id) = first_id.filter(|value| !value.is_empty()) {
        links.push(format!(
            "<{}>; rel=\"prev\"",
            build_path("min_id", first_id)
        ));
    }
    (!links.is_empty()).then(|| links.join(", "))
}

#[derive(Debug, Default, Deserialize)]
pub struct CreateStatusPollRequest {
    #[serde(default)]
    pub options: Vec<String>,
    pub expires_in: i64,
    #[serde(default)]
    pub multiple: bool,
    #[serde(default, rename = "hide_totals")]
    pub hide_totals: bool,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct StatusMediaAttributeRequest {
    pub id: Option<String>,
    pub description: Option<String>,
    pub focus: Option<String>,
}

/// Status creation request
#[derive(Debug, Default, Deserialize)]
pub struct CreateStatusRequest {
    pub status: Option<String>,
    pub media_ids: Option<Vec<String>>,
    #[serde(default)]
    pub media_attributes: Vec<StatusMediaAttributeRequest>,
    pub in_reply_to_id: Option<String>,
    pub quoted_status_id: Option<String>,
    pub poll: Option<CreateStatusPollRequest>,
    pub scheduled_at: Option<String>,
    #[serde(rename = "sensitive")]
    pub sensitive: Option<bool>,
    pub spoiler_text: Option<String>,
    pub visibility: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct StatusActionParams {
    pub uri: Option<String>,
    pub visibility: Option<String>,
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

fn should_federate_to_followers(visibility: StatusVisibility) -> bool {
    matches!(
        visibility,
        StatusVisibility::Public | StatusVisibility::Unlisted
    )
}

fn should_deliver_to_followers_collection(visibility: StatusVisibility) -> bool {
    matches!(
        visibility,
        StatusVisibility::Public | StatusVisibility::Unlisted | StatusVisibility::Private
    )
}

fn ensure_public_visibility_for_public_endpoint(
    visibility: StatusVisibility,
) -> Result<(), AppError> {
    if should_federate_to_followers(visibility) {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}

async fn request_is_authenticated(
    state: &StatusApiState,
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
            .filter(|token| {
                matches!(
                    token.grant_type.as_str(),
                    "authorization_code" | "refresh_token"
                )
            })
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

fn normalize_visibility_input(
    raw_visibility: Option<String>,
) -> Result<StatusVisibility, AppError> {
    let visibility = match raw_visibility {
        Some(value) => StatusVisibility::parse(&value),
        None => Some(DEFAULT_VISIBILITY),
    };

    visibility.ok_or_else(|| {
        AppError::Validation(
            "visibility must be one of: public, unlisted, private, direct".to_string(),
        )
    })
}

fn normalize_optional_identifier(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

#[derive(Debug)]
struct ResolvedQuoteTarget {
    uri: String,
    remote_account_address: Option<String>,
}

async fn resolve_quote_target_uri(
    state: &StatusApiState,
    status_service: &StatusService,
    quoted_status_id: Option<&str>,
) -> Result<Option<ResolvedQuoteTarget>, AppError> {
    let Some(quoted_status_id) = quoted_status_id else {
        return Ok(None);
    };

    if let Some(quoted_target) = status_service.find(quoted_status_id).await? {
        return Ok(Some(ResolvedQuoteTarget {
            uri: quoted_target.uri,
            remote_account_address: (!quoted_target.is_local
                && !quoted_target.account_address.is_empty())
            .then_some(quoted_target.account_address),
        }));
    }
    if let Some(quoted_target) = status_service.find_by_uri(quoted_status_id).await? {
        return Ok(Some(ResolvedQuoteTarget {
            uri: quoted_target.uri,
            remote_account_address: (!quoted_target.is_local
                && !quoted_target.account_address.is_empty())
            .then_some(quoted_target.account_address),
        }));
    }
    if let Some(cached_target) = state.timeline_cache.get_by_uri(quoted_status_id).await {
        return Ok(Some(ResolvedQuoteTarget {
            uri: cached_target.uri.clone(),
            remote_account_address: (!cached_target.account_address.is_empty())
                .then_some(cached_target.account_address.clone()),
        }));
    }
    if quoted_status_id.starts_with("http://") || quoted_status_id.starts_with("https://") {
        let quoted_target = status_service
            .ensure_remote_status_persisted(quoted_status_id, PersistedReason::Own)
            .await?;
        return Ok(Some(ResolvedQuoteTarget {
            uri: quoted_target.uri,
            remote_account_address: (!quoted_target.account_address.is_empty())
                .then_some(quoted_target.account_address),
        }));
    }

    Err(AppError::Validation(
        "quoted_status_id does not exist".to_string(),
    ))
}

#[derive(Debug)]
struct NormalizedCreatePoll {
    options: Vec<String>,
    expires_in: i64,
    multiple: bool,
    hide_totals: bool,
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
        hide_totals: poll.hide_totals,
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

fn normalize_content_warning(
    spoiler_text: Option<String>,
    sensitive: Option<bool>,
    current: Option<&str>,
) -> Option<String> {
    match (spoiler_text, sensitive) {
        (Some(text), _) if !text.is_empty() => Some(text),
        (Some(_), Some(true)) => Some(String::new()),
        (Some(_), _) => None,
        (None, Some(true)) => current
            .map(ToOwned::to_owned)
            .or_else(|| Some(String::new())),
        (None, Some(false)) => None,
        (None, None) => current.map(ToOwned::to_owned),
    }
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

fn build_status_service(state: &StatusApiState) -> StatusService {
    StatusService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.storage.clone(),
        state.streaming_event_bus.clone(),
        state.config.server.base_url().to_string(),
        state.config.auth.username.clone(),
    )
}

fn build_account_service(state: &StatusApiState) -> AccountService {
    AccountService::new(state.db.clone(), state.storage.clone())
}

fn local_actor_uri(state: &StatusApiState, username: &str) -> String {
    crate::federation::local_actor_uri(&state.config.server.base_url(), username)
}

fn build_delivery(
    state: &StatusApiState,
    account: &Account,
) -> crate::federation::ActivityDelivery {
    crate::federation::build_local_delivery(
        state.http_client.clone(),
        &state.config.server.base_url(),
        account,
    )
    .with_media_storage(state.storage.clone())
}

async fn status_response_without_interaction_state(
    state: &StatusApiState,
    account: &crate::data::Account,
    account_stats: crate::api::AccountStats,
    status: &crate::data::Status,
) -> Result<crate::api::StatusResponse, AppError> {
    let remote_account_stats = crate::api::load_remote_account_stats_map(
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        &state.config.server.protocol,
        std::slice::from_ref(status),
    )
    .await?
    .get(status.account_address.trim())
    .cloned();

    crate::api::build_status_response_with_account_stats_and_remote_stats(
        state.db.as_ref(),
        status,
        account,
        &state.config,
        account_stats,
        remote_account_stats,
        crate::api::StatusInteractions::default(),
    )
    .await
}

async fn status_response_with_viewer_interactions(
    state: &StatusApiState,
    account: &crate::data::Account,
    account_stats: crate::api::AccountStats,
    status: &crate::data::Status,
) -> Result<crate::api::StatusResponse, AppError> {
    let remote_account_stats = crate::api::load_remote_account_stats_map(
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        &state.config.server.protocol,
        std::slice::from_ref(status),
    )
    .await?
    .get(status.account_address.trim())
    .cloned();
    let thread_uri = state.db.resolve_thread_root_uri(status).await?;

    crate::api::build_status_response_with_account_stats_and_remote_stats(
        state.db.as_ref(),
        status,
        account,
        &state.config,
        account_stats,
        remote_account_stats,
        crate::api::StatusInteractions::new(
            Some(state.db.is_favourited(&status.id).await?),
            Some(state.db.is_reposted(&status.id).await?),
            Some(state.db.is_thread_muted(&thread_uri).await?),
            Some(state.db.is_bookmarked(&status.id).await?),
            Some(state.db.is_status_pinned(&status.id).await?),
        ),
    )
    .await
}

async fn reblog_wrapper_response(
    state: &StatusApiState,
    account: &crate::data::Account,
    account_stats: crate::api::AccountStats,
    status: &crate::data::Status,
    repost_id: &str,
    repost_uri: &str,
    announce_created_at: chrono::DateTime<chrono::Utc>,
    announce_visibility: crate::data::StatusVisibility,
) -> Result<crate::api::StatusResponse, AppError> {
    let mut reblogged_status =
        status_response_with_viewer_interactions(state, account, account_stats, status).await?;
    reblogged_status.reblogged = true;

    let mut wrapper = reblogged_status.clone();
    wrapper.id = repost_id.to_string();
    wrapper.uri = repost_uri.to_string();
    wrapper.url = repost_uri.to_string();
    wrapper.created_at = announce_created_at;
    wrapper.visibility = announce_visibility.to_string();
    wrapper.account =
        crate::api::account_to_response_with_stats(account, &state.config, account_stats);
    wrapper.content.clear();
    wrapper.text.clear();
    wrapper.media_attachments.clear();
    wrapper.mentions.clear();
    wrapper.tags.clear();
    wrapper.emojis.clear();
    wrapper.poll = None;
    wrapper.quote = None;
    wrapper.quote_approval = None;
    wrapper.card = None;
    wrapper.reblog = Some(Box::new(reblogged_status));
    Ok(wrapper)
}

async fn status_context_response(
    state: &StatusApiState,
    account: &crate::data::Account,
    account_stats: crate::api::AccountStats,
    status: &crate::data::Status,
    is_authenticated: bool,
) -> Result<crate::api::StatusResponse, AppError> {
    if is_authenticated {
        status_response_with_viewer_interactions(state, account, account_stats, status).await
    } else {
        status_response_without_interaction_state(state, account, account_stats, status).await
    }
}

async fn build_status_edit_snapshot_payload(
    state: &StatusApiState,
    account: &crate::data::Account,
    account_stats: crate::api::AccountStats,
    status: &crate::data::Status,
) -> Result<(Option<String>, Option<String>, Option<String>), AppError> {
    let response =
        status_response_with_viewer_interactions(state, account, account_stats, status).await?;
    let media_json = Some(
        serde_json::to_string(&response.media_attachments)
            .map_err(|error| AppError::serialization("status edit media snapshot", error))?,
    );
    let poll_json = response
        .poll
        .as_ref()
        .map(|value| serde_json::to_string(value))
        .transpose()
        .map_err(|error| AppError::serialization("status edit poll snapshot", error))?;
    let quote_json = response
        .quote
        .as_ref()
        .map(|value| serde_json::to_string(value))
        .transpose()
        .map_err(|error| AppError::serialization("status edit quote snapshot", error))?;
    Ok((media_json, poll_json, quote_json))
}

async fn resolve_interaction_account_response(
    state: &StatusApiState,
    local_account: &Account,
    local_account_stats: crate::api::AccountStats,
    identity: &str,
) -> Option<serde_json::Value> {
    if let Some(response) = super::accounts::resolve_account_response_for_identity(
        state.config.as_ref(),
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        Some(state.federation_fetch_client.as_ref()),
        identity,
    )
    .await
    {
        return serde_json::to_value(response).ok();
    }

    (identity.eq_ignore_ascii_case(&local_account.id)
        || identity.eq_ignore_ascii_case(&local_actor_uri(state, &local_account.username))
        || identity.eq_ignore_ascii_case(&local_account.username)
        || identity.eq_ignore_ascii_case(&format!(
            "{}@{}",
            local_account.username, state.config.server.domain
        )))
    .then(|| {
        crate::api::account_to_response_with_stats(
            local_account,
            &state.config,
            local_account_stats,
        )
    })
    .and_then(|response| serde_json::to_value(response).ok())
}

fn status_content_to_source_text(content: &str) -> String {
    // Convert common block separators before tag stripping so paragraph and line
    // boundaries are preserved in source text output.
    let normalized = content
        .replace("<br />", "\n")
        .replace("<br/>", "\n")
        .replace("<br>", "\n")
        .replace("</p>", "\n\n")
        .replace("<p>", "");

    let mut without_tags = String::with_capacity(normalized.len());
    let mut in_tag = false;
    for ch in normalized.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => without_tags.push(ch),
            _ => {}
        }
    }

    let decoded = html_escape::decode_html_entities(without_tags.trim()).into_owned();
    let mut lines = decoded.lines().map(str::trim_end).peekable();
    let mut output = String::new();
    let mut previous_blank = false;
    while let Some(line) = lines.next() {
        let is_blank = line.trim().is_empty();
        if is_blank {
            if !previous_blank && lines.peek().is_some() {
                output.push('\n');
            }
        } else {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(line);
        }
        previous_blank = is_blank;
    }
    output.trim().to_string()
}

async fn remote_account_address_for_status_uri(
    state: &StatusApiState,
    status_service: &StatusService,
    status_uri: Option<&str>,
) -> Result<Option<String>, AppError> {
    let Some(status_uri) = status_uri else {
        return Ok(None);
    };

    if let Some(status) = status_service.find_by_uri(status_uri).await? {
        return Ok((!status.is_local && !status.account_address.is_empty())
            .then_some(status.account_address));
    }
    if let Some(cached_status) = state.timeline_cache.get_by_uri(status_uri).await {
        return Ok((!cached_status.account_address.is_empty())
            .then_some(cached_status.account_address.clone()));
    }

    Ok(None)
}

async fn resolve_explicit_remote_recipients(
    state: &StatusApiState,
    content: &str,
    extra_addresses: impl IntoIterator<Item = String>,
) -> (Vec<ResolvedRemoteRecipient>, Vec<serde_json::Value>) {
    let mention_addresses_all = extract_mentions_from_content(content);
    let mut addresses = extract_remote_mentions_from_content(content, &state.config.server.domain);
    let mention_addresses = addresses
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    addresses.extend(extra_addresses);

    let recipients = resolve_remote_recipients_with_dependencies(
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        state.federation_fetch_client.as_ref(),
        addresses,
    )
    .await;

    let mut mention_tags = recipients
        .iter()
        .filter(|recipient| mention_addresses.contains(&recipient.address))
        .map(|recipient| {
            serde_json::json!({
                "type": "Mention",
                "href": recipient.actor_uri,
                "name": format!("@{}", recipient.address),
            })
        })
        .collect::<Vec<_>>();

    mention_tags.extend(
        mention_addresses_all
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

    (recipients, mention_tags)
}

fn merge_delivery_targets(
    mut follower_inboxes: Vec<String>,
    recipients: &[ResolvedRemoteRecipient],
) -> Vec<String> {
    let mut seen = follower_inboxes.iter().cloned().collect::<HashSet<_>>();
    for recipient in recipients {
        if seen.insert(recipient.inbox_uri.clone()) {
            follower_inboxes.push(recipient.inbox_uri.clone());
        }
    }
    follower_inboxes
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
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    headers: HeaderMap,
    request: Request,
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

    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 256 * 1024)
        .await
        .map_err(|error| AppError::Validation(format!("failed to read request body: {error}")))?;
    let req = if body.is_empty() {
        CreateStatusRequest::default()
    } else {
        parse_create_status_request(&parts.headers, &body)?
    };

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
            media_attributes,
            in_reply_to_id,
            quoted_status_id,
            poll,
            scheduled_at,
            sensitive,
            spoiler_text,
            visibility,
            language,
        } = req;

        let visibility = normalize_visibility_input(visibility)?;
        let poll = normalize_poll_input(poll)?;
        let scheduled_at = normalize_scheduled_at(scheduled_at)?;
        let media_ids = media_ids.unwrap_or_default();
        let quoted_status_id = normalize_optional_identifier(quoted_status_id);
        let content_warning = normalize_content_warning(spoiler_text, sensitive, None);

        if poll.is_some() && !media_ids.is_empty() {
            return Err(AppError::Unprocessable(
                "poll and media_ids cannot be used together".to_string(),
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
        let mut persisted_reason = PersistedReason::Own;
        if let Some(in_reply_to_id) = in_reply_to_id.as_deref() {
            if let Some(reply_target) = status_service.find(in_reply_to_id).await? {
                in_reply_to_uri = Some(reply_target.uri.clone());
                if reply_target.is_local {
                    persisted_reason = PersistedReason::ReplyToOwn;
                } else if !reply_target.account_address.is_empty() {
                    reply_target_account_address = Some(reply_target.account_address);
                }
            } else if let Some(reply_target) = status_service.find_by_uri(in_reply_to_id).await? {
                in_reply_to_uri = Some(reply_target.uri.clone());
                if reply_target.is_local {
                    persisted_reason = PersistedReason::ReplyToOwn;
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

        let quote_target =
            resolve_quote_target_uri(&state, &status_service, quoted_status_id.as_deref()).await?;
        let quote_of_uri = quote_target.as_ref().map(|target| target.uri.clone());

        if let Some(scheduled_at) = scheduled_at {
            let media_ids_json = if media_ids.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&media_ids).map_err(|error| {
                    AppError::serialization("scheduled media_ids serialization", error)
                })?)
            };
            let poll_options_json = match &poll {
                Some(poll) => Some(serde_json::to_string(&poll.options).map_err(|error| {
                    AppError::serialization("scheduled poll options serialization", error)
                })?),
                None => None,
            };

            let scheduled_id = status_service
                .create_scheduled_status(&ScheduledStatusInsert {
                    scheduled_at,
                    status_text: content.clone(),
                    visibility: visibility.to_string(),
                    content_warning: content_warning.clone(),
                    in_reply_to_id: in_reply_to_id.clone(),
                    quoted_status_id: quoted_status_id.clone(),
                    media_ids: media_ids_json,
                    poll_options: poll_options_json,
                    poll_expires_in: poll.as_ref().map(|poll| poll.expires_in),
                    poll_multiple: poll.as_ref().is_some_and(|poll| poll.multiple),
                    language: language.clone(),
                })
                .await?;
            return status_service
                .get_scheduled_status(&scheduled_id)
                .await?
                .ok_or(AppError::NotFound);
        }

        let status_id = EntityId::new_string();
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
            content_warning: content_warning.clone(),
            visibility,
            language: language.or(Some("en".to_string())),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri,
            boost_of_uri: None,
            quote_of_uri,
            persisted_reason,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let should_deliver_to_followers = should_deliver_to_followers_collection(status.visibility);
        let create_delivery_targets = if should_deliver_to_followers {
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

        let mut extra_addresses = Vec::new();
        if let Some(reply_target_account_address) = reply_target_account_address.clone() {
            extra_addresses.push(reply_target_account_address);
        }
        if let Some(quote_target_account_address) = quote_target
            .as_ref()
            .and_then(|target| target.remote_account_address.clone())
        {
            extra_addresses.push(quote_target_account_address);
        }
        let (explicit_recipients, mention_tags) =
            resolve_explicit_remote_recipients(&state, &content, extra_addresses).await;

        // Save to database
        let db_timer = DB_QUERY_DURATION_SECONDS
            .with_label_values(&["INSERT", "statuses"])
            .start_timer();
        status_service
            .persist_local_status_with_media_and_poll(
                &status,
                &media_ids,
                poll.as_ref().map(|poll| {
                    (
                        poll.options.as_slice(),
                        poll.expires_in,
                        poll.multiple,
                        poll.hide_totals,
                    )
                }),
            )
            .await?;
        apply_status_media_attributes(&state, &media_ids, &status.id, &media_attributes).await?;
        DB_QUERIES_TOTAL
            .with_label_values(&["INSERT", "statuses"])
            .inc();
        db_timer.observe_duration();

        // Update posts total metric
        POSTS_TOTAL.inc();

        let delivery_targets =
            merge_delivery_targets(create_delivery_targets, &explicit_recipients);
        if !delivery_targets.is_empty() {
            let delivery = build_delivery(&state, &account);
            let state_for_delivery = state.clone();
            let status_for_delivery = status.clone();
            let explicit_recipient_actor_uris = explicit_recipients
                .iter()
                .map(|recipient| recipient.actor_uri.clone())
                .collect::<Vec<_>>();
            let mention_tags_for_delivery = mention_tags.clone();
            spawn_best_effort_batch_delivery("create_status", async move {
                delivery
                    .queue_create_with_audience(
                        state_for_delivery.db.as_ref(),
                        &status_for_delivery,
                        delivery_targets,
                        &explicit_recipient_actor_uris,
                        &mention_tags_for_delivery,
                    )
                    .await
            });
        } else {
            tracing::debug!(
                visibility = %status.visibility,
                "Skipping outbound Create delivery because no remote targets were found"
            );
        }

        let media_attachments = if !media_ids.is_empty() {
            status_service.get_media_by_status(&status.id).await?
        } else {
            Vec::new()
        };

        let poll_value = if poll.is_some() {
            if let Some((
                poll_id,
                expires_at,
                expired,
                multiple,
                hide_totals,
                votes_count,
                voters_count,
            )) = status_service.get_poll_by_status_id(&status.id).await?
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
                    "hide_totals": hide_totals,
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

        let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
        let response = crate::api::build_status_response_with_media(
            state.db.as_ref(),
            &status,
            &account,
            &state.config,
            account_stats,
            None,
            crate::api::StatusInteractions::new(
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            ),
            &media_attachments,
        )
        .await?;
        let mut response_value = serde_json::to_value(response)
            .map_err(|error| AppError::serialization("status response serialization", error))?;
        if let Some(obj) = response_value.as_object_mut()
            && let Some(poll_value) = poll_value
        {
            obj.insert("poll".to_string(), poll_value);
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
    State(state): State<StatusApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<serde_json::Value>, AppError> {
    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["GET", "/api/v1/statuses/:id"])
        .start_timer();

    let status_service = build_status_service(&state);
    let is_authenticated = request_is_authenticated(&state, &headers, &jar).await;

    // Get status from database
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "statuses"])
        .start_timer();
    let status = if let Some(repost) = state.db.get_repost_by_id(&id).await? {
        let boosted_status = status_service.get(&repost.status_id).await?;
        if !is_authenticated {
            ensure_public_visibility_for_public_endpoint(boosted_status.visibility)?;
        }
        let account = build_account_service(&state).get_account().await?;
        let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
        let wrapper = reblog_wrapper_response(
            &state,
            &account,
            account_stats,
            &boosted_status,
            &repost.id,
            &repost.uri,
            repost.created_at,
            boosted_status.visibility,
        )
        .await?;
        HTTP_REQUESTS_TOTAL
            .with_label_values(&["GET", "/api/v1/statuses/:id", "200"])
            .inc();
        return Ok(Json(serde_json::to_value(wrapper).unwrap()));
    } else {
        status_service.get(&id).await?
    };
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "statuses"])
        .inc();
    db_timer.observe_duration();
    if !is_authenticated {
        ensure_public_visibility_for_public_endpoint(status.visibility)?;
    }

    // Get account
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "accounts"])
        .start_timer();
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "accounts"])
        .inc();
    db_timer.observe_duration();

    // Convert to API response
    let response = if is_authenticated {
        status_response_with_viewer_interactions(&state, &account, account_stats, &status).await?
    } else {
        status_response_without_interaction_state(&state, &account, account_stats, &status).await?
    };

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/v1/statuses/:id", "200"])
        .inc();

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/statuses/:id/card
pub async fn get_status_card(
    State(state): State<StatusApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<serde_json::Value>, AppError> {
    let status = build_status_service(&state).get(&id).await?;
    if !request_is_authenticated(&state, &headers, &jar).await {
        ensure_public_visibility_for_public_endpoint(status.visibility)?;
    }
    Ok(Json(
        crate::api::build_status_card_value(&status).unwrap_or(serde_json::Value::Null),
    ))
}

/// DELETE /api/v1/statuses/:id
pub async fn delete_status(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<DeleteStatusParams>,
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
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "accounts"])
        .inc();
    db_timer.observe_duration();

    if params.delete_media.unwrap_or(false) {
        state.db.replace_status_media(&id, &[]).await?;
    }

    // Delete the status
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["DELETE", "statuses"])
        .start_timer();
    status_service.delete_loaded(&status).await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["DELETE", "statuses"])
        .inc();
    db_timer.observe_duration();

    let extra_addresses = {
        let mut extra_addresses = Vec::new();
        if let Some(reply_target_account_address) = remote_account_address_for_status_uri(
            &state,
            &status_service,
            status.in_reply_to_uri.as_deref(),
        )
        .await?
        {
            extra_addresses.push(reply_target_account_address);
        }
        if let Some(quote_target_account_address) = remote_account_address_for_status_uri(
            &state,
            &status_service,
            status.quote_of_uri.as_deref(),
        )
        .await?
        {
            extra_addresses.push(quote_target_account_address);
        }
        extra_addresses
    };
    let source_text = status_content_to_source_text(&status.content);
    let (explicit_recipients, _) =
        resolve_explicit_remote_recipients(&state, &source_text, extra_addresses).await;
    let should_deliver_to_followers = should_deliver_to_followers_collection(status.visibility);
    let follower_inboxes = if should_deliver_to_followers {
        build_account_service(&state)
            .get_follower_inboxes()
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(
                    %error,
                    "Skipping follower fan-out prefetch for Delete delivery"
                );
                Vec::new()
            })
    } else {
        Vec::new()
    };
    let delivery_targets = merge_delivery_targets(follower_inboxes, &explicit_recipients);
    if !delivery_targets.is_empty() {
        let explicit_recipient_actor_uris = explicit_recipients
            .iter()
            .map(|recipient| recipient.actor_uri.clone())
            .collect::<Vec<_>>();
        let delivery = build_delivery(&state, &account);
        let state_for_delivery = state.clone();
        let status_uri = status.uri.clone();
        let status_visibility = status.visibility;
        spawn_best_effort_batch_delivery("delete_status", async move {
            delivery
                .queue_delete_with_audience(
                    state_for_delivery.db.as_ref(),
                    &status_uri,
                    status_visibility.as_str(),
                    delivery_targets,
                    &explicit_recipient_actor_uris,
                )
                .await
        });
    } else {
        tracing::debug!(
            visibility = %status.visibility,
            "Skipping outbound Delete delivery because no remote targets were found"
        );
    }

    // Update posts total metric
    POSTS_TOTAL.dec();

    // Return the deleted status
    let response = crate::api::build_status_response_with_account_stats(
        state.db.as_ref(),
        &status,
        &account,
        &state.config,
        account_stats,
        crate::api::StatusInteractions::new(
            Some(false),
            Some(false),
            Some(false),
            Some(false),
            Some(false),
        ),
    )
    .await?;

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["DELETE", "/api/v1/statuses/:id", "200"])
        .inc();

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/statuses/:id/context
pub async fn get_status_context(
    State(state): State<StatusApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::api::dto::ContextResponse;
    let status_service = build_status_service(&state);
    let is_authenticated = request_is_authenticated(&state, &headers, &jar).await;

    // Get the status to verify it exists
    let status = status_service.get(&id).await?;
    if !is_authenticated {
        ensure_public_visibility_for_public_endpoint(status.visibility)?;
    }

    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let mut ancestors = Vec::new();
    let mut seen_ancestors = HashSet::new();
    let mut current_parent_uri = status.in_reply_to_uri.clone();
    while let Some(parent_uri) = current_parent_uri {
        if ancestors.len() >= MAX_STATUS_CONTEXT_ANCESTORS {
            break;
        }
        if !seen_ancestors.insert(parent_uri.clone()) {
            break;
        }

        let Some(parent_status) = status_service.find_by_uri(&parent_uri).await? else {
            break;
        };
        if !is_authenticated && !should_federate_to_followers(parent_status.visibility) {
            break;
        }

        current_parent_uri = parent_status.in_reply_to_uri.clone();
        ancestors.push(parent_status);
    }
    ancestors.reverse();

    let mut descendants = Vec::new();
    let mut queue = VecDeque::new();
    let mut seen_descendants = HashSet::new();
    queue.push_back(status.uri.clone());
    seen_descendants.insert(status.uri.clone());

    'descendant_scan: while let Some(parent_uri) = queue.pop_front() {
        let remaining = MAX_STATUS_CONTEXT_DESCENDANTS.saturating_sub(descendants.len());
        if remaining == 0 {
            break;
        }
        let replies = status_service
            .get_replies_limited(&parent_uri, remaining)
            .await?;
        for reply in replies {
            if descendants.len() >= MAX_STATUS_CONTEXT_DESCENDANTS {
                break 'descendant_scan;
            }
            if !is_authenticated && !should_federate_to_followers(reply.visibility) {
                continue;
            }
            if !seen_descendants.insert(reply.uri.clone()) {
                continue;
            }
            queue.push_back(reply.uri.clone());
            descendants.push(reply);
        }
    }

    let mut ancestor_responses = Vec::with_capacity(ancestors.len());
    for ancestor in &ancestors {
        ancestor_responses.push(
            status_context_response(&state, &account, account_stats, ancestor, is_authenticated)
                .await?,
        );
    }

    let mut descendant_responses = Vec::with_capacity(descendants.len());
    for descendant in &descendants {
        descendant_responses.push(
            status_context_response(
                &state,
                &account,
                account_stats,
                descendant,
                is_authenticated,
            )
            .await?,
        );
    }

    let context = ContextResponse {
        ancestors: ancestor_responses,
        descendants: descendant_responses,
    };

    Ok(Json(serde_json::to_value(context).unwrap()))
}

/// GET /api/v1/statuses/:id/reblogged_by
pub async fn get_reblogged_by(
    State(state): State<StatusApiState>,
    Path(id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<impl IntoResponse, AppError> {
    let status_service = build_status_service(&state);

    // Get the status to verify it exists
    let status = status_service.get(&id).await?;
    ensure_public_visibility_for_public_endpoint(status.visibility)?;
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    let limit = params.limit.unwrap_or(40).min(80);
    let fetch_limit = limit.max(120).min(400);

    let mut identities = Vec::new();
    if state.db.is_reposted(&status.id).await? && identities.len() < fetch_limit {
        identities.push(local_actor_uri(&state, &account.username));
    }
    if identities.len() < fetch_limit {
        let remaining = fetch_limit - identities.len();
        identities.extend(
            state
                .db
                .list_remote_repost_actor_addresses(&status.id, remaining)
                .await?,
        );
    }

    let mut responses = Vec::new();
    for identity in identities {
        if let Some(account_response) =
            resolve_interaction_account_response(&state, &account, account_stats, &identity).await
        {
            responses.push(account_response);
        }
    }

    let responses = paginate_account_values(responses, &params, limit);
    let first_id = responses
        .first()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let last_id = responses
        .last()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let mut headers = HeaderMap::new();
    if let Some(link) = interaction_account_link_header(
        &format!("/api/v1/statuses/{}/reblogged_by", urlencoding::encode(&id)),
        limit,
        first_id,
        last_id,
    ) {
        headers.insert(LINK, link.parse().expect("valid link header"));
    }

    Ok((headers, Json(responses)))
}

/// GET /api/v1/statuses/:id/favourited_by
pub async fn get_favourited_by(
    State(state): State<StatusApiState>,
    Path(id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<impl IntoResponse, AppError> {
    let status_service = build_status_service(&state);

    // Get the status to verify it exists
    let status = status_service.get(&id).await?;
    ensure_public_visibility_for_public_endpoint(status.visibility)?;
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    let limit = params.limit.unwrap_or(40).min(80);
    let fetch_limit = limit.max(120).min(400);

    let mut identities = Vec::new();
    if status_service.is_favourited(&status.id).await? && identities.len() < fetch_limit {
        identities.push(local_actor_uri(&state, &account.username));
    }
    if identities.len() < fetch_limit {
        let remaining = fetch_limit - identities.len();
        identities.extend(
            state
                .db
                .list_remote_favourite_actor_addresses(&status.id, remaining)
                .await?,
        );
    }

    let mut responses = Vec::new();
    for identity in identities {
        if let Some(account_response) =
            resolve_interaction_account_response(&state, &account, account_stats, &identity).await
        {
            responses.push(account_response);
        }
    }

    let responses = paginate_account_values(responses, &params, limit);
    let first_id = responses
        .first()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let last_id = responses
        .last()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let mut headers = HeaderMap::new();
    if let Some(link) = interaction_account_link_header(
        &format!(
            "/api/v1/statuses/{}/favourited_by",
            urlencoding::encode(&id)
        ),
        limit,
        first_id,
        last_id,
    ) {
        headers.insert(LINK, link.parse().expect("valid link header"));
    }

    Ok((headers, Json(responses)))
}

/// GET /api/v1/statuses/:id/source
pub async fn get_status_source(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get the status
    let status = status_service.get(&id).await?;
    if !status.is_local {
        return Err(AppError::Forbidden);
    }

    // Return the source
    let source = StatusSourceResponse {
        id: status.id.clone(),
        text: status_content_to_source_text(&status.content),
        spoiler_text: status.content_warning.unwrap_or_default(),
    };

    Ok(Json(serde_json::to_value(source).unwrap()))
}

/// POST /api/v1/statuses/:id/favourite
pub async fn favourite_status(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<StatusActionParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

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
            let (target_actor_uri, target_inbox_uri) =
                resolve_remote_actor_and_inbox_with_dependencies(
                    state_for_delivery.db.as_ref(),
                    state_for_delivery.profile_cache.as_ref(),
                    state_for_delivery.federation_fetch_client.as_ref(),
                    &account_address_for_delivery,
                )
                .await?;
            let delivery = build_delivery(&state_for_delivery, &account_for_delivery);
            delivery
                .queue_like_with_id(
                    state_for_delivery.db.as_ref(),
                    &like_activity_uri,
                    &status_uri,
                    &target_inbox_uri,
                    &target_actor_uri,
                )
                .await
        });
    }

    // Return status with favourited=true
    let response = crate::api::build_status_response_with_account_stats(
        state.db.as_ref(),
        &status,
        &account,
        &state.config,
        account_stats,
        crate::api::StatusInteractions::new(
            Some(true),
            status_service.is_reposted(&status_id).await.ok(),
            status_service.is_muted(&status_id).await.ok(),
            status_service.is_bookmarked(&status_id).await.ok(),
            status_service.is_pinned(&status_id).await.ok(),
        ),
    )
    .await?;

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unfavourite
pub async fn unfavourite_status(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<StatusActionParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let status = if let Some(uri) = resolve_action_uri(&id, &params)? {
        status_service
            .ensure_remote_status_persisted(uri, PersistedReason::Favourited)
            .await?
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

    if let Some(like_activity_uri) = like_activity_uri
        && !status.is_local
        && !status.account_address.is_empty()
    {
        let state_for_delivery = state.clone();
        let account_for_delivery = account.clone();
        let account_address_for_delivery = status.account_address.clone();
        spawn_best_effort_delivery("unfavourite_status", async move {
            let (target_actor_uri, target_inbox_uri) =
                resolve_remote_actor_and_inbox_with_dependencies(
                    state_for_delivery.db.as_ref(),
                    state_for_delivery.profile_cache.as_ref(),
                    state_for_delivery.federation_fetch_client.as_ref(),
                    &account_address_for_delivery,
                )
                .await?;
            let delivery = build_delivery(&state_for_delivery, &account_for_delivery);
            delivery
                .queue_undo_to_inbox_with_type_and_object(
                    state_for_delivery.db.as_ref(),
                    &like_activity_uri,
                    Some("Like"),
                    None,
                    Some(&target_actor_uri),
                    &target_inbox_uri,
                )
                .await
        });
    }

    // Return status with favourited=false
    let response = crate::api::build_status_response_with_account_stats(
        state.db.as_ref(),
        &status,
        &account,
        &state.config,
        account_stats,
        crate::api::StatusInteractions::new(
            Some(false),
            status_service.is_reposted(&status_id).await.ok(),
            status_service.is_muted(&status_id).await.ok(),
            status_service.is_bookmarked(&status_id).await.ok(),
            status_service.is_pinned(&status_id).await.ok(),
        ),
    )
    .await?;

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/reblog
pub async fn reblog_status(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<StatusActionParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::data::EntityId;

    let status_service = build_status_service(&state);

    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let action_uri = resolve_action_uri(&id, &params)?;
    let (status, repost_uri, repost_id) = if let Some(uri) = action_uri {
        let status = status_service.repost(uri).await?;
        let repost_uri = status_service
            .get_repost_uri(&status.id)
            .await?
            .ok_or_else(|| {
                AppError::internal("repost URI missing after creating repost activity".to_string())
            })?;
        let repost_id = state.db.get_repost_id(&status.id).await?.ok_or_else(|| {
            AppError::internal("repost ID missing after creating repost activity".to_string())
        })?;
        (status, repost_uri, repost_id)
    } else {
        // Create repost record
        let repost_id = EntityId::new_string();
        let repost_uri = format!(
            "{}/users/{}/statuses/{}/activity",
            state.config.server.base_url(),
            account.username,
            repost_id
        );
        let status = status_service.repost_by_id(&id, &repost_uri).await?;
        let persisted_repost_id = state
            .db
            .get_repost_id(&status.id)
            .await?
            .unwrap_or(repost_id);
        (status, repost_uri, persisted_repost_id)
    };
    let announce_visibility = params
        .visibility
        .clone()
        .map(|value| normalize_visibility_input(Some(value)))
        .transpose()?
        .unwrap_or(status.visibility);

    let should_federate_reblog = should_federate_to_followers(announce_visibility);
    if should_federate_reblog {
        match build_account_service(&state).get_follower_inboxes().await {
            Ok(follower_inboxes) if !follower_inboxes.is_empty() => {
                let delivery = build_delivery(&state, &account);
                let state_for_delivery = state.clone();
                let announce_activity_uri = repost_uri.clone();
                let announced_status_uri = status.uri.clone();
                spawn_best_effort_batch_delivery("reblog_status", async move {
                    delivery
                        .queue_announce_with_id(
                            state_for_delivery.db.as_ref(),
                            &announce_activity_uri,
                            &announced_status_uri,
                            announce_visibility.as_str(),
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
            visibility = %announce_visibility,
            "Skipping outbound Announce delivery for non-public visibility"
        );
    }

    if !status.is_local {
        let mut reblogged_status =
            status_response_with_viewer_interactions(&state, &account, account_stats, &status)
                .await?;
        reblogged_status.reblogged = true;
        return Ok(Json(serde_json::to_value(reblogged_status).map_err(
            |error| AppError::serialization("reblogged status response serialization", error),
        )?));
    }

    let wrapper = reblog_wrapper_response(
        &state,
        &account,
        account_stats,
        &status,
        &repost_id,
        &repost_uri,
        Utc::now(),
        announce_visibility,
    )
    .await?;

    Ok(Json(serde_json::to_value(wrapper).unwrap()))
}

/// POST /api/v1/statuses/:id/unreblog
pub async fn unreblog_status(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<StatusActionParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let action_uri = resolve_action_uri(&id, &params)?;
    let status = if let Some(uri) = action_uri {
        status_service
            .ensure_remote_status_persisted(uri, PersistedReason::Reposted)
            .await?
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
        let should_federate_unreblog = should_federate_to_followers(status.visibility);
        if should_federate_unreblog {
            match build_account_service(&state).get_follower_inboxes().await {
                Ok(follower_inboxes) if !follower_inboxes.is_empty() => {
                    let delivery = build_delivery(&state, &account);
                    let state_for_delivery = state.clone();
                    spawn_best_effort_batch_delivery("unreblog_status", async move {
                        delivery
                            .queue_undo_with_type(
                                state_for_delivery.db.as_ref(),
                                &repost_uri,
                                Some("Announce"),
                                follower_inboxes,
                            )
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
    let response = crate::api::build_status_response_with_account_stats(
        state.db.as_ref(),
        &status,
        &account,
        &state.config,
        account_stats,
        crate::api::StatusInteractions::new(
            status_service.is_favourited(&status_id).await.ok(),
            Some(false),
            status_service.is_muted(&status_id).await.ok(),
            status_service.is_bookmarked(&status_id).await.ok(),
            status_service.is_pinned(&status_id).await.ok(),
        ),
    )
    .await?;

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/bookmark
pub async fn bookmark_status(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<StatusActionParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    // Get status and add bookmark.
    let status = if let Some(uri) = resolve_action_uri(&id, &params)? {
        status_service.bookmark(uri).await?
    } else {
        status_service.bookmark_by_id(&id).await?
    };
    let status_id = status.id.clone();

    // Return status with bookmarked=true
    let response = crate::api::build_status_response_with_account_stats(
        state.db.as_ref(),
        &status,
        &account,
        &state.config,
        account_stats,
        crate::api::StatusInteractions::new(
            status_service.is_favourited(&status_id).await.ok(),
            status_service.is_reposted(&status_id).await.ok(),
            status_service.is_muted(&status_id).await.ok(),
            Some(true),
            status_service.is_pinned(&status_id).await.ok(),
        ),
    )
    .await?;

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unbookmark
pub async fn unbookmark_status(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Query(params): Query<StatusActionParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    // Get status and remove bookmark.
    let status = if let Some(uri) = resolve_action_uri(&id, &params)? {
        let status = status_service
            .ensure_remote_status_persisted(uri, PersistedReason::Bookmarked)
            .await?;
        status_service.unbookmark_loaded(&status).await?;
        status
    } else {
        status_service.unbookmark_by_id(&id).await?
    };
    let status_id = status.id.clone();

    // Return status with bookmarked=false
    let response = crate::api::build_status_response_with_account_stats(
        state.db.as_ref(),
        &status,
        &account,
        &state.config,
        account_stats,
        crate::api::StatusInteractions::new(
            status_service.is_favourited(&status_id).await.ok(),
            status_service.is_reposted(&status_id).await.ok(),
            status_service.is_muted(&status_id).await.ok(),
            Some(false),
            status_service.is_pinned(&status_id).await.ok(),
        ),
    )
    .await?;

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// Update status request
#[derive(Debug, Default, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: Option<String>,
    pub spoiler_text: Option<String>,
    #[serde(rename = "sensitive")]
    pub sensitive: Option<bool>,
    pub media_ids: Option<Vec<String>>,
    #[serde(default)]
    pub media_attributes: Vec<StatusMediaAttributeRequest>,
    pub language: Option<String>,
    pub poll: Option<CreateStatusPollRequest>,
}

fn parse_status_bool(field: &str, value: &str) -> Result<bool, AppError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Ok(true),
        "0" | "false" | "off" | "no" => Ok(false),
        _ => Err(AppError::Validation(format!(
            "{field} must be a boolean value"
        ))),
    }
}

fn parse_indexed_media_attribute_key<'a>(key: &'a str) -> Option<(usize, &'a str)> {
    let remainder = key.strip_prefix("media_attributes[")?;
    let (index, remainder) = remainder.split_once(']')?;
    let index = index.parse::<usize>().ok()?;
    let field = remainder.strip_prefix('[')?.strip_suffix(']')?;
    Some((index, field))
}

fn ensure_media_attribute_slot(
    media_attributes: &mut Vec<StatusMediaAttributeRequest>,
    index: usize,
) -> &mut StatusMediaAttributeRequest {
    while media_attributes.len() <= index {
        media_attributes.push(StatusMediaAttributeRequest::default());
    }
    &mut media_attributes[index]
}

fn parse_create_status_request(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<CreateStatusRequest, AppError> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if content_type.starts_with("application/json") || content_type.is_empty() {
        return serde_json::from_slice(body)
            .map_err(|error| AppError::Validation(format!("invalid JSON body: {error}")));
    }

    if content_type.starts_with("application/x-www-form-urlencoded") {
        let mut request = CreateStatusRequest::default();
        for (key, value) in url::form_urlencoded::parse(body).into_owned() {
            match key.as_str() {
                "status" => request.status = Some(value),
                "media_ids[]" | "media_ids" => {
                    request.media_ids.get_or_insert_with(Vec::new).push(value);
                }
                key if key.starts_with("media_attributes[") => {
                    if let Some((index, field)) = parse_indexed_media_attribute_key(key) {
                        let attribute =
                            ensure_media_attribute_slot(&mut request.media_attributes, index);
                        match field {
                            "id" => attribute.id = Some(value),
                            "description" => attribute.description = Some(value),
                            "focus" => attribute.focus = Some(value),
                            _ => {}
                        }
                    }
                }
                "in_reply_to_id" => request.in_reply_to_id = Some(value),
                "quoted_status_id" => request.quoted_status_id = Some(value),
                "scheduled_at" => request.scheduled_at = Some(value),
                "sensitive" => request.sensitive = Some(parse_status_bool("sensitive", &value)?),
                "spoiler_text" => request.spoiler_text = Some(value),
                "visibility" => request.visibility = Some(value),
                "language" => request.language = Some(value),
                "poll[options][]" | "poll[options]" => {
                    request
                        .poll
                        .get_or_insert_with(CreateStatusPollRequest::default)
                        .options
                        .push(value);
                }
                "poll[expires_in]" => {
                    request
                        .poll
                        .get_or_insert_with(CreateStatusPollRequest::default)
                        .expires_in = value.parse::<i64>().map_err(|_| {
                        AppError::Validation("poll[expires_in] must be an integer".to_string())
                    })?;
                }
                "poll[multiple]" => {
                    request
                        .poll
                        .get_or_insert_with(CreateStatusPollRequest::default)
                        .multiple = parse_status_bool("poll[multiple]", &value)?;
                }
                "poll[hide_totals]" => {
                    request
                        .poll
                        .get_or_insert_with(CreateStatusPollRequest::default)
                        .hide_totals = parse_status_bool("poll[hide_totals]", &value)?;
                }
                _ => {}
            }
        }
        return Ok(request);
    }

    Err(AppError::Validation(
        "unsupported content type for status payload".to_string(),
    ))
}

fn parse_update_status_request(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<UpdateStatusRequest, AppError> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if content_type.starts_with("application/json") || content_type.is_empty() {
        return serde_json::from_slice(body)
            .map_err(|error| AppError::Validation(format!("invalid JSON body: {error}")));
    }

    if content_type.starts_with("application/x-www-form-urlencoded") {
        let mut request = UpdateStatusRequest::default();
        for (key, value) in url::form_urlencoded::parse(body).into_owned() {
            match key.as_str() {
                "status" => request.status = Some(value),
                "spoiler_text" => request.spoiler_text = Some(value),
                "sensitive" => request.sensitive = Some(parse_status_bool("sensitive", &value)?),
                "media_ids[]" | "media_ids" => {
                    request.media_ids.get_or_insert_with(Vec::new).push(value);
                }
                key if key.starts_with("media_attributes[") => {
                    if let Some((index, field)) = parse_indexed_media_attribute_key(key) {
                        let attribute =
                            ensure_media_attribute_slot(&mut request.media_attributes, index);
                        match field {
                            "id" => attribute.id = Some(value),
                            "description" => attribute.description = Some(value),
                            "focus" => attribute.focus = Some(value),
                            _ => {}
                        }
                    }
                }
                "language" => request.language = Some(value),
                "poll[options][]" | "poll[options]" => {
                    request
                        .poll
                        .get_or_insert_with(CreateStatusPollRequest::default)
                        .options
                        .push(value);
                }
                "poll[expires_in]" => {
                    request
                        .poll
                        .get_or_insert_with(CreateStatusPollRequest::default)
                        .expires_in = value.parse::<i64>().map_err(|_| {
                        AppError::Validation("poll[expires_in] must be an integer".to_string())
                    })?;
                }
                "poll[multiple]" => {
                    request
                        .poll
                        .get_or_insert_with(CreateStatusPollRequest::default)
                        .multiple = parse_status_bool("poll[multiple]", &value)?;
                }
                "poll[hide_totals]" => {
                    request
                        .poll
                        .get_or_insert_with(CreateStatusPollRequest::default)
                        .hide_totals = parse_status_bool("poll[hide_totals]", &value)?;
                }
                _ => {}
            }
        }
        return Ok(request);
    }

    Err(AppError::Validation(
        "unsupported content type for status payload".to_string(),
    ))
}

async fn apply_status_media_attributes(
    state: &StatusApiState,
    media_ids: &[String],
    status_id: &str,
    media_attributes: &[StatusMediaAttributeRequest],
) -> Result<(), AppError> {
    for (index, attribute) in media_attributes.iter().enumerate() {
        let Some(media_id) = attribute
            .id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| media_ids.get(index).cloned())
        else {
            continue;
        };

        let mut media = state.db.get_media(&media_id).await?.ok_or_else(|| {
            AppError::Validation(format!(
                "unknown media attachment in media_attributes: {media_id}"
            ))
        })?;
        if media.status_id.as_deref() != Some(status_id) {
            return Err(AppError::Validation(format!(
                "media attachment `{media_id}` is not attached to status `{status_id}`"
            )));
        }

        if let Some(description) = attribute.description.clone() {
            media.description = Some(description);
        }

        if let Some(focus) = attribute.focus.as_deref() {
            let trimmed = focus.trim();
            if trimmed.is_empty() {
                media.focus_x = None;
                media.focus_y = None;
            } else {
                let (focus_x, focus_y) = crate::api::mastodon::media::parse_media_focus(trimmed)?;
                media.focus_x = Some(focus_x);
                media.focus_y = Some(focus_y);
            }
        }

        state.db.update_media(&media).await?;
    }

    Ok(())
}

#[derive(Debug, Default, Deserialize)]
pub struct DeleteStatusParams {
    pub delete_media: Option<bool>,
}

/// PUT /api/v1/statuses/:id
/// Edit an existing status
///
pub async fn update_status(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 256 * 1024)
        .await
        .map_err(|error| AppError::Validation(format!("failed to read request body: {error}")))?;
    let req = if body.is_empty() {
        UpdateStatusRequest::default()
    } else {
        parse_update_status_request(&parts.headers, &body)?
    };
    let UpdateStatusRequest {
        status: requested_content,
        spoiler_text,
        sensitive,
        media_ids: requested_media_ids,
        media_attributes,
        language,
        poll,
    } = req;

    // Get the status
    let mut status = status_service.get(&id).await?;
    let previous_status = status.clone();

    // Only allow editing local statuses
    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    // Update fields if provided
    let mut changed = false;
    let mut media_ids_to_replace: Option<Vec<String>> = None;
    let mut media_ids_after_update = status_service
        .get_media_by_status(&id)
        .await?
        .into_iter()
        .map(|media| media.id)
        .collect::<Vec<_>>();
    let existing_poll = state.db.get_poll_by_status_id(&id).await?;
    let mut poll_to_replace = None;
    let mut delete_poll = false;

    if let Some(content) = requested_content {
        let next_content = format!("<p>{}</p>", html_escape::encode_text(&content));
        if status.content != next_content {
            status.content = next_content;
            changed = true;
        }
    }

    let next_content_warning =
        normalize_content_warning(spoiler_text, sensitive, status.content_warning.as_deref());
    if status.content_warning != next_content_warning {
        status.content_warning = next_content_warning;
        changed = true;
    }

    if let Some(language) = language {
        let normalized_language = (!language.trim().is_empty()).then_some(language);
        if status.language != normalized_language {
            status.language = normalized_language;
            changed = true;
        }
    }

    if let Some(media_ids) = requested_media_ids {
        let mut normalized_media_ids = Vec::with_capacity(media_ids.len());
        let mut seen_media_ids = HashSet::new();
        for media_id in media_ids {
            let trimmed = media_id.trim();
            if trimmed.is_empty() {
                return Err(AppError::Validation(
                    "media_ids must not contain empty values".to_string(),
                ));
            }
            if seen_media_ids.insert(trimmed.to_string()) {
                normalized_media_ids.push(trimmed.to_string());
            }
        }

        let current_media_ids = media_ids_after_update
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let requested_media_ids = normalized_media_ids.iter().cloned().collect::<HashSet<_>>();
        if current_media_ids != requested_media_ids {
            media_ids_after_update = normalized_media_ids.clone();
            media_ids_to_replace = Some(normalized_media_ids);
            changed = true;
        }
    }

    if let Some(poll_request) = poll {
        let normalized_poll = normalize_poll_input(Some(poll_request))?;
        if !media_ids_after_update.is_empty() {
            return Err(AppError::Unprocessable(
                "poll and media_ids cannot be used together".to_string(),
            ));
        }
        if let Some((poll_id, _expires_at, expired, _multiple, _hide_totals, votes_count, _)) =
            existing_poll.as_ref()
            && (*expired
                || *votes_count > 0
                || state
                    .db
                    .get_poll_options(poll_id)
                    .await?
                    .iter()
                    .any(|(_, _, votes)| *votes > 0))
        {
            return Err(AppError::Unprocessable(
                "cannot edit a poll after voting has started or it has expired".to_string(),
            ));
        }
        poll_to_replace = normalized_poll;
        changed = true;
    } else if !media_ids_after_update.is_empty() && existing_poll.is_some() {
        delete_poll = true;
        changed = true;
    }

    if !media_attributes.is_empty() {
        changed = true;
    }

    if changed {
        let (snapshot_media_json, snapshot_poll_json, snapshot_quote_json) =
            build_status_edit_snapshot_payload(&state, &account, account_stats, &previous_status)
                .await?;
        status_service
            .update_with_edit_snapshot_and_media(
                &previous_status,
                &status,
                media_ids_to_replace.as_deref(),
                snapshot_media_json.as_deref(),
                snapshot_poll_json.as_deref(),
                snapshot_quote_json.as_deref(),
            )
            .await?;
        if let Some(poll) = poll_to_replace.as_ref() {
            let expires_at = (Utc::now() + chrono::Duration::seconds(poll.expires_in)).to_rfc3339();
            state
                .db
                .replace_poll_for_status(
                    &status.id,
                    &expires_at,
                    false,
                    poll.multiple,
                    poll.hide_totals,
                    0,
                    0,
                    &poll
                        .options
                        .iter()
                        .cloned()
                        .map(|option| (option, 0))
                        .collect::<Vec<_>>(),
                )
                .await?;
        } else if delete_poll {
            state.db.delete_poll_by_status_id(&status.id).await?;
        }
        if !media_attributes.is_empty() {
            apply_status_media_attributes(
                &state,
                &media_ids_after_update,
                &status.id,
                &media_attributes,
            )
            .await?;
        }

        let mut extra_addresses = Vec::new();
        if let Some(reply_target_account_address) = remote_account_address_for_status_uri(
            &state,
            &status_service,
            status.in_reply_to_uri.as_deref(),
        )
        .await?
        {
            extra_addresses.push(reply_target_account_address);
        }
        if let Some(quote_target_account_address) = remote_account_address_for_status_uri(
            &state,
            &status_service,
            status.quote_of_uri.as_deref(),
        )
        .await?
        {
            extra_addresses.push(quote_target_account_address);
        }

        let source_text = status_content_to_source_text(&status.content);
        let (explicit_recipients, mention_tags) =
            resolve_explicit_remote_recipients(&state, &source_text, extra_addresses).await;
        let follower_inboxes = if should_deliver_to_followers_collection(status.visibility) {
            build_account_service(&state)
                .get_follower_inboxes()
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        %error,
                        status_id = %status.id,
                        "Skipping follower fan-out prefetch for Update delivery"
                    );
                    Vec::new()
                })
        } else {
            Vec::new()
        };
        let delivery_targets = merge_delivery_targets(follower_inboxes, &explicit_recipients);
        if !delivery_targets.is_empty() {
            let delivery = build_delivery(&state, &account);
            let state_for_delivery = state.clone();
            let status_for_delivery = status.clone();
            let explicit_recipient_actor_uris = explicit_recipients
                .iter()
                .map(|recipient| recipient.actor_uri.clone())
                .collect::<Vec<_>>();
            spawn_best_effort_batch_delivery("update_status", async move {
                delivery
                    .queue_update_status(
                        state_for_delivery.db.as_ref(),
                        &status_for_delivery,
                        delivery_targets,
                        &explicit_recipient_actor_uris,
                        &mention_tags,
                    )
                    .await
            });
        }
    }

    // Return updated status
    let response = crate::api::build_status_response_with_account_stats(
        state.db.as_ref(),
        &status,
        &account,
        &state.config,
        account_stats,
        crate::api::StatusInteractions::new(
            status_service.is_favourited(&status.id).await.ok(),
            status_service.is_reposted(&status.id).await.ok(),
            status_service.is_muted(&status.id).await.ok(),
            status_service.is_bookmarked(&status.id).await.ok(),
            status_service.is_pinned(&status.id).await.ok(),
        ),
    )
    .await?;

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/statuses/:id/history
/// Get edit history for a status
///
pub async fn get_status_history(
    State(state): State<StatusApiState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let status_service = build_status_service(&state);

    // Get the status
    let status = status_service.get(&id).await?;

    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    let current_response =
        status_response_with_viewer_interactions(&state, &account, account_stats, &status).await?;

    let edits = status_service.get_edit_history(&id, 40).await?;
    let current_revision_created_at = edits
        .first()
        .map(|(_, _, _, _, _, _, created_at)| (*created_at).max(status.created_at))
        .unwrap_or(status.created_at);
    let current_version = serde_json::json!({
        "content": status.content,
        "spoiler_text": status.content_warning.clone().unwrap_or_default(),
        "sensitive": status.content_warning.is_some(),
        "created_at": current_revision_created_at.to_rfc3339(),
        "account": current_response.account.clone(),
        "media_attachments": current_response.media_attachments.clone(),
        "emojis": current_response.emojis.clone(),
        "poll": current_response.poll.clone(),
        "quote": current_response.quote.clone(),
    });

    let mut history = vec![current_version];
    for (_, content, content_warning, media_attachments_json, poll_json, quote_json, created_at) in
        edits
    {
        let media_attachments = media_attachments_json
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()
            .map_err(|error| AppError::serialization("status history media snapshot", error))?
            .unwrap_or_else(|| serde_json::json!(current_response.media_attachments.clone()));
        let poll = poll_json
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()
            .map_err(|error| AppError::serialization("status history poll snapshot", error))?
            .unwrap_or_else(|| {
                current_response
                    .poll
                    .clone()
                    .unwrap_or(serde_json::Value::Null)
            });
        let quote = quote_json
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()
            .map_err(|error| AppError::serialization("status history quote snapshot", error))?
            .unwrap_or_else(|| {
                current_response
                    .quote
                    .clone()
                    .unwrap_or(serde_json::Value::Null)
            });
        history.push(serde_json::json!({
            "content": content,
            "spoiler_text": content_warning.clone().unwrap_or_default(),
            "sensitive": content_warning.is_some(),
            "created_at": created_at.to_rfc3339(),
            "account": current_response.account.clone(),
            "media_attachments": media_attachments,
            "emojis": current_response.emojis.clone(),
            "poll": poll,
            "quote": quote,
        }));
    }

    Ok(Json(history))
}

/// POST /api/v1/statuses/:id/pin
/// Pin a status to profile
///
pub async fn pin_status(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get status and store pinned marker
    let status = status_service.pin_by_id(&id).await?;

    // Only allow pinning local statuses
    if !status.is_local {
        return Err(AppError::Validation(
            "Can only pin own statuses".to_string(),
        ));
    }

    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let response = crate::api::build_status_response_with_account_stats(
        state.db.as_ref(),
        &status,
        &account,
        &state.config,
        account_stats,
        crate::api::StatusInteractions::new(
            status_service.is_favourited(&status.id).await.ok(),
            status_service.is_reposted(&status.id).await.ok(),
            status_service.is_muted(&status.id).await.ok(),
            status_service.is_bookmarked(&status.id).await.ok(),
            Some(true),
        ),
    )
    .await?;

    if should_federate_to_followers(status.visibility) {
        let follower_inboxes = state.db.get_follower_inboxes().await.unwrap_or_default();
        if !follower_inboxes.is_empty() {
            let state_for_delivery = state.clone();
            let account_for_delivery = account.clone();
            let status_uri = status.uri.clone();
            spawn_best_effort_delivery("pin_status", async move {
                let delivery = build_delivery(&state_for_delivery, &account_for_delivery);
                delivery
                    .queue_add_featured(
                        state_for_delivery.db.as_ref(),
                        &status_uri,
                        follower_inboxes,
                    )
                    .await;
                Ok(())
            });
        }
    }

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unpin
/// Unpin a status from profile
///
pub async fn unpin_status(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get status and remove pinned marker
    let status = status_service.unpin_by_id(&id).await?;

    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let response = crate::api::build_status_response_with_account_stats(
        state.db.as_ref(),
        &status,
        &account,
        &state.config,
        account_stats,
        crate::api::StatusInteractions::new(
            status_service.is_favourited(&status.id).await.ok(),
            status_service.is_reposted(&status.id).await.ok(),
            status_service.is_muted(&status.id).await.ok(),
            status_service.is_bookmarked(&status.id).await.ok(),
            Some(false),
        ),
    )
    .await?;

    if should_federate_to_followers(status.visibility) {
        let follower_inboxes = state.db.get_follower_inboxes().await.unwrap_or_default();
        if !follower_inboxes.is_empty() {
            let state_for_delivery = state.clone();
            let account_for_delivery = account.clone();
            let status_uri = status.uri.clone();
            spawn_best_effort_delivery("unpin_status", async move {
                let delivery = build_delivery(&state_for_delivery, &account_for_delivery);
                delivery
                    .queue_remove_featured(
                        state_for_delivery.db.as_ref(),
                        &status_uri,
                        follower_inboxes,
                    )
                    .await;
                Ok(())
            });
        }
    }

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/mute
/// Mute notifications from a conversation
///
pub async fn mute_status(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get status and persist muted conversation marker
    let status = status_service.mute_by_id(&id).await?;

    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let response = crate::api::build_status_response_with_account_stats(
        state.db.as_ref(),
        &status,
        &account,
        &state.config,
        account_stats,
        crate::api::StatusInteractions::new(
            status_service.is_favourited(&status.id).await.ok(),
            status_service.is_reposted(&status.id).await.ok(),
            Some(true),
            status_service.is_bookmarked(&status.id).await.ok(),
            status_service.is_pinned(&status.id).await.ok(),
        ),
    )
    .await?;

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v1/statuses/:id/unmute
/// Unmute notifications from a conversation
///
pub async fn unmute_status(
    State(state): State<StatusApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status_service = build_status_service(&state);

    // Get status and remove muted conversation marker
    let status = status_service.unmute_by_id(&id).await?;

    // Get account
    let account = build_account_service(&state).get_account().await?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let response = crate::api::build_status_response_with_account_stats(
        state.db.as_ref(),
        &status,
        &account,
        &state.config,
        account_stats,
        crate::api::StatusInteractions::new(
            status_service.is_favourited(&status.id).await.ok(),
            status_service.is_reposted(&status.id).await.ok(),
            Some(false),
            status_service.is_bookmarked(&status.id).await.ok(),
            status_service.is_pinned(&status.id).await.ok(),
        ),
    )
    .await?;

    Ok(Json(serde_json::to_value(response).unwrap()))
}
