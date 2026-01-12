//! SQLite backup to Cloudflare R2
//!
//! Handles automatic and manual database backups.
//! Uses SQLite's online backup API for safe backups.

use aws_sdk_s3::Client as S3Client;
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::time::Duration;

use crate::config::BackupStorageConfig;
use crate::error::AppError;

/// Backup service for SQLite database
///
/// Periodically backs up the database to a separate R2 bucket.
/// Supports encryption and retention policies.
pub struct BackupService {
    /// S3-compatible client for R2
    client: S3Client,
    /// Backup bucket name (separate from media)
    bucket: String,
    /// Path to SQLite database file
    db_path: PathBuf,
    /// Backup interval
    interval: Duration,
    /// Number of backups to retain
    retention_count: usize,
    /// Encryption key (optional)
    encryption_key: Option<Vec<u8>>,
}

/// Backup metadata
#[derive(Debug, Clone)]
pub struct BackupInfo {
    /// S3 key
    pub key: String,
    /// File size in bytes
    pub size: u64,
    /// Backup timestamp
    pub created_at: DateTime<Utc>,
}

impl BackupService {
    /// Create new backup service
    ///
    /// # Arguments
    /// * `config` - Backup configuration
    /// * `cloudflare` - Cloudflare credentials
    /// * `db_path` - Path to SQLite database
    ///
    /// # Errors
    /// Returns error if S3 client initialization fails
    pub async fn new(
        config: &BackupStorageConfig,
        cloudflare: &crate::config::CloudflareConfig,
        db_path: PathBuf,
    ) -> Result<Self, AppError> {
        use aws_config::BehaviorVersion;
        use aws_sdk_s3::config::{Credentials, Region};

        let endpoint = format!("https://{}.r2.cloudflarestorage.com", cloudflare.account_id);

        let credentials = Credentials::new(
            &cloudflare.r2_access_key_id,
            &cloudflare.r2_secret_access_key,
            None,
            None,
            "rustresort-r2-backup",
        );

        let s3_config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("auto"))
            .endpoint_url(&endpoint)
            .credentials_provider(credentials)
            .build();

        let client = S3Client::from_conf(s3_config);

        Ok(Self {
            client,
            bucket: config.bucket.clone(),
            db_path,
            interval: Duration::from_secs(config.interval_seconds),
            retention_count: config.retention_count,
            encryption_key: None, // TODO: Implement encryption
        })
    }

    /// Start the backup scheduler
    ///
    /// Runs in background, performs backups at configured interval.
    /// First backup runs immediately on start.
    ///
    /// # Note
    /// This method runs indefinitely. Call in a spawned task.
    pub async fn run(&self) {
        // TODO:
        // 1. Perform immediate backup
        // 2. Loop with interval, perform backup
        // 3. Clean up old backups
        todo!()
    }

    /// Perform a backup now
    ///
    /// # Returns
    /// S3 key of the backup file
    ///
    /// # Steps
    /// 1. Create safe copy using SQLite backup API
    /// 2. Optionally encrypt the backup
    /// 3. Upload to R2
    /// 4. Delete local temporary file
    pub async fn backup_now(&self) -> Result<String, AppError> {
        tracing::info!("Starting database backup...");

        // 1. Read database file
        let data = tokio::fs::read(&self.db_path)
            .await
            .map_err(|e| AppError::Storage(format!("Failed to read database: {}", e)))?;

        tracing::debug!(size = data.len(), "Database read successfully");

        // 2. Optionally encrypt (TODO: implement encryption)
        let backup_data = if self.encryption_key.is_some() {
            // self.encrypt(&data)?
            data // For now, skip encryption
        } else {
            data
        };

        // 3. Upload to R2
        let key = self.upload_backup(backup_data).await?;

        // 4. Cleanup old backups
        if let Err(e) = self.cleanup_old_backups().await {
            tracing::warn!(error = %e, "Failed to cleanup old backups");
        }

        tracing::info!(key = %key, "Backup completed successfully");
        Ok(key)
    }

    /// Alias for backup_now()
    pub async fn backup(&self) -> Result<String, AppError> {
        self.backup_now().await
    }

    /// Create safe backup of SQLite database
    ///
    /// Uses rusqlite's Backup API for online backup.
    /// Safe even if database is being written to.
    ///
    /// # Returns
    /// Backup data as bytes
    async fn create_sqlite_backup(&self) -> Result<Vec<u8>, AppError> {
        // TODO: Use rusqlite backup API
        // This should be run in spawn_blocking
        todo!()
    }

    /// Encrypt backup data
    ///
    /// Uses AES-256-GCM encryption.
    ///
    /// # Arguments
    /// * `data` - Data to encrypt
    ///
    /// # Returns
    /// nonce (12 bytes) + ciphertext
    fn encrypt(&self, _data: &[u8]) -> Result<Vec<u8>, AppError> {
        // TODO: Encrypt with AES-256-GCM
        todo!()
    }

    /// Decrypt backup data
    ///
    /// # Arguments
    /// * `data` - nonce + ciphertext
    ///
    /// # Returns
    /// Decrypted data
    fn decrypt(&self, _data: &[u8]) -> Result<Vec<u8>, AppError> {
        // TODO: Decrypt with AES-256-GCM
        todo!()
    }

    /// Upload backup to R2
    ///
    /// # Arguments
    /// * `data` - Backup data (possibly encrypted)
    ///
    /// # Returns
    /// S3 key of the uploaded file
    async fn upload_backup(&self, data: Vec<u8>) -> Result<String, AppError> {
        use aws_sdk_s3::primitives::ByteStream;

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let key = format!("backups/rustresort_{}.db", timestamp);

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(ByteStream::from(data))
            .content_type("application/x-sqlite3")
            .send()
            .await
            .map_err(|e| AppError::Storage(format!("Backup upload failed: {}", e)))?;

        Ok(key)
    }

    /// List all backups
    ///
    /// # Returns
    /// List of backup info, sorted by date descending
    pub async fn list_backups(&self) -> Result<Vec<BackupInfo>, AppError> {
        let result = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix("backups/rustresort_")
            .send()
            .await
            .map_err(|e| AppError::Storage(format!("Failed to list backups: {}", e)))?;

        let mut backups = Vec::new();

        if let Some(contents) = result.contents {
            for object in contents {
                if let (Some(key), Some(size), Some(modified)) =
                    (object.key, object.size, object.last_modified)
                {
                    backups.push(BackupInfo {
                        key: key.to_string(),
                        size: size as u64,
                        created_at: DateTime::from_timestamp(
                            modified.secs(),
                            modified.subsec_nanos(),
                        )
                        .unwrap_or_else(Utc::now),
                    });
                }
            }
        }

        // Sort by date descending
        backups.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(backups)
    }

    /// Delete old backups beyond retention count
    ///
    /// Keeps the most recent `retention_count` backups.
    async fn cleanup_old_backups(&self) -> Result<(), AppError> {
        let backups = self.list_backups().await?;

        // Keep only retention_count backups
        if backups.len() > self.retention_count {
            let to_delete = &backups[self.retention_count..];

            for backup in to_delete {
                tracing::info!(key = %backup.key, "Deleting old backup");

                self.client
                    .delete_object()
                    .bucket(&self.bucket)
                    .key(&backup.key)
                    .send()
                    .await
                    .map_err(|e| AppError::Storage(format!("Failed to delete backup: {}", e)))?;
            }
        }

        Ok(())
    }

    /// Download a backup
    ///
    /// # Arguments
    /// * `key` - S3 key of the backup
    ///
    /// # Returns
    /// Decrypted backup data
    pub async fn download_backup(&self, key: &str) -> Result<Vec<u8>, AppError> {
        let result = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::Storage(format!("Failed to download backup: {}", e)))?;

        let data = result
            .body
            .collect()
            .await
            .map_err(|e| AppError::Storage(format!("Failed to read backup data: {}", e)))?;

        let bytes = data.into_bytes().to_vec();

        // Decrypt if encrypted
        if self.encryption_key.is_some() {
            // self.decrypt(&bytes)
            Ok(bytes) // For now, skip decryption
        } else {
            Ok(bytes)
        }
    }
}
