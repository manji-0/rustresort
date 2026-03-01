//! SQLite backup to Cloudflare R2
//!
//! Handles automatic and manual database backups.
//! Uses SQLite's online backup API for safe backups.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use aws_sdk_s3::Client as S3Client;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{DateTime, Utc};
use rand::RngCore;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::BackupStorageConfig;
use crate::error::AppError;
use crate::storage::build_r2_http_client;

const AES_256_KEY_BYTES: usize = 32;
const AES_GCM_NONCE_BYTES: usize = 12;
const ENCRYPTED_BACKUP_SUFFIX: &str = ".enc";

fn parse_backup_encryption_key(config: &BackupStorageConfig) -> Result<Option<Vec<u8>>, AppError> {
    if !config.enabled || !config.encryption.enabled {
        return Ok(None);
    }

    let raw_key = config
        .encryption
        .key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::Config(
                "storage.backup.encryption.key is required when backup encryption is enabled"
                    .to_string(),
            )
        })?;

    let key = BASE64_STANDARD.decode(raw_key).map_err(|_| {
        AppError::Config(
            "storage.backup.encryption.key must be valid base64-encoded bytes".to_string(),
        )
    })?;
    if key.len() != AES_256_KEY_BYTES {
        return Err(AppError::Config(format!(
            "storage.backup.encryption.key must decode to {} bytes",
            AES_256_KEY_BYTES
        )));
    }

    Ok(Some(key))
}

fn encrypt_backup_payload(key: &[u8], data: &[u8]) -> Result<Vec<u8>, AppError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| {
        AppError::Encryption(format!(
            "invalid backup encryption key length (expected {} bytes)",
            AES_256_KEY_BYTES
        ))
    })?;

    let mut nonce = [0_u8; AES_GCM_NONCE_BYTES];
    rand::thread_rng().fill_bytes(&mut nonce);
    let nonce_value = Nonce::from_slice(&nonce);
    let ciphertext = cipher
        .encrypt(nonce_value, data)
        .map_err(|_| AppError::Encryption("backup encryption failed".to_string()))?;

    let mut out = Vec::with_capacity(AES_GCM_NONCE_BYTES + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn decrypt_backup_payload(key: &[u8], data: &[u8]) -> Result<Vec<u8>, AppError> {
    if data.len() <= AES_GCM_NONCE_BYTES {
        return Err(AppError::Encryption(
            "encrypted backup payload is too short".to_string(),
        ));
    }

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| {
        AppError::Encryption(format!(
            "invalid backup encryption key length (expected {} bytes)",
            AES_256_KEY_BYTES
        ))
    })?;

    let (nonce, ciphertext) = data.split_at(AES_GCM_NONCE_BYTES);
    let nonce_value = Nonce::from_slice(nonce);
    cipher
        .decrypt(nonce_value, ciphertext)
        .map_err(|_| AppError::Encryption("backup decryption failed".to_string()))
}

async fn create_sqlite_backup_snapshot(db_path: &Path) -> Result<Vec<u8>, AppError> {
    use sqlx::Connection;

    let temp_dir = tempfile::tempdir()
        .map_err(|error| AppError::Storage(format!("Failed to create temp dir: {}", error)))?;
    let snapshot_path = temp_dir.path().join("backup_snapshot.db");
    let escaped_snapshot_path = snapshot_path.to_string_lossy().replace('\'', "''");
    let connection_string = format!("sqlite:{}?mode=rw", db_path.display());

    let mut connection = sqlx::SqliteConnection::connect(&connection_string)
        .await
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to open SQLite connection for backup: {}",
                error
            ))
        })?;
    sqlx::query(&format!("VACUUM INTO '{}'", escaped_snapshot_path))
        .execute(&mut connection)
        .await
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to create SQLite backup snapshot: {}",
                error
            ))
        })?;
    connection.close().await.map_err(|error| {
        AppError::Storage(format!(
            "Failed to close SQLite backup connection: {}",
            error
        ))
    })?;

    tokio::fs::read(&snapshot_path)
        .await
        .map_err(|error| AppError::Storage(format!("Failed to read backup snapshot: {}", error)))
}

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
        use aws_sdk_s3::config::BehaviorVersion;
        use aws_sdk_s3::config::{Credentials, Region};

        let endpoint = format!("https://{}.r2.cloudflarestorage.com", cloudflare.account_id);

        let credentials = Credentials::new(
            &cloudflare.r2_access_key_id,
            &cloudflare.r2_secret_access_key,
            None,
            None,
            "rustresort-r2-backup",
        );

        let http_client = build_r2_http_client();

        let s3_config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .http_client(http_client)
            .region(Region::new("auto"))
            .endpoint_url(&endpoint)
            .credentials_provider(credentials)
            .build();

        let client = S3Client::from_conf(s3_config);
        let encryption_key = parse_backup_encryption_key(config)?;

        Ok(Self {
            client,
            bucket: config.bucket.clone(),
            db_path,
            interval: Duration::from_secs(config.interval_seconds),
            retention_count: config.retention_count,
            encryption_key,
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
        let mut interval = tokio::time::interval(self.interval);

        loop {
            interval.tick().await;
            if let Err(error) = self.backup_now().await {
                tracing::error!(%error, "Scheduled backup failed");
            }
        }
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

        // 1. Create safe SQLite snapshot
        let data = self.create_sqlite_backup().await?;

        tracing::debug!(size = data.len(), "Database read successfully");

        // 2. Optionally encrypt
        let encrypt_backup = self.encryption_key.is_some();
        let backup_data = if let Some(key) = self.encryption_key.as_deref() {
            self.encrypt(key, &data)?
        } else {
            data
        };

        // 3. Upload to R2
        let key = self.upload_backup(backup_data, encrypt_backup).await?;

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
        create_sqlite_backup_snapshot(&self.db_path).await
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
    fn encrypt(&self, key: &[u8], data: &[u8]) -> Result<Vec<u8>, AppError> {
        encrypt_backup_payload(key, data)
    }

    /// Decrypt backup data
    ///
    /// # Arguments
    /// * `data` - nonce + ciphertext
    ///
    /// # Returns
    /// Decrypted data
    fn decrypt(&self, key: &[u8], data: &[u8]) -> Result<Vec<u8>, AppError> {
        decrypt_backup_payload(key, data)
    }

    /// Upload backup to R2
    ///
    /// # Arguments
    /// * `data` - Backup data (possibly encrypted)
    ///
    /// # Returns
    /// S3 key of the uploaded file
    async fn upload_backup(&self, data: Vec<u8>, encrypted: bool) -> Result<String, AppError> {
        use aws_sdk_s3::primitives::ByteStream;

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let suffix = if encrypted {
            format!("db{}", ENCRYPTED_BACKUP_SUFFIX)
        } else {
            "db".to_string()
        };
        let key = format!("backups/rustresort_{}.{}", timestamp, suffix);

        let mut request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(ByteStream::from(data));
        request = if encrypted {
            request
                .content_type("application/octet-stream")
                .metadata("encryption", "aes-256-gcm-v1")
        } else {
            request.content_type("application/x-sqlite3")
        };

        request
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

        if key.ends_with(ENCRYPTED_BACKUP_SUFFIX) {
            let encryption_key = self.encryption_key.as_deref().ok_or_else(|| {
                AppError::Encryption(
                    "backup is encrypted but no backup encryption key is configured".to_string(),
                )
            })?;
            self.decrypt(encryption_key, &bytes)
        } else {
            Ok(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AES_256_KEY_BYTES, create_sqlite_backup_snapshot, decrypt_backup_payload,
        encrypt_backup_payload, parse_backup_encryption_key,
    };
    use crate::config::{BackupEncryptionConfig, BackupStorageConfig};
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use sqlx::Connection;
    use tempfile::TempDir;

    fn backup_config(
        backup_enabled: bool,
        encryption_enabled: bool,
        key: Option<String>,
    ) -> BackupStorageConfig {
        BackupStorageConfig {
            enabled: backup_enabled,
            bucket: "test-backup".to_string(),
            interval_seconds: 86400,
            retention_count: 7,
            encryption: BackupEncryptionConfig {
                enabled: encryption_enabled,
                key,
            },
        }
    }

    #[test]
    fn parse_backup_encryption_key_accepts_valid_base64_32byte_key() {
        let key_bytes = vec![7_u8; AES_256_KEY_BYTES];
        let config = backup_config(true, true, Some(BASE64_STANDARD.encode(&key_bytes)));

        let parsed = parse_backup_encryption_key(&config).unwrap().unwrap();
        assert_eq!(parsed, key_bytes);
    }

    #[test]
    fn parse_backup_encryption_key_rejects_missing_key_when_enabled() {
        let config = backup_config(true, true, None);
        let error = parse_backup_encryption_key(&config).unwrap_err();
        assert!(matches!(error, crate::error::AppError::Config(_)));
    }

    #[test]
    fn parse_backup_encryption_key_rejects_non_base64() {
        let config = backup_config(true, true, Some("not-base64".to_string()));
        let error = parse_backup_encryption_key(&config).unwrap_err();
        assert!(matches!(error, crate::error::AppError::Config(_)));
    }

    #[test]
    fn parse_backup_encryption_key_rejects_wrong_length() {
        let short_key = BASE64_STANDARD.encode([1_u8; 16]);
        let config = backup_config(true, true, Some(short_key));
        let error = parse_backup_encryption_key(&config).unwrap_err();
        assert!(matches!(error, crate::error::AppError::Config(_)));
    }

    #[test]
    fn parse_backup_encryption_key_ignores_encryption_when_backup_is_disabled() {
        let config = backup_config(false, true, Some("not-base64".to_string()));
        let parsed = parse_backup_encryption_key(&config).unwrap();
        assert!(parsed.is_none());
    }

    #[test]
    fn encrypt_decrypt_roundtrip_succeeds() {
        let key = vec![9_u8; AES_256_KEY_BYTES];
        let payload = b"sqlite backup payload".to_vec();

        let encrypted = encrypt_backup_payload(&key, &payload).unwrap();
        let decrypted = decrypt_backup_payload(&key, &encrypted).unwrap();
        assert_eq!(decrypted, payload);
    }

    #[test]
    fn decrypt_rejects_short_payload() {
        let key = vec![9_u8; AES_256_KEY_BYTES];
        let error = decrypt_backup_payload(&key, &[0_u8; 8]).unwrap_err();
        assert!(matches!(error, crate::error::AppError::Encryption(_)));
    }

    #[tokio::test]
    async fn create_sqlite_backup_snapshot_creates_valid_copy() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("source.db");
        let connection_string = format!("sqlite:{}?mode=rwc", db_path.display());
        let mut connection = sqlx::SqliteConnection::connect(&connection_string)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE example (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO example (value) VALUES ('hello')")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();

        let backup = create_sqlite_backup_snapshot(&db_path).await.unwrap();
        assert!(backup.len() > 100);
        assert_eq!(&backup[..16], b"SQLite format 3\0");
    }
}
