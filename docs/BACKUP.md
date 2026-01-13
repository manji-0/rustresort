# RustResort Backup Design

## Overview

RustResort uses a single SQLite database file. Backups are implemented through periodic uploads to **Cloudflare R2**.

**Important:** Database backups are stored in a separate R2 bucket from media files.

## Design Philosophy

### Database Support

| Item | Choice | Reason |
|------|--------|--------|
| Database | **SQLite** | Optimal for single-user personal instances |
| PostgreSQL | ✗ Not supported | Eliminates excessive infrastructure requirements |

### Backup Strategy

```
┌─────────────────────────────────────────────────────────────┐
│                      RustResort                              │
│                                                              │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐  │
│  │   SQLite     │───▶│   Backup     │───▶│   R2 Backup  │  │
│  │  Database    │    │   Scheduler  │    │   Bucket     │  │
│  │              │    │              │    │  (Private)    │  │
│  │ rustresort.db│    │ - Daily      │    │              │  │
│  └──────────────┘    └──────────────┘    └──────────────┘  │
│                                                              │
│  ┌──────────────┐                        ┌──────────────┐  │
│  │    Media     │───────────────────────▶│   R2 Media   │  │
│  │   Upload     │                        │   Bucket     │  │
│  └──────────────┘                        │  (Public)     │  │
│                                          └──────────────┘  │
└─────────────────────────────────────────────────────────────┘

Bucket Separation:
- rustresort-media: Public via Custom Domain (media.example.com)
- rustresort-backup: Fully private (API access only)
```

## Backup Implementation

### Configuration

```toml
# Database backup
[storage.backup]
enabled = true
bucket = "rustresort-backup"  # Separate from media bucket
interval_seconds = 86400      # Every 24 hours
retention_count = 7           # Keep 7 generations

# Cloudflare R2 authentication
[cloudflare]
account_id = "${CLOUDFLARE_ACCOUNT_ID}"
r2_access_key_id = "${R2_ACCESS_KEY_ID}"
r2_secret_access_key = "${R2_SECRET_ACCESS_KEY}"

# Optional: Backup file encryption
[storage.backup.encryption]
enabled = true
key = "${BACKUP_ENCRYPTION_KEY}"  # 32 bytes, Base64 encoded
```

### Backup Scheduler

```rust
use aws_sdk_s3::Client as S3Client;
use tokio::time::{interval, Duration};
use std::path::Path;

/// Backup scheduler
pub struct BackupScheduler {
    config: BackupConfig,
    s3_client: S3Client,
    db_path: PathBuf,
}

impl BackupScheduler {
    pub fn new(config: BackupConfig, db_path: PathBuf) -> Result<Self, Error> {
        let s3_config = aws_config::from_env()
            .endpoint_url(&config.s3.endpoint)
            .region(Region::new(config.s3.region.clone()))
            .credentials_provider(Credentials::new(
                &config.s3.access_key,
                &config.s3.secret_key,
                None, None, "rustresort"
            ))
            .load()
            .await?;
        
        let s3_client = S3Client::new(&s3_config);
        
        Ok(Self {
            config,
            s3_client,
            db_path,
        })
    }
    
    /// Start backup loop
    pub async fn run(&self) {
        if !self.config.enabled {
            tracing::info!("Backup is disabled");
            return;
        }
        
        let mut interval = interval(Duration::from_secs(self.config.interval_seconds));
        
        // Run once on startup
        self.perform_backup().await;
        
        loop {
            interval.tick().await;
            self.perform_backup().await;
        }
    }
    
    /// Perform backup
    async fn perform_backup(&self) {
        tracing::info!("Starting scheduled backup");
        
        match self.backup_database().await {
            Ok(key) => {
                tracing::info!(%key, "Backup completed successfully");
                
                // Clean up old backups
                if self.config.retention_count > 0 {
                    if let Err(e) = self.cleanup_old_backups().await {
                        tracing::warn!(error = %e, "Failed to cleanup old backups");
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Backup failed");
            }
        }
    }
    
    /// Backup database to S3
    async fn backup_database(&self) -> Result<String, Error> {
        // 1. Use SQLite backup API for safe copy
        let backup_file = self.create_safe_backup().await?;
        
        // 2. Optional: Encryption
        let upload_data = if self.config.encryption.enabled {
            self.encrypt_file(&backup_file).await?
        } else {
            tokio::fs::read(&backup_file).await?
        };
        
        // 3. Upload to S3
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let key = format!("backups/rustresort_{}.db", timestamp);
        
        self.s3_client
            .put_object()
            .bucket(&self.config.s3.bucket)
            .key(&key)
            .body(ByteStream::from(upload_data))
            .content_type("application/x-sqlite3")
            .send()
            .await?;
        
        // 4. Delete temporary file
        tokio::fs::remove_file(&backup_file).await?;
        
        Ok(key)
    }
    
    /// Use SQLite online backup API
    async fn create_safe_backup(&self) -> Result<PathBuf, Error> {
        let backup_path = self.db_path.with_extension("db.backup");
        
        // Use SQLite online backup (safe during writes)
        let db_path = self.db_path.clone();
        let backup_path_clone = backup_path.clone();
        
        tokio::task::spawn_blocking(move || {
            use rusqlite::Connection;
            
            let src = Connection::open(&db_path)?;
            let mut dst = Connection::open(&backup_path_clone)?;
            
            let backup = rusqlite::backup::Backup::new(&src, &mut dst)?;
            backup.run_to_completion(100, Duration::from_millis(250), None)?;
            
            Ok::<_, Error>(())
        })
        .await??;
        
        Ok(backup_path)
    }
    
    /// Encrypt file
    async fn encrypt_file(&self, path: &Path) -> Result<Vec<u8>, Error> {
        use aes_gcm::{Aes256Gcm, Key, Nonce};
        use aes_gcm::aead::{Aead, NewAead};
        use rand::Rng;
        
        let data = tokio::fs::read(path).await?;
        let key = Key::from_slice(&self.config.encryption.key);
        let cipher = Aes256Gcm::new(key);
        
        // Generate random nonce
        let nonce_bytes: [u8; 12] = rand::thread_rng().gen();
        let nonce = Nonce::from_slice(&nonce_bytes);
        
        // Encrypt
        let ciphertext = cipher.encrypt(nonce, data.as_ref())
            .map_err(|e| Error::Encryption(e.to_string()))?;
        
        // Combine nonce + ciphertext
        let mut result = nonce_bytes.to_vec();
        result.extend(ciphertext);
        
        Ok(result)
    }
    
    /// Clean up old backups
    async fn cleanup_old_backups(&self) -> Result<(), Error> {
        // List backups
        let response = self.s3_client
            .list_objects_v2()
            .bucket(&self.config.s3.bucket)
            .prefix("backups/rustresort_")
            .send()
            .await?;
        
        let mut objects: Vec<_> = response.contents()
            .iter()
            .filter_map(|obj| obj.key().map(|k| k.to_string()))
            .collect();
        
        // Sort by oldest first
        objects.sort();
        
        // Delete excess backups
        let to_delete = objects.len().saturating_sub(self.config.retention_count);
        
        for key in objects.into_iter().take(to_delete) {
            tracing::info!(%key, "Deleting old backup");
            self.s3_client
                .delete_object()
                .bucket(&self.config.s3.bucket)
                .key(&key)
                .send()
                .await?;
        }
        
        Ok(())
    }
}
```

### Manual Backup API

```rust
/// Admin endpoints
impl AdminApi {
    /// POST /api/admin/backup
    /// Trigger manual backup
    pub async fn trigger_backup(
        State(state): State<AppState>,
    ) -> Result<Json<BackupResponse>, AppError> {
        let key = state.backup_scheduler.backup_database().await?;
        
        Ok(Json(BackupResponse {
            success: true,
            key,
            timestamp: Utc::now(),
        }))
    }
    
    /// GET /api/admin/backups
    /// List backups
    pub async fn list_backups(
        State(state): State<AppState>,
    ) -> Result<Json<Vec<BackupInfo>>, AppError> {
        let response = state.s3_client
            .list_objects_v2()
            .bucket(&state.config.backup.s3.bucket)
            .prefix("backups/rustresort_")
            .send()
            .await?;
        
        let backups: Vec<BackupInfo> = response.contents()
            .iter()
            .filter_map(|obj| {
                Some(BackupInfo {
                    key: obj.key()?.to_string(),
                    size: obj.size() as u64,
                    last_modified: obj.last_modified()?.to_string(),
                })
            })
            .collect();
        
        Ok(Json(backups))
    }
}

#[derive(Serialize)]
pub struct BackupResponse {
    pub success: bool,
    pub key: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct BackupInfo {
    pub key: String,
    pub size: u64,
    pub last_modified: String,
}
```

### Restore Procedure

Restore from backup is done manually:

```bash
# 1. Download backup from S3
aws s3 cp s3://my-rustresort-backup/backups/rustresort_20240101_120000.db ./restore.db

# 2. Decrypt if encrypted (decryption tool to be provided)
rustresort-cli decrypt --key $BACKUP_ENCRYPTION_KEY ./restore.db ./rustresort.db

# 3. Stop server
systemctl stop rustresort

# 4. Replace database
cp ./rustresort.db /var/lib/rustresort/data/rustresort.db

# 5. Start server
systemctl start rustresort
```

## Dependencies

```toml
[dependencies]
# S3 client
aws-sdk-s3 = "1.0"
aws-config = "1.0"

# SQLite online backup
rusqlite = { version = "0.31", features = ["bundled", "backup"] }

# Encryption
aes-gcm = "0.10"
rand = "0.8"
```

## Security Considerations

### Credential Management

```bash
# Manage via environment variables (recommended)
export BACKUP_S3_ACCESS_KEY="AKIAIOSFODNN7EXAMPLE"
export BACKUP_S3_SECRET_KEY="wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
export BACKUP_ENCRYPTION_KEY="base64encodedkey..."

# Or use AWS IAM roles (when using EC2/ECS)
# Credentials are automatically retrieved
```

### Encryption

- Backup files encrypted with AES-256-GCM
- Encryption key managed via environment variables
- S3 bucket server-side encryption also recommended

### Access Control

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "s3:PutObject",
        "s3:GetObject",
        "s3:DeleteObject",
        "s3:ListBucket"
      ],
      "Resource": [
        "arn:aws:s3:::my-rustresort-backup",
        "arn:aws:s3:::my-rustresort-backup/*"
      ]
    }
  ]
}
```

## Monitoring and Alerts

```rust
/// Backup status health check
impl HealthCheck {
    pub async fn check_backup_status(&self) -> BackupHealth {
        let last_backup = self.get_last_backup_time().await;
        
        let status = match last_backup {
            Some(time) if Utc::now() - time < Duration::hours(25) => {
                HealthStatus::Healthy
            }
            Some(time) if Utc::now() - time < Duration::hours(48) => {
                HealthStatus::Warning
            }
            _ => HealthStatus::Critical
        };
        
        BackupHealth {
            status,
            last_backup,
            next_scheduled: self.next_backup_time(),
        }
    }
}
```

## Next Steps

- [CLOUDFLARE.md](./CLOUDFLARE.md) - Cloudflare infrastructure design
- [STORAGE_STRATEGY.md](./STORAGE_STRATEGY.md) - Data persistence strategy
- [DEVELOPMENT.md](./DEVELOPMENT.md) - Development guide
