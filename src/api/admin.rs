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
    State(_state): State<AppState>,
    CurrentUser(_user): CurrentUser,
) -> Result<Json<BackupResponse>, AppError> {
    Err(AppError::NotImplemented(
        "admin backup endpoint is not implemented yet".to_string(),
    ))
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
    State(_state): State<AppState>,
    CurrentUser(_user): CurrentUser,
) -> Result<Json<Vec<BackupInfo>>, AppError> {
    Err(AppError::NotImplemented(
        "admin backups listing endpoint is not implemented yet".to_string(),
    ))
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
#[allow(dead_code)]
struct BlockDomainRequest {
    domain: String,
}

/// POST /api/admin/domain_blocks
async fn block_domain(
    State(_state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    Json(_req): Json<BlockDomainRequest>,
) -> Result<(), AppError> {
    Err(AppError::NotImplemented(
        "admin domain block endpoint is not implemented yet".to_string(),
    ))
}

/// DELETE /api/admin/domain_blocks/:domain
async fn unblock_domain(
    State(_state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    axum::extract::Path(_domain): axum::extract::Path<String>,
) -> Result<(), AppError> {
    Err(AppError::NotImplemented(
        "admin domain unblock endpoint is not implemented yet".to_string(),
    ))
}

/// GET /api/admin/domain_blocks
async fn list_domain_blocks(
    State(_state): State<AppState>,
    CurrentUser(_user): CurrentUser,
) -> Result<Json<Vec<String>>, AppError> {
    Err(AppError::NotImplemented(
        "admin domain blocks listing endpoint is not implemented yet".to_string(),
    ))
}
