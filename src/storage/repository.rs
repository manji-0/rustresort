//! Repository traits for storage backends.
//!
//! These traits decouple service and handler logic from concrete storage
//! implementations to make dependency injection and testing easier.

use axum::async_trait;

use crate::error::AppError;

use super::{BackupService, MediaStorage};
use crate::storage::BackupInfo;

/// Storage operations required by service and API layers.
#[async_trait]
pub trait MediaStorageRepository: Send + Sync {
    async fn upload(
        &self,
        key: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> Result<String, AppError>;
    async fn upload_avatar(&self, id: &str, data: Vec<u8>) -> Result<(String, String), AppError>;
    async fn upload_header(&self, id: &str, data: Vec<u8>) -> Result<(String, String), AppError>;
    async fn delete(&self, key: &str) -> Result<(), AppError>;
    fn get_public_url(&self, key: &str) -> String;
}

/// Storage operations required by system admin API and background tasks.
#[async_trait]
pub trait BackupRepository: Send + Sync {
    async fn backup(&self) -> Result<String, AppError>;
    async fn list_backups(&self) -> Result<Vec<BackupInfo>, AppError>;
}

#[async_trait]
impl MediaStorageRepository for MediaStorage {
    async fn upload(
        &self,
        key: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> Result<String, AppError> {
        MediaStorage::upload(self, key, data, content_type).await
    }

    async fn upload_avatar(&self, id: &str, data: Vec<u8>) -> Result<(String, String), AppError> {
        MediaStorage::upload_avatar(self, id, data).await
    }

    async fn upload_header(&self, id: &str, data: Vec<u8>) -> Result<(String, String), AppError> {
        MediaStorage::upload_header(self, id, data).await
    }

    async fn delete(&self, key: &str) -> Result<(), AppError> {
        MediaStorage::delete(self, key).await
    }

    fn get_public_url(&self, key: &str) -> String {
        MediaStorage::get_public_url(self, key)
    }
}

#[async_trait]
impl BackupRepository for BackupService {
    async fn backup(&self) -> Result<String, AppError> {
        BackupService::backup(self).await
    }

    async fn list_backups(&self) -> Result<Vec<BackupInfo>, AppError> {
        BackupService::list_backups(self).await
    }
}
