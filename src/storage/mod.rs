//! Cloudflare R2 storage module
//!
//! Handles:
//! - Media file upload/download (public bucket)
//! - Database backup (private bucket)

mod backup;
mod media;

pub use backup::BackupService;
pub use media::MediaStorage;
