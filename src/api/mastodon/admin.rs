//! Mastodon-compatible Admin API endpoints

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, header::CONTENT_TYPE},
    response::Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

use super::accounts::{
    build_remote_account_placeholder_response, default_port_for_protocol,
    normalize_account_address, parse_actor_uri_account_address,
    resolve_account_response_for_identity,
};
use crate::AdminApiState;
use crate::auth::CurrentUser;
use crate::data::{AdminReportState, Notification, NotificationType, Status};
use crate::error::AppError;

const DEFAULT_INSTANCE_RULES: [&str; 3] = [
    "Be respectful and civil in all interactions.",
    "No spam, harassment, or illegal content.",
    "Content warnings are required for sensitive material.",
];

#[derive(Debug, Deserialize)]
pub struct AdminAccountParams {
    #[serde(rename = "local")]
    local: Option<bool>,
    #[serde(rename = "remote")]
    remote: Option<bool>,
    #[serde(rename = "active")]
    active: Option<bool>,
    #[serde(rename = "pending")]
    pending: Option<bool>,
    #[serde(rename = "disabled")]
    disabled: Option<bool>,
    #[serde(rename = "silenced")]
    silenced: Option<bool>,
    #[serde(rename = "suspended")]
    suspended: Option<bool>,
    #[serde(rename = "username")]
    username: Option<String>,
    #[serde(rename = "display_name")]
    display_name: Option<String>,
    #[serde(rename = "email")]
    email: Option<String>,
    #[serde(rename = "ip")]
    ip: Option<String>,
    #[serde(rename = "max_id")]
    max_id: Option<String>,
    #[serde(rename = "since_id")]
    since_id: Option<String>,
    #[serde(rename = "min_id")]
    min_id: Option<String>,
    #[serde(rename = "limit")]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct AdminAccount {
    pub id: String,
    pub username: String,
    pub domain: Option<String>,
    pub created_at: String,
    pub email: Option<String>,
    pub ip: Option<String>,
    pub role: String,
    pub confirmed: bool,
    pub suspended: bool,
    pub silenced: bool,
    pub disabled: bool,
    pub approved: bool,
    pub account: serde_json::Value,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Default)]
pub struct AdminActionRequest {
    #[serde(default, rename = "type", alias = "action")]
    pub action: String,
    pub report_id: Option<String>,
    pub warning_preset_id: Option<String>,
    pub text: Option<String>,
    pub send_email_notification: Option<bool>,
    #[serde(rename = "reason")]
    pub _reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct UpdateReportRequest {
    pub category: Option<String>,
    #[serde(default, rename = "rule_ids[]", alias = "rule_ids")]
    pub rule_ids: Vec<String>,
}

enum AdminActionTarget {
    Local(crate::data::Account),
    Remote {
        address: String,
        actor_uri: Option<String>,
        inbox_uri: Option<String>,
    },
}

#[derive(Clone)]
struct AdminRemoteAccountCandidate {
    identity: String,
    suspended: bool,
    silenced: bool,
}

fn normalize_domain_block_domain(domain: &str) -> Result<String, AppError> {
    let normalized = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(AppError::Validation("domain is required".to_string()));
    }

    match url::Host::parse(&normalized) {
        Ok(url::Host::Domain(valid_domain)) => Ok(valid_domain.to_owned()),
        _ => Err(AppError::Validation(
            "domain must be a valid DNS hostname".to_string(),
        )),
    }
}

fn normalize_domain_block_severity(raw: Option<&str>) -> Result<String, AppError> {
    let severity = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("suspend")
        .to_ascii_lowercase();

    match severity.as_str() {
        "noop" | "silence" | "suspend" => Ok(severity),
        _ => Err(AppError::Validation(
            "severity must be one of: noop, silence, suspend".to_string(),
        )),
    }
}

fn normalize_admin_report_category(raw: Option<&str>) -> Result<String, AppError> {
    let category = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("other")
        .to_ascii_lowercase();

    match category.as_str() {
        "spam" | "legal" | "violation" | "other" => Ok(category),
        _ => Err(AppError::Validation(
            "category must be one of: spam, legal, violation, other".to_string(),
        )),
    }
}

fn parse_admin_action_request_json(body: &[u8]) -> Result<AdminActionRequest, AppError> {
    serde_json::from_slice(body).map_err(|error| {
        AppError::Validation(format!("invalid admin action request body: {error}"))
    })
}

fn parse_admin_action_request_form(body: &[u8]) -> Result<AdminActionRequest, AppError> {
    serde_urlencoded::from_bytes(body)
        .map_err(|error| AppError::Validation(format!("invalid admin action form body: {error}")))
}

fn parse_update_report_request_json(body: &[u8]) -> Result<UpdateReportRequest, AppError> {
    serde_json::from_slice(body).map_err(|error| {
        AppError::Validation(format!("invalid report update request body: {error}"))
    })
}

fn parse_update_report_request_form(body: &[u8]) -> Result<UpdateReportRequest, AppError> {
    let mut request = UpdateReportRequest::default();
    for (key, value) in url::form_urlencoded::parse(body) {
        match key.as_ref() {
            "category" => request.category = Some(value.into_owned()),
            "rule_ids" | "rule_ids[]" => request.rule_ids.push(value.into_owned()),
            _ => {}
        }
    }
    Ok(request)
}

fn content_type(headers: &HeaderMap) -> &str {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
}

fn parse_admin_action_request(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<AdminActionRequest, AppError> {
    let content_type = content_type(headers);
    if content_type.starts_with("application/x-www-form-urlencoded") {
        return parse_admin_action_request_form(body);
    }
    if content_type.starts_with("application/json") || content_type.is_empty() {
        return parse_admin_action_request_json(body);
    }

    parse_admin_action_request_json(body).or_else(|_| parse_admin_action_request_form(body))
}

fn parse_update_report_request(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<UpdateReportRequest, AppError> {
    let content_type = content_type(headers);
    if content_type.starts_with("application/x-www-form-urlencoded") {
        return parse_update_report_request_form(body);
    }
    if content_type.starts_with("application/json") || content_type.is_empty() {
        return parse_update_report_request_json(body);
    }

    parse_update_report_request_json(body).or_else(|_| parse_update_report_request_form(body))
}

fn parse_report_rule_ids(rule_ids: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for rule_id in rule_ids {
        let rule_id = rule_id.trim();
        if !rule_id.is_empty() && !normalized.iter().any(|existing| existing == rule_id) {
            normalized.push(rule_id.to_string());
        }
    }
    normalized
}

fn parse_instance_rule_texts(raw: &str) -> Option<Vec<String>> {
    let parsed: serde_json::Value = serde_json::from_str(raw).ok()?;
    let items = parsed.as_array()?;
    let mut rules = Vec::with_capacity(items.len());
    for item in items {
        if let Some(text) = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            rules.push(text.to_string());
            continue;
        }
        if let Some(text) = item
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            rules.push(text.to_string());
        }
    }
    (!rules.is_empty()).then_some(rules)
}

async fn load_instance_rule_map(state: &AdminApiState) -> BTreeMap<String, String> {
    let rule_texts = match state.db.get_setting("instance.rules").await {
        Ok(Some(raw)) => parse_instance_rule_texts(&raw).unwrap_or_else(|| {
            DEFAULT_INSTANCE_RULES
                .iter()
                .map(|rule| rule.to_string())
                .collect()
        }),
        _ => DEFAULT_INSTANCE_RULES
            .iter()
            .map(|rule| rule.to_string())
            .collect(),
    };

    rule_texts
        .into_iter()
        .enumerate()
        .map(|(index, text)| ((index + 1).to_string(), text))
        .collect()
}

fn serialize_rule_ids(rule_ids: &[String]) -> Result<Option<String>, AppError> {
    if rule_ids.is_empty() {
        return Ok(None);
    }

    serde_json::to_string(rule_ids)
        .map(Some)
        .map_err(|error| AppError::serialization("admin report rule ids", error))
}

fn deserialize_rule_ids(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

fn build_admin_report_rules(
    rule_ids: &[String],
    rule_map: &BTreeMap<String, String>,
) -> Vec<serde_json::Value> {
    rule_ids
        .iter()
        .filter_map(|rule_id| {
            rule_map.get(rule_id).map(|text| {
                serde_json::json!({
                    "id": rule_id,
                    "text": text
                })
            })
        })
        .collect()
}

fn default_admin_report_state(report_id: &str, created_at: DateTime<Utc>) -> AdminReportState {
    AdminReportState {
        report_id: report_id.to_string(),
        category: "other".to_string(),
        comment: String::new(),
        forwarded: false,
        rule_ids_json: None,
        assigned_account_id: None,
        action_taken: false,
        action_taken_at: None,
        action_taken_by_account_id: None,
        updated_at: created_at,
    }
}

async fn resolve_admin_action_target(
    state: &AdminApiState,
    raw_id: &str,
) -> Result<AdminActionTarget, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let local_actor_uri = format!(
        "{}/users/{}",
        state.config.server.base_url(),
        account.username
    );
    let local_address = format!("{}@{}", account.username, state.config.server.domain);
    let trimmed = raw_id.trim();

    if trimmed.eq_ignore_ascii_case(account.id.as_str())
        || trimmed.eq_ignore_ascii_case(local_actor_uri.as_str())
        || normalize_account_address(trimmed).ok().as_deref()
            == normalize_account_address(&local_address).ok().as_deref()
    {
        return Ok(AdminActionTarget::Local(account));
    }

    let (address, actor_uri, inbox_uri) =
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            let address = parse_actor_uri_account_address(trimmed)
                .ok_or_else(|| AppError::Validation("Invalid account ID format".to_string()))?;
            let profile = state.profile_cache.get_by_uri(trimmed).await;
            (
                normalize_account_address(&address)?,
                Some(trimmed.trim_end_matches('/').to_string()),
                profile.map(|value| value.inbox_uri.clone()),
            )
        } else {
            let normalized = normalize_account_address(trimmed)?;
            let profile = state.profile_cache.get(&normalized).await;
            (
                normalized,
                profile.as_ref().map(|value| value.uri.clone()),
                profile.map(|value| value.inbox_uri.clone()),
            )
        };

    Ok(AdminActionTarget::Remote {
        address,
        actor_uri,
        inbox_uri,
    })
}

fn admin_identity_key(identity: &str) -> Option<String> {
    normalize_account_address(identity)
        .ok()
        .map(|value| value.to_ascii_lowercase())
        .or_else(|| {
            parse_actor_uri_account_address(identity)
                .and_then(|value| normalize_account_address(&value).ok())
                .map(|value| value.to_ascii_lowercase())
        })
        .or_else(|| {
            let trimmed = identity.trim();
            (!trimmed.is_empty()).then(|| trimmed.trim_end_matches('/').to_ascii_lowercase())
        })
}

fn admin_account_matches_filters(account: &AdminAccount, params: &AdminAccountParams) -> bool {
    let is_local = account.domain.is_none();
    let is_pending = !account.approved || !account.confirmed;
    let is_active = !is_pending && !account.disabled && !account.suspended;

    if let Some(local) = params.local
        && local != is_local
    {
        return false;
    }
    if let Some(remote) = params.remote
        && remote == is_local
    {
        return false;
    }
    if let Some(active) = params.active
        && active != is_active
    {
        return false;
    }
    if let Some(pending) = params.pending
        && pending != is_pending
    {
        return false;
    }
    if let Some(disabled) = params.disabled
        && disabled != account.disabled
    {
        return false;
    }
    if let Some(silenced) = params.silenced
        && silenced != account.silenced
    {
        return false;
    }
    if let Some(suspended) = params.suspended
        && suspended != account.suspended
    {
        return false;
    }
    if let Some(username) = params.username.as_deref() {
        let needle = username.trim().to_ascii_lowercase();
        if !needle.is_empty() && !account.username.to_ascii_lowercase().contains(&needle) {
            return false;
        }
    }
    if let Some(display_name) = params.display_name.as_deref() {
        let needle = display_name.trim().to_ascii_lowercase();
        let haystack = account.account["display_name"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !needle.is_empty() && !haystack.contains(&needle) {
            return false;
        }
    }
    if let Some(email) = params.email.as_deref() {
        let needle = email.trim().to_ascii_lowercase();
        let haystack = account
            .email
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !needle.is_empty() && !haystack.contains(&needle) {
            return false;
        }
    }
    if let Some(ip) = params.ip.as_deref() {
        let needle = ip.trim().to_ascii_lowercase();
        let haystack = account
            .ip
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !needle.is_empty() && !haystack.contains(&needle) {
            return false;
        }
    }

    true
}

fn normalize_admin_account_cursor(raw: &str) -> String {
    let trimmed = raw.trim();
    normalize_account_address(trimmed)
        .unwrap_or_else(|_| trimmed.trim_end_matches('/').to_ascii_lowercase())
}

fn admin_account_cursor(account: &AdminAccount) -> String {
    normalize_admin_account_cursor(&account.id)
}

fn apply_admin_account_pagination(
    mut accounts: Vec<AdminAccount>,
    params: &AdminAccountParams,
) -> Vec<AdminAccount> {
    accounts.sort_by(|left, right| admin_account_cursor(right).cmp(&admin_account_cursor(left)));

    let max_id = params.max_id.as_deref().map(normalize_admin_account_cursor);
    let min_id = params
        .min_id
        .as_deref()
        .or(params.since_id.as_deref())
        .map(normalize_admin_account_cursor);

    accounts
        .into_iter()
        .filter(|account| {
            let cursor = admin_account_cursor(account);
            max_id.as_ref().map(|value| cursor < *value).unwrap_or(true)
                && min_id.as_ref().map(|value| cursor > *value).unwrap_or(true)
        })
        .collect()
}

async fn build_local_admin_account(state: &AdminApiState) -> Result<AdminAccount, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    Ok(AdminAccount {
        id: account.id.to_string(),
        username: account.username.clone(),
        domain: None,
        created_at: account.created_at.to_rfc3339(),
        email: None,
        ip: None,
        role: "owner".to_string(),
        confirmed: true,
        suspended: false,
        silenced: false,
        disabled: false,
        approved: true,
        account: serde_json::to_value(crate::api::account_to_response_with_stats(
            &account,
            &state.config,
            account_stats,
        ))
        .map_err(|error| AppError::serialization("admin local account", error))?,
    })
}

async fn build_remote_admin_account(
    state: &AdminApiState,
    identity: &str,
    suspended: bool,
    silenced: bool,
) -> Result<AdminAccount, AppError> {
    let response = resolve_account_response_for_identity(
        state.config.as_ref(),
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        Some(state.federation_fetch_client.as_ref()),
        identity,
    )
    .await
    .or_else(|| build_remote_account_placeholder_response(identity, &state.config, 0))
    .ok_or(AppError::NotFound)?;
    let domain = response
        .acct
        .split_once('@')
        .map(|(_, domain)| domain.to_string());

    Ok(AdminAccount {
        id: response.id.clone(),
        username: response.username.clone(),
        domain,
        created_at: response.created_at.to_rfc3339(),
        email: None,
        ip: None,
        role: "user".to_string(),
        confirmed: true,
        suspended,
        silenced,
        disabled: false,
        approved: true,
        account: serde_json::to_value(response)
            .map_err(|error| AppError::serialization("admin remote account", error))?,
    })
}

async fn collect_remote_admin_candidates(
    state: &AdminApiState,
) -> Result<Vec<AdminRemoteAccountCandidate>, AppError> {
    let mut candidates = BTreeMap::<String, AdminRemoteAccountCandidate>::new();
    let blocked_details = state.db.get_blocked_account_details(500).await?;
    let muted_details = state.db.get_muted_account_details(500).await?;
    let blocked_keys = blocked_details
        .iter()
        .filter_map(|(address, _, _)| admin_identity_key(address))
        .collect::<HashSet<_>>();
    let muted_keys = muted_details
        .iter()
        .filter_map(|(address, _)| admin_identity_key(address))
        .collect::<HashSet<_>>();

    let mut insert_candidate = |identity: String, suspended: bool, silenced: bool| {
        let Some(key) = admin_identity_key(&identity) else {
            return;
        };
        candidates
            .entry(key)
            .and_modify(|candidate| {
                candidate.suspended |= suspended;
                candidate.silenced |= silenced;
            })
            .or_insert(AdminRemoteAccountCandidate {
                identity,
                suspended,
                silenced,
            });
    };

    let moderation_state_for = |address: &str| {
        let key = admin_identity_key(address);
        let suspended = key
            .as_ref()
            .is_some_and(|normalized| blocked_keys.contains(normalized));
        let silenced = key
            .as_ref()
            .is_some_and(|normalized| muted_keys.contains(normalized));
        (suspended, silenced)
    };

    for profile in state.db.list_remote_profiles().await? {
        let identity = if profile.uri.trim().is_empty() {
            profile.address.clone()
        } else {
            profile.uri.clone()
        };
        let (suspended, silenced) = moderation_state_for(&profile.address);
        insert_candidate(identity, suspended, silenced);
    }

    for (address, actor_uri, _) in blocked_details {
        insert_candidate(actor_uri.unwrap_or(address), true, false);
    }
    for (address, actor_uri) in muted_details {
        insert_candidate(actor_uri.unwrap_or(address), false, true);
    }
    for (address, actor_uri) in state.db.get_follow_request_details(500).await? {
        let identity = actor_uri.unwrap_or(address.clone());
        let (suspended, silenced) = moderation_state_for(&address);
        insert_candidate(identity, suspended, silenced);
    }

    let mut seen_notification_ids = HashSet::new();
    let mut cursor: Option<String> = None;
    loop {
        let notifications = state
            .db
            .get_notifications(200, cursor.as_deref(), false)
            .await?;
        if notifications.is_empty() {
            break;
        }
        let reached_end = notifications.len() < 200;
        cursor = notifications
            .last()
            .map(|notification| notification.id.clone());

        for notification in notifications
            .into_iter()
            .filter(|notification| notification.notification_type == NotificationType::AdminReport)
        {
            if !seen_notification_ids.insert(notification.id.clone()) {
                continue;
            }
            let (suspended, silenced) = moderation_state_for(&notification.origin_account_address);
            insert_candidate(notification.origin_account_address, suspended, silenced);
        }

        if reached_end {
            break;
        }
    }

    Ok(candidates.into_values().collect())
}

#[derive(Debug, Deserialize, Default)]
pub struct AdminReportParams {
    #[serde(rename = "resolved")]
    resolved: Option<bool>,
    #[serde(rename = "account_id")]
    account_id: Option<String>,
    #[serde(rename = "target_account_id")]
    target_account_id: Option<String>,
    #[serde(rename = "max_id")]
    max_id: Option<String>,
    #[serde(rename = "since_id")]
    since_id: Option<String>,
    #[serde(rename = "min_id")]
    min_id: Option<String>,
    #[serde(rename = "limit")]
    limit: Option<usize>,
}

#[derive(Debug)]
struct AdminReportEntry {
    report: AdminReport,
    created_at: DateTime<Utc>,
}

fn admin_report_matches_filters(report: &AdminReport, params: &AdminReportParams) -> bool {
    if let Some(resolved) = params.resolved
        && resolved != report.action_taken
    {
        return false;
    }

    if let Some(account_id) = params.account_id.as_deref() {
        let needle = normalize_admin_account_cursor(account_id);
        let report_account_id = report.account["id"]
            .as_str()
            .map(normalize_admin_account_cursor)
            .unwrap_or_default();
        if report_account_id != needle {
            return false;
        }
    }

    if let Some(target_account_id) = params.target_account_id.as_deref() {
        let needle = normalize_admin_account_cursor(target_account_id);
        let report_target_account_id = report.target_account["id"]
            .as_str()
            .map(normalize_admin_account_cursor)
            .unwrap_or_default();
        if report_target_account_id != needle {
            return false;
        }
    }

    true
}

async fn admin_report_cursor_tuple(
    state: &AdminApiState,
    cursor_id: &str,
) -> Result<Option<(DateTime<Utc>, String)>, AppError> {
    Ok(state
        .db
        .get_notification(cursor_id)
        .await?
        .map(|notification| (notification.created_at, notification.id)))
}

async fn apply_admin_report_pagination(
    state: &AdminApiState,
    entries: Vec<AdminReportEntry>,
    params: &AdminReportParams,
) -> Result<Vec<AdminReport>, AppError> {
    let max_cursor = match params.max_id.as_deref() {
        Some(cursor_id) => admin_report_cursor_tuple(state, cursor_id).await?,
        None => None,
    };
    let min_cursor = match params.min_id.as_deref().or(params.since_id.as_deref()) {
        Some(cursor_id) => admin_report_cursor_tuple(state, cursor_id).await?,
        None => None,
    };

    Ok(entries
        .into_iter()
        .filter(|entry| {
            let cursor = (entry.created_at, entry.report.id.clone());
            max_cursor
                .as_ref()
                .map(|value| cursor < *value)
                .unwrap_or(true)
                && min_cursor
                    .as_ref()
                    .map(|value| cursor > *value)
                    .unwrap_or(true)
        })
        .map(|entry| entry.report)
        .collect())
}

/// GET /api/v1/admin/accounts
pub async fn list_accounts(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<AdminAccountParams>,
) -> Result<Json<Vec<AdminAccount>>, AppError> {
    let limit = params.limit.unwrap_or(100).min(200);
    let mut accounts = Vec::new();

    let local_account = build_local_admin_account(&state).await?;
    if admin_account_matches_filters(&local_account, &params) {
        accounts.push(local_account);
    }

    for candidate in collect_remote_admin_candidates(&state).await? {
        let account = build_remote_admin_account(
            &state,
            &candidate.identity,
            candidate.suspended,
            candidate.silenced,
        )
        .await?;
        if admin_account_matches_filters(&account, &params) {
            accounts.push(account);
        }
    }

    let paginated = apply_admin_account_pagination(accounts, &params);
    Ok(Json(paginated.into_iter().take(limit).collect()))
}

/// GET /api/v1/admin/accounts/:id
pub async fn get_account(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<AdminAccount>, AppError> {
    match resolve_admin_action_target(&state, &id).await? {
        AdminActionTarget::Local(_) => Ok(Json(build_local_admin_account(&state).await?)),
        AdminActionTarget::Remote {
            address, actor_uri, ..
        } => {
            let default_port = default_port_for_protocol(&state.config.server.protocol);
            let identity = actor_uri.unwrap_or(address.clone());
            let suspended = state.db.is_account_blocked(&address, default_port).await?;
            let silenced = state.db.is_account_muted(&address, default_port).await?;
            Ok(Json(
                build_remote_admin_account(&state, &identity, suspended, silenced).await?,
            ))
        }
    }
}

async fn apply_admin_account_action(
    state: &AdminApiState,
    id: &str,
    action: &str,
    report_id: Option<&str>,
) -> Result<(), AppError> {
    let target = resolve_admin_action_target(state, id).await?;
    let default_port = default_port_for_protocol(&state.config.server.protocol);

    match target {
        AdminActionTarget::Local(account) => {
            return Err(AppError::Validation(format!(
                "admin action `{}` is not supported for the local owner account `{}`",
                action, account.username
            )));
        }
        AdminActionTarget::Remote {
            address,
            actor_uri,
            inbox_uri,
        } => match action {
            "suspend" | "disable" => {
                state
                    .db
                    .block_account_with_remote_metadata(
                        &address,
                        actor_uri.as_deref(),
                        inbox_uri.as_deref(),
                        default_port,
                    )
                    .await?;
            }
            "unsuspend" | "enable" => {
                state.db.unblock_account(&address, default_port).await?;
            }
            "silence" => {
                state
                    .db
                    .mute_account_with_actor_uri(
                        &address,
                        true,
                        None,
                        actor_uri.as_deref(),
                        default_port,
                    )
                    .await?;
            }
            "unsilence" => {
                state.db.unmute_account(&address, default_port).await?;
            }
            "none" | "sensitive" | "unsensitive" => {}
            _ => {
                return Err(AppError::Validation(format!(
                    "unsupported admin account action `{}`",
                    action
                )));
            }
        },
    }

    if let Some(report_id) = report_id {
        let _ = set_report_resolution(state, report_id, true).await?;
    }

    Ok(())
}

/// POST /api/v1/admin/accounts/:id/action
pub async fn account_action(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    let req = parse_admin_action_request(&headers, &body)?;
    let action = req.action.trim().to_ascii_lowercase();
    if action.is_empty() {
        return Err(AppError::Validation("type is required".to_string()));
    }

    apply_admin_account_action(&state, &id, &action, req.report_id.as_deref()).await?;
    Ok(Json(serde_json::json!({})))
}

async fn admin_account_action_response(
    state: &AdminApiState,
    id: &str,
    action: &str,
) -> Result<AdminAccount, AppError> {
    apply_admin_account_action(state, id, action, None).await?;
    match resolve_admin_action_target(state, id).await? {
        AdminActionTarget::Local(_) => build_local_admin_account(state).await,
        AdminActionTarget::Remote {
            address, actor_uri, ..
        } => {
            let default_port = default_port_for_protocol(&state.config.server.protocol);
            let identity = actor_uri.unwrap_or(address.clone());
            let suspended = state.db.is_account_blocked(&address, default_port).await?;
            let silenced = state.db.is_account_muted(&address, default_port).await?;
            build_remote_admin_account(state, &identity, suspended, silenced).await
        }
    }
}

pub async fn enable_account(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<AdminAccount>, AppError> {
    Ok(Json(
        admin_account_action_response(&state, &id, "enable").await?,
    ))
}

pub async fn unsilence_account(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<AdminAccount>, AppError> {
    Ok(Json(
        admin_account_action_response(&state, &id, "unsilence").await?,
    ))
}

pub async fn unsuspend_account(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<AdminAccount>, AppError> {
    Ok(Json(
        admin_account_action_response(&state, &id, "unsuspend").await?,
    ))
}

pub async fn unsensitive_account(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<AdminAccount>, AppError> {
    Ok(Json(
        admin_account_action_response(&state, &id, "unsensitive").await?,
    ))
}

#[derive(Debug, Serialize)]
pub struct AdminReport {
    pub id: String,
    pub action_taken: bool,
    pub action_taken_at: Option<String>,
    pub category: String,
    pub comment: String,
    pub forwarded: bool,
    pub created_at: String,
    pub updated_at: String,
    pub account: serde_json::Value,
    pub target_account: serde_json::Value,
    pub assigned_account: Option<serde_json::Value>,
    pub action_taken_by_account: Option<serde_json::Value>,
    pub statuses: Vec<serde_json::Value>,
    pub rules: Vec<serde_json::Value>,
}

async fn get_report_status(state: &AdminApiState, status_uri: &str) -> Option<Status> {
    state.db.get_status_by_uri(status_uri).await.ok().flatten()
}

async fn build_report_status_response(
    state: &AdminApiState,
    account: &crate::data::Account,
    account_stats: crate::api::AccountStats,
    status: &Status,
) -> Result<serde_json::Value, AppError> {
    let remote_account_stats = crate::api::load_remote_account_stats_map(
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        &state.config.server.protocol,
        std::slice::from_ref(status),
    )
    .await?
    .get(status.account_address.trim())
    .cloned();

    let response = crate::api::build_status_response_with_account_stats_and_remote_stats(
        state.db.as_ref(),
        status,
        account,
        &state.config,
        account_stats,
        remote_account_stats,
        crate::api::StatusInteractions::default(),
    )
    .await?;

    serde_json::to_value(response)
        .map_err(|error| AppError::serialization("admin report status response", error))
}

async fn build_admin_report(
    state: &AdminApiState,
    notification: Notification,
    local_account: &crate::data::Account,
    local_account_stats: crate::api::AccountStats,
    local_admin_account: &AdminAccount,
    local_admin_account_value: &serde_json::Value,
    instance_rule_map: &BTreeMap<String, String>,
) -> Result<AdminReportEntry, AppError> {
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let suspended = state
        .db
        .is_account_blocked(&notification.origin_account_address, default_port)
        .await?;
    let silenced = state
        .db
        .is_account_muted(&notification.origin_account_address, default_port)
        .await?;
    let report_account = build_remote_admin_account(
        state,
        &notification.origin_account_address,
        suspended,
        silenced,
    )
    .await?;
    let account_value = serde_json::to_value(&report_account)
        .map_err(|error| AppError::serialization("admin report account response", error))?;

    let mut statuses = Vec::new();
    if let Some(status_uri) = notification.status_uri.as_deref()
        && let Some(status) = get_report_status(state, status_uri).await
    {
        statuses.push(
            build_report_status_response(state, local_account, local_account_stats, &status)
                .await?,
        );
    }

    let report_state = state
        .db
        .get_admin_report_state(&notification.id)
        .await?
        .unwrap_or_else(|| default_admin_report_state(&notification.id, notification.created_at));
    let rule_ids = deserialize_rule_ids(report_state.rule_ids_json.as_deref());
    let assigned_account = report_state
        .assigned_account_id
        .as_deref()
        .filter(|account_id| *account_id == local_admin_account.id)
        .map(|_| local_admin_account_value.clone());
    let action_taken_by_account = report_state
        .action_taken_by_account_id
        .as_deref()
        .filter(|account_id| *account_id == local_admin_account.id)
        .map(|_| local_admin_account_value.clone());

    Ok(AdminReportEntry {
        created_at: notification.created_at,
        report: AdminReport {
            id: notification.id,
            action_taken: report_state.action_taken,
            action_taken_at: report_state.action_taken_at.map(|value| value.to_rfc3339()),
            category: report_state.category,
            comment: report_state.comment,
            forwarded: report_state.forwarded,
            created_at: notification.created_at.to_rfc3339(),
            updated_at: report_state.updated_at.to_rfc3339(),
            account: account_value,
            target_account: serde_json::to_value(local_admin_account)
                .map_err(|error| AppError::serialization("admin report target account", error))?,
            assigned_account,
            action_taken_by_account,
            statuses,
            rules: build_admin_report_rules(&rule_ids, instance_rule_map),
        },
    })
}

async fn get_admin_report_notification(
    state: &AdminApiState,
    id: &str,
) -> Result<Notification, AppError> {
    state
        .db
        .get_notification(id)
        .await?
        .filter(|notification| notification.notification_type == NotificationType::AdminReport)
        .ok_or(AppError::NotFound)
}

async fn build_single_admin_report(
    state: &AdminApiState,
    id: &str,
) -> Result<AdminReport, AppError> {
    let notification = get_admin_report_notification(state, id).await?;
    let local_account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let local_account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    let local_admin_account = build_local_admin_account(state).await?;
    let local_admin_account_value = serde_json::to_value(&local_admin_account)
        .map_err(|error| AppError::serialization("admin local account response", error))?;
    let instance_rule_map = load_instance_rule_map(state).await;

    Ok(build_admin_report(
        state,
        notification,
        &local_account,
        local_account_stats,
        &local_admin_account,
        &local_admin_account_value,
        &instance_rule_map,
    )
    .await?
    .report)
}

/// GET /api/v1/admin/reports
pub async fn list_reports(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<AdminReportParams>,
) -> Result<Json<Vec<AdminReport>>, AppError> {
    let local_account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    let local_admin_account = build_local_admin_account(&state).await?;
    let local_admin_account_value = serde_json::to_value(&local_admin_account)
        .map_err(|error| AppError::serialization("admin local account response", error))?;
    let instance_rule_map = load_instance_rule_map(&state).await;

    let limit = params.limit.unwrap_or(100).min(200);
    let mut reports = Vec::new();
    let mut cursor: Option<String> = None;
    const FETCH_LIMIT: usize = 200;

    loop {
        let notifications = state
            .db
            .get_notifications(FETCH_LIMIT, cursor.as_deref(), false)
            .await?;
        if notifications.is_empty() {
            break;
        }

        let reached_end = notifications.len() < FETCH_LIMIT;
        cursor = notifications
            .last()
            .map(|notification| notification.id.clone());

        for notification in notifications
            .into_iter()
            .filter(|notification| notification.notification_type == NotificationType::AdminReport)
        {
            let report = build_admin_report(
                &state,
                notification,
                &local_account,
                account_stats,
                &local_admin_account,
                &local_admin_account_value,
                &instance_rule_map,
            )
            .await?;

            if admin_report_matches_filters(&report.report, &params) {
                reports.push(report);
            }
        }

        if reached_end {
            break;
        }
    }

    let paginated = apply_admin_report_pagination(&state, reports, &params).await?;
    Ok(Json(paginated.into_iter().take(limit).collect()))
}

/// GET /api/v1/admin/reports/:id
pub async fn get_report(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<AdminReport>, AppError> {
    Ok(Json(build_single_admin_report(&state, &id).await?))
}

/// PUT /api/v1/admin/reports/:id
pub async fn update_report(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AdminReport>, AppError> {
    let req = parse_update_report_request(&headers, &body)?;
    let notification = get_admin_report_notification(&state, &id).await?;
    let mut report_state = state
        .db
        .get_admin_report_state(&id)
        .await?
        .unwrap_or_else(|| default_admin_report_state(&id, notification.created_at));

    if let Some(category) = req.category.as_deref() {
        report_state.category = normalize_admin_report_category(Some(category))?;
        if report_state.category != "violation" {
            report_state.rule_ids_json = None;
        }
    }

    if !req.rule_ids.is_empty() || report_state.category == "violation" {
        let rule_ids = parse_report_rule_ids(&req.rule_ids);
        report_state.rule_ids_json = serialize_rule_ids(&rule_ids)?;
    }

    report_state.updated_at = Utc::now();
    state.db.save_admin_report_state(&report_state).await?;
    Ok(Json(build_single_admin_report(&state, &id).await?))
}

async fn set_report_assignment(
    state: &AdminApiState,
    id: &str,
    assigned_account_id: Option<String>,
) -> Result<AdminReport, AppError> {
    let notification = get_admin_report_notification(state, id).await?;
    let mut report_state = state
        .db
        .get_admin_report_state(id)
        .await?
        .unwrap_or_else(|| default_admin_report_state(id, notification.created_at));
    report_state.assigned_account_id = assigned_account_id;
    report_state.updated_at = Utc::now();
    state.db.save_admin_report_state(&report_state).await?;
    build_single_admin_report(state, id).await
}

async fn set_report_resolution(
    state: &AdminApiState,
    id: &str,
    action_taken: bool,
) -> Result<AdminReport, AppError> {
    let notification = get_admin_report_notification(state, id).await?;
    let local_account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let mut report_state = state
        .db
        .get_admin_report_state(id)
        .await?
        .unwrap_or_else(|| default_admin_report_state(id, notification.created_at));
    report_state.action_taken = action_taken;
    report_state.action_taken_at = action_taken.then_some(Utc::now());
    report_state.action_taken_by_account_id = action_taken.then_some(local_account.id);
    report_state.updated_at = Utc::now();
    state.db.save_admin_report_state(&report_state).await?;
    build_single_admin_report(state, id).await
}

/// POST /api/v1/admin/reports/:id/assign_to_self
pub async fn assign_report_to_self(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<AdminReport>, AppError> {
    let local_account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    Ok(Json(
        set_report_assignment(&state, &id, Some(local_account.id)).await?,
    ))
}

/// POST /api/v1/admin/reports/:id/unassign
pub async fn unassign_report(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<AdminReport>, AppError> {
    Ok(Json(set_report_assignment(&state, &id, None).await?))
}

/// POST /api/v1/admin/reports/:id/resolve
pub async fn resolve_report(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<AdminReport>, AppError> {
    Ok(Json(set_report_resolution(&state, &id, true).await?))
}

/// POST /api/v1/admin/reports/:id/reopen
pub async fn reopen_report(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<AdminReport>, AppError> {
    Ok(Json(set_report_resolution(&state, &id, false).await?))
}

#[derive(Debug, Serialize)]
pub struct DomainBlock {
    pub id: String,
    pub domain: String,
    pub created_at: String,
    pub severity: String,
    pub reject_media: bool,
    pub reject_reports: bool,
    pub private_comment: Option<String>,
    pub public_comment: Option<String>,
    pub obfuscate: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateDomainBlockRequest {
    pub domain: String,
    pub severity: Option<String>,
    pub reject_media: Option<bool>,
    pub reject_reports: Option<bool>,
    pub private_comment: Option<String>,
    pub public_comment: Option<String>,
    pub obfuscate: Option<bool>,
}

/// GET /api/v1/admin/domain_blocks
pub async fn list_domain_blocks_v1(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<Vec<DomainBlock>>, AppError> {
    let blocks = state.db.get_all_domain_blocks().await?;

    let domain_blocks: Vec<DomainBlock> = blocks
        .into_iter()
        .map(|block| DomainBlock {
            id: block.id,
            domain: block.domain,
            created_at: block.created_at.to_rfc3339(),
            severity: block.severity,
            reject_media: block.reject_media,
            reject_reports: block.reject_reports,
            private_comment: block.private_comment,
            public_comment: block.public_comment,
            obfuscate: block.obfuscate,
        })
        .collect();

    Ok(Json(domain_blocks))
}

/// POST /api/v1/admin/domain_blocks
pub async fn create_domain_block_v1(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Json(req): Json<CreateDomainBlockRequest>,
) -> Result<Json<DomainBlock>, AppError> {
    let normalized_domain = normalize_domain_block_domain(&req.domain)?;
    let severity = normalize_domain_block_severity(req.severity.as_deref())?;
    let block = state
        .db
        .upsert_domain_block(
            &normalized_domain,
            &severity,
            req.reject_media.unwrap_or(true),
            req.reject_reports.unwrap_or(true),
            req.private_comment.as_deref(),
            req.public_comment.as_deref(),
            req.obfuscate.unwrap_or(false),
        )
        .await?;

    Ok(Json(DomainBlock {
        id: block.id,
        domain: block.domain,
        created_at: block.created_at.to_rfc3339(),
        severity: block.severity,
        reject_media: block.reject_media,
        reject_reports: block.reject_reports,
        private_comment: block.private_comment,
        public_comment: block.public_comment,
        obfuscate: block.obfuscate,
    }))
}

/// DELETE /api/v1/admin/domain_blocks/:id
pub async fn delete_domain_block_v1(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !state.db.delete_domain_block_by_id(&id).await? {
        return Err(AppError::NotFound);
    }

    Ok(Json(serde_json::json!({})))
}
