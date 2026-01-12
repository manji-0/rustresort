# RustResort バックアップ設計

## 概要

RustResortはSQLite単一ファイルデータベースを採用しています。バックアップは**Cloudflare R2**への定期アップロードにより実現されます。

**重要:** DBバックアップはメディアとは別のR2バケットに保存されます。

## 設計方針

### データベースサポート

| 項目 | 選択 | 理由 |
|------|------|------|
| データベース | **SQLite** | シングルユーザー個人インスタンスに最適 |
| PostgreSQL | ✗ 非サポート | 過剰なインフラ要件を排除 |

### バックアップ戦略

```
┌─────────────────────────────────────────────────────────────┐
│                      RustResort                              │
│                                                              │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐  │
│  │   SQLite     │───▶│   Backup     │───▶│   R2 Backup  │  │
│  │  Database    │    │   Scheduler  │    │   Bucket     │  │
│  │              │    │              │    │  (非公開)     │  │
│  │ rustresort.db│    │ - 日次       │    │              │  │
│  └──────────────┘    └──────────────┘    └──────────────┘  │
│                                                              │
│  ┌──────────────┐                        ┌──────────────┐  │
│  │    Media     │───────────────────────▶│   R2 Media   │  │
│  │   Upload     │                        │   Bucket     │  │
│  └──────────────┘                        │  (公開)       │  │
│                                          └──────────────┘  │
└─────────────────────────────────────────────────────────────┘

バケット分離:
- rustresort-media: Custom Domain経由で公開（media.example.com）
- rustresort-backup: 完全非公開（APIアクセスのみ）
```

## バックアップ実装

### 設定

```toml
# DBバックアップ
[storage.backup]
enabled = true
bucket = "rustresort-backup"  # メディアとは別バケット
interval_seconds = 86400      # 24時間ごと
retention_count = 7           # 7世代保持

# Cloudflare R2認証
[cloudflare]
account_id = "${CLOUDFLARE_ACCOUNT_ID}"
r2_access_key_id = "${R2_ACCESS_KEY_ID}"
r2_secret_access_key = "${R2_SECRET_ACCESS_KEY}"

# オプション: バックアップファイルの暗号化
[storage.backup.encryption]
enabled = true
key = "${BACKUP_ENCRYPTION_KEY}"  # 32バイト、Base64エンコード
```

### バックアップスケジューラ

```rust
use aws_sdk_s3::Client as S3Client;
use tokio::time::{interval, Duration};
use std::path::Path;

/// バックアップスケジューラ
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
    
    /// バックアップループを開始
    pub async fn run(&self) {
        if !self.config.enabled {
            tracing::info!("Backup is disabled");
            return;
        }
        
        let mut interval = interval(Duration::from_secs(self.config.interval_seconds));
        
        // 起動時に一度実行
        self.perform_backup().await;
        
        loop {
            interval.tick().await;
            self.perform_backup().await;
        }
    }
    
    /// バックアップを実行
    async fn perform_backup(&self) {
        tracing::info!("Starting scheduled backup");
        
        match self.backup_database().await {
            Ok(key) => {
                tracing::info!(%key, "Backup completed successfully");
                
                // 古いバックアップを削除
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
    
    /// データベースをS3にバックアップ
    async fn backup_database(&self) -> Result<String, Error> {
        // 1. SQLiteのバックアップAPIを使用して安全にコピー
        let backup_file = self.create_safe_backup().await?;
        
        // 2. オプション: 暗号化
        let upload_data = if self.config.encryption.enabled {
            self.encrypt_file(&backup_file).await?
        } else {
            tokio::fs::read(&backup_file).await?
        };
        
        // 3. S3にアップロード
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
        
        // 4. 一時ファイル削除
        tokio::fs::remove_file(&backup_file).await?;
        
        Ok(key)
    }
    
    /// SQLiteのオンラインバックアップAPIを使用
    async fn create_safe_backup(&self) -> Result<PathBuf, Error> {
        let backup_path = self.db_path.with_extension("db.backup");
        
        // SQLiteのオンラインバックアップを使用（書き込み中でも安全）
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
    
    /// ファイルを暗号化
    async fn encrypt_file(&self, path: &Path) -> Result<Vec<u8>, Error> {
        use aes_gcm::{Aes256Gcm, Key, Nonce};
        use aes_gcm::aead::{Aead, NewAead};
        use rand::Rng;
        
        let data = tokio::fs::read(path).await?;
        let key = Key::from_slice(&self.config.encryption.key);
        let cipher = Aes256Gcm::new(key);
        
        // ランダムなnonceを生成
        let nonce_bytes: [u8; 12] = rand::thread_rng().gen();
        let nonce = Nonce::from_slice(&nonce_bytes);
        
        // 暗号化
        let ciphertext = cipher.encrypt(nonce, data.as_ref())
            .map_err(|e| Error::Encryption(e.to_string()))?;
        
        // nonce + ciphertext を結合
        let mut result = nonce_bytes.to_vec();
        result.extend(ciphertext);
        
        Ok(result)
    }
    
    /// 古いバックアップを削除
    async fn cleanup_old_backups(&self) -> Result<(), Error> {
        // バックアップ一覧を取得
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
        
        // 古い順にソート
        objects.sort();
        
        // 保持数を超えた分を削除
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

### 手動バックアップAPI

```rust
/// 管理者用エンドポイント
impl AdminApi {
    /// POST /api/admin/backup
    /// 手動でバックアップを実行
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
    /// バックアップ一覧を取得
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

### 復元手順

バックアップからの復元は手動で行います：

```bash
# 1. S3からバックアップをダウンロード
aws s3 cp s3://my-rustresort-backup/backups/rustresort_20240101_120000.db ./restore.db

# 2. 暗号化されている場合は復号（復号ツールを提供予定）
rustresort-cli decrypt --key $BACKUP_ENCRYPTION_KEY ./restore.db ./rustresort.db

# 3. サーバーを停止
systemctl stop rustresort

# 4. データベースを置き換え
cp ./rustresort.db /var/lib/rustresort/data/rustresort.db

# 5. サーバーを起動
systemctl start rustresort
```



## 依存クレート

```toml
[dependencies]
# S3クライアント
aws-sdk-s3 = "1.0"
aws-config = "1.0"

# SQLiteオンラインバックアップ
rusqlite = { version = "0.31", features = ["bundled", "backup"] }

# 暗号化
aes-gcm = "0.10"
rand = "0.8"
```

## セキュリティ考慮事項

### 認証情報の管理

```bash
# 環境変数で管理（推奨）
export BACKUP_S3_ACCESS_KEY="AKIAIOSFODNN7EXAMPLE"
export BACKUP_S3_SECRET_KEY="wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
export BACKUP_ENCRYPTION_KEY="base64encodedkey..."

# または AWS IAMロール（EC2/ECS使用時）
# 認証情報は自動的に取得される
```

### 暗号化

- バックアップファイルはAES-256-GCMで暗号化
- 暗号化キーは環境変数で管理
- S3バケット自体のサーバーサイド暗号化も推奨

### アクセス制御

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


## 監視とアラート

```rust
/// バックアップ状態のヘルスチェック
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

## 次のステップ

- [CLOUDFLARE.md](./CLOUDFLARE.md) - Cloudflareインフラ設計
- [STORAGE_STRATEGY.md](./STORAGE_STRATEGY.md) - データ永続化戦略
- [DEVELOPMENT.md](./DEVELOPMENT.md) - 開発ガイド
