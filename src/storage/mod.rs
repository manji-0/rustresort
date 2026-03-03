//! Cloudflare R2 storage module
//!
//! Handles:
//! - Media file upload/download (public bucket)
//! - Database backup (private bucket)

mod repository;

pub use repository::{BackupRepository, MediaStorageRepository};
pub use rustresort_storage::BackupInfo;

use std::path::PathBuf;

use crate::error::AppError;

fn map_storage_error(error: rustresort_storage::StorageError) -> AppError {
    match error {
        rustresort_storage::StorageError::Validation(message) => AppError::Validation(message),
        rustresort_storage::StorageError::Config(message) => AppError::Config(message),
        rustresort_storage::StorageError::Encryption(message) => AppError::Encryption(message),
        rustresort_storage::StorageError::Database(message)
        | rustresort_storage::StorageError::Storage(message) => AppError::Storage(message),
    }
}

fn cloudflare_config(
    config: &crate::config::CloudflareConfig,
) -> rustresort_storage::CloudflareConfig {
    rustresort_storage::CloudflareConfig {
        account_id: config.account_id.clone(),
        r2_access_key_id: config.r2_access_key_id.clone(),
        r2_secret_access_key: config.r2_secret_access_key.clone(),
    }
}

fn media_storage_config(
    config: &crate::config::MediaStorageConfig,
) -> rustresort_storage::MediaStorageConfig {
    rustresort_storage::MediaStorageConfig {
        bucket: config.bucket.clone(),
        public_url: config.public_url.clone(),
    }
}

fn backup_storage_config(
    config: &crate::config::BackupStorageConfig,
) -> rustresort_storage::BackupStorageConfig {
    rustresort_storage::BackupStorageConfig {
        enabled: config.enabled,
        bucket: config.bucket.clone(),
        interval_seconds: config.interval_seconds,
        retention_count: config.retention_count,
        encryption: rustresort_storage::BackupEncryptionConfig {
            enabled: config.encryption.enabled,
            key: config.encryption.key.clone(),
        },
    }
}

/// Media storage service.
pub struct MediaStorage {
    inner: rustresort_storage::MediaStorage,
}

impl MediaStorage {
    pub async fn new(
        config: &crate::config::MediaStorageConfig,
        cloudflare: &crate::config::CloudflareConfig,
    ) -> Result<Self, AppError> {
        let config = media_storage_config(config);
        let cloudflare = cloudflare_config(cloudflare);
        let inner = rustresort_storage::MediaStorage::new(&config, &cloudflare)
            .await
            .map_err(map_storage_error)?;
        Ok(Self { inner })
    }

    pub async fn upload(
        &self,
        key: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> Result<String, AppError> {
        self.inner
            .upload(key, data, content_type)
            .await
            .map_err(map_storage_error)
    }

    pub async fn upload_avatar(
        &self,
        id: &str,
        data: Vec<u8>,
    ) -> Result<(String, String), AppError> {
        self.inner
            .upload_avatar(id, data)
            .await
            .map_err(map_storage_error)
    }

    pub async fn upload_header(
        &self,
        id: &str,
        data: Vec<u8>,
    ) -> Result<(String, String), AppError> {
        self.inner
            .upload_header(id, data)
            .await
            .map_err(map_storage_error)
    }

    pub async fn upload_attachment(
        &self,
        id: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> Result<(String, String), AppError> {
        self.inner
            .upload_attachment(id, data, content_type)
            .await
            .map_err(map_storage_error)
    }

    pub async fn upload_thumbnail(
        &self,
        id: &str,
        data: Vec<u8>,
    ) -> Result<(String, String), AppError> {
        self.inner
            .upload_thumbnail(id, data)
            .await
            .map_err(map_storage_error)
    }

    pub async fn delete(&self, key: &str) -> Result<(), AppError> {
        self.inner.delete(key).await.map_err(map_storage_error)
    }

    pub fn get_public_url(&self, key: &str) -> String {
        self.inner.get_public_url(key)
    }
}

/// Backup service for SQLite database.
pub struct BackupService {
    inner: rustresort_storage::BackupService,
}

impl BackupService {
    pub async fn new(
        config: &crate::config::BackupStorageConfig,
        cloudflare: &crate::config::CloudflareConfig,
        db_path: PathBuf,
    ) -> Result<Self, AppError> {
        let config = backup_storage_config(config);
        let cloudflare = cloudflare_config(cloudflare);
        let inner = rustresort_storage::BackupService::new(&config, &cloudflare, db_path)
            .await
            .map_err(map_storage_error)?;
        Ok(Self { inner })
    }

    pub async fn run(&self) {
        self.inner.run().await
    }

    pub async fn backup_now(&self) -> Result<String, AppError> {
        self.inner.backup_now().await.map_err(map_storage_error)
    }

    pub async fn backup(&self) -> Result<String, AppError> {
        self.inner.backup().await.map_err(map_storage_error)
    }

    pub async fn list_backups(&self) -> Result<Vec<BackupInfo>, AppError> {
        self.inner.list_backups().await.map_err(map_storage_error)
    }

    pub async fn download_backup(&self, key: &str) -> Result<Vec<u8>, AppError> {
        self.inner
            .download_backup(key)
            .await
            .map_err(map_storage_error)
    }
}
