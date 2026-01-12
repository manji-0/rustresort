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
    // TODO:
    // 1. Call backup service
    // 2. Return backup info
    Err(AppError::NotFound)
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
    // TODO: List backups from R2
    Err(AppError::NotFound)
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
    // TODO: Add domain to block list
    Err(AppError::NotFound)
}

/// DELETE /api/admin/domain_blocks/:domain
async fn unblock_domain(
    State(_state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    axum::extract::Path(_domain): axum::extract::Path<String>,
) -> Result<(), AppError> {
    // TODO: Remove domain from block list
    Err(AppError::NotFound)
}

/// GET /api/admin/domain_blocks
async fn list_domain_blocks(
    State(_state): State<AppState>,
    CurrentUser(_user): CurrentUser,
) -> Result<Json<Vec<String>>, AppError> {
    // TODO: List blocked domains
    Err(AppError::NotFound)
}
