//! Mastodon-compatible Admin API endpoints

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::{Deserialize, Serialize};

use crate::AdminApiState;
use crate::auth::CurrentUser;
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
    State(_state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<AdminActionRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(serde_json::json!({
        "action": req.action,
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

/// GET /api/v1/admin/reports
pub async fn list_reports(
    State(_state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<Vec<AdminReport>>, AppError> {
    Ok(Json(vec![]))
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
    use crate::data::EntityId;
    use chrono::Utc;

    let id = EntityId::new_string();
    state.db.insert_domain_block(&req.domain).await?;

    Ok(Json(DomainBlock {
        id,
        domain: req.domain,
        created_at: Utc::now().to_rfc3339(),
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
    State(_state): State<AdminApiState>,
    CurrentUser(_session): CurrentUser,
    Path(_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(serde_json::json!({})))
}
