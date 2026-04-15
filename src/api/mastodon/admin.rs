//! Mastodon-compatible Admin API endpoints

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::{Deserialize, Serialize};

use super::accounts::{
    build_remote_account_placeholder_response, default_port_for_protocol,
    normalize_account_address, parse_actor_uri_account_address,
    resolve_account_response_for_identity,
};
use crate::AdminApiState;
use crate::auth::CurrentUser;
use crate::data::{NotificationType, Status};
use crate::error::AppError;

#[derive(Debug, Deserialize)]
pub struct AdminAccountParams {
    #[serde(rename = "local")]
    _local: Option<bool>,
    #[serde(rename = "remote")]
    _remote: Option<bool>,
    #[serde(rename = "active")]
    _active: Option<bool>,
    #[serde(rename = "pending")]
    _pending: Option<bool>,
    #[serde(rename = "disabled")]
    _disabled: Option<bool>,
    #[serde(rename = "silenced")]
    _silenced: Option<bool>,
    #[serde(rename = "suspended")]
    _suspended: Option<bool>,
    #[serde(rename = "username")]
    _username: Option<String>,
    #[serde(rename = "display_name")]
    _display_name: Option<String>,
    #[serde(rename = "email")]
    _email: Option<String>,
    #[serde(rename = "ip")]
    _ip: Option<String>,
    #[serde(rename = "max_id")]
    _max_id: Option<String>,
    #[serde(rename = "since_id")]
    _since_id: Option<String>,
    #[serde(rename = "min_id")]
    _min_id: Option<String>,
    #[serde(rename = "limit")]
    _limit: Option<usize>,
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

#[derive(Debug, Deserialize)]
pub struct AdminActionRequest {
    pub action: String,
    #[serde(rename = "reason")]
    pub _reason: Option<String>,
}

enum AdminActionTarget {
    Local(crate::data::Account),
    Remote {
        address: String,
        actor_uri: Option<String>,
        inbox_uri: Option<String>,
    },
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

/// GET /api/v1/admin/accounts
pub async fn list_accounts(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Query(_params): Query<AdminAccountParams>,
) -> Result<Json<Vec<AdminAccount>>, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    let admin_account = AdminAccount {
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
        .unwrap(),
    };

    Ok(Json(vec![admin_account]))
}

/// GET /api/v1/admin/accounts/:id
pub async fn get_account(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<AdminAccount>, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;

    if account.id.as_str() != id {
        return Err(AppError::NotFound);
    }

    let admin_account = AdminAccount {
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
        .unwrap(),
    };

    Ok(Json(admin_account))
}

/// POST /api/v1/admin/accounts/:id/action
pub async fn account_action(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<AdminActionRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let target = resolve_admin_action_target(&state, &id).await?;
    let action = req.action.trim().to_ascii_lowercase();
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
        } => match action.as_str() {
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
            _ => {
                return Err(AppError::Validation(format!(
                    "unsupported admin account action `{}`",
                    action
                )));
            }
        },
    }

    Ok(Json(serde_json::json!({
        "action": action,
        "account_id": id,
        "status": "completed"
    })))
}

#[derive(Debug, Serialize)]
pub struct AdminReport {
    pub id: String,
    pub action_taken: bool,
    pub comment: String,
    pub created_at: String,
    pub updated_at: String,
    pub account: serde_json::Value,
    pub target_account: serde_json::Value,
    pub assigned_account: Option<serde_json::Value>,
    pub action_taken_by_account: Option<serde_json::Value>,
    pub statuses: Vec<serde_json::Value>,
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

/// GET /api/v1/admin/reports
pub async fn list_reports(
    State(state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<Vec<AdminReport>>, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    let local_account_response =
        crate::api::account_to_response_with_stats(&account, &state.config, account_stats);
    let local_account_value = serde_json::to_value(&local_account_response)
        .map_err(|error| AppError::serialization("admin local account response", error))?;

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
            let report_account = resolve_account_response_for_identity(
                state.config.as_ref(),
                state.db.as_ref(),
                state.profile_cache.as_ref(),
                Some(state.federation_fetch_client.as_ref()),
                &notification.origin_account_address,
            )
            .await
            .or_else(|| {
                build_remote_account_placeholder_response(
                    &notification.origin_account_address,
                    &state.config,
                    0,
                )
            })
            .unwrap_or_else(|| local_account_response.clone());
            let account_value = serde_json::to_value(&report_account)
                .map_err(|error| AppError::serialization("admin report account response", error))?;

            let mut statuses = Vec::new();
            if let Some(status_uri) = notification.status_uri.as_deref()
                && let Some(status) = get_report_status(&state, status_uri).await
            {
                statuses.push(
                    build_report_status_response(&state, &account, account_stats, &status).await?,
                );
            }

            reports.push(AdminReport {
                id: notification.id,
                action_taken: false,
                comment: String::new(),
                created_at: notification.created_at.to_rfc3339(),
                updated_at: notification.created_at.to_rfc3339(),
                account: account_value,
                target_account: local_account_value.clone(),
                assigned_account: None,
                action_taken_by_account: None,
                statuses,
            });
        }

        if reached_end {
            break;
        }
    }

    Ok(Json(reports))
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
        .map(|(id, domain, created_at)| DomainBlock {
            id,
            domain,
            created_at: created_at.to_rfc3339(),
            severity: "suspend".to_string(),
            reject_media: true,
            reject_reports: true,
            private_comment: None,
            public_comment: None,
            obfuscate: false,
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
    let (id, domain, created_at) = state
        .db
        .create_or_get_domain_block(&normalized_domain)
        .await?;

    Ok(Json(DomainBlock {
        id,
        domain,
        created_at: created_at.to_rfc3339(),
        severity: req.severity.unwrap_or_else(|| "suspend".to_string()),
        reject_media: req.reject_media.unwrap_or(true),
        reject_reports: req.reject_reports.unwrap_or(true),
        private_comment: req.private_comment,
        public_comment: req.public_comment,
        obfuscate: req.obfuscate.unwrap_or(false),
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
