//! Admin API endpoints
//!
//! Instance administration endpoints.
//! All routes require authentication.

use axum::{
    Router,
    extract::State,
    response::Json,
    routing::{get, post},
};
use chrono::Utc;

use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::AppError;

/// Create admin router
///
/// Routes:
/// - POST /api/admin/backup - Trigger manual backup
/// - GET /api/admin/backups - List backups
/// - POST /api/admin/domain_blocks - Block domain
/// - DELETE /api/admin/domain_blocks/:domain - Unblock domain
/// - GET /api/admin/domain_blocks - List blocked domains
pub fn admin_router() -> Router<AppState> {
    Router::new()
        // Backup
        .route("/backup", post(trigger_backup))
        .route("/backups", get(list_backups))
        // Domain blocks
        .route("/domain_blocks", post(block_domain))
        .route(
            "/domain_blocks/:domain",
            axum::routing::delete(unblock_domain),
        )
        .route("/domain_blocks", get(list_domain_blocks))
}

// =============================================================================
// Backup
// =============================================================================

/// POST /api/admin/backup
///
/// Triggers a manual database backup.
async fn trigger_backup(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
) -> Result<Json<BackupResponse>, AppError> {
    let key = state.backup.backup().await?;
    Ok(Json(BackupResponse {
        success: true,
        key,
        timestamp: Utc::now().to_rfc3339(),
    }))
}

/// Backup response
#[derive(Debug, serde::Serialize)]
pub struct BackupResponse {
    pub success: bool,
    pub key: String,
    pub timestamp: String,
}

/// GET /api/admin/backups
///
/// Lists all available backups.
async fn list_backups(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
) -> Result<Json<Vec<BackupInfo>>, AppError> {
    let backups = state
        .backup
        .list_backups()
        .await?
        .into_iter()
        .map(|backup| BackupInfo {
            key: backup.key,
            size: backup.size,
            created_at: backup.created_at.to_rfc3339(),
        })
        .collect();
    Ok(Json(backups))
}

/// Backup info
#[derive(Debug, serde::Serialize)]
pub struct BackupInfo {
    pub key: String,
    pub size: u64,
    pub created_at: String,
}

// =============================================================================
// Domain blocks
// =============================================================================

/// Block domain request
#[derive(Debug, serde::Deserialize)]
struct BlockDomainRequest {
    domain: String,
}

fn normalize_domain(domain: &str) -> Result<String, AppError> {
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

/// POST /api/admin/domain_blocks
async fn block_domain(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    Json(req): Json<BlockDomainRequest>,
) -> Result<(), AppError> {
    let domain = normalize_domain(&req.domain)?;
    if !state.db.is_domain_blocked(&domain).await? {
        state.db.block_domain(&domain).await?;
    }
    Ok(())
}

/// DELETE /api/admin/domain_blocks/:domain
async fn unblock_domain(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    axum::extract::Path(domain): axum::extract::Path<String>,
) -> Result<(), AppError> {
    let domain = normalize_domain(&domain)?;
    state.db.unblock_domain(&domain).await?;
    Ok(())
}

/// GET /api/admin/domain_blocks
async fn list_domain_blocks(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
) -> Result<Json<Vec<String>>, AppError> {
    let domains = state.db.get_blocked_domains().await?;
    Ok(Json(domains))
}

#[cfg(test)]
mod tests {
    use super::normalize_domain;

    #[test]
    fn normalize_domain_trims_and_lowercases() {
        let domain = normalize_domain("  ExAmple.COM. ").expect("valid domain");
        assert_eq!(domain, "example.com");
    }

    #[test]
    fn normalize_domain_rejects_invalid_hostname() {
        let error = normalize_domain("http://example.com").expect_err("invalid domain");
        assert!(matches!(
            error,
            crate::error::AppError::Validation(message)
                if message.contains("valid DNS hostname")
        ));
    }
}
