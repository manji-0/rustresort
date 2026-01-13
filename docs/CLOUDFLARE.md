# RustResort Cloudflare R2 Configuration

## Overview

RustResort uses Cloudflare R2 for media storage.

| Component | Service | Purpose |
|-----------|---------|---------|
| Database | **SQLite** | Main DB (local) |
| Media Storage | **Cloudflare R2** | Image/video storage |
| DB Backup | **Cloudflare R2** (separate bucket) | SQLite automatic backup |
| CDN | **Cloudflare CDN** | Media delivery via Custom Domain |

## Architecture

```
                                    ┌─────────────────────┐
                                    │   Cloudflare CDN    │
                                    │  (Custom Domain)    │
                                    │  media.example.com  │
                                    └──────────┬──────────┘
                                               │
┌─────────────────────────────────────────────────────────────────┐
│                         Cloudflare                               │
│  ┌─────────────────────────┐  ┌─────────────────────────┐       │
│  │          R2             │  │          R2             │       │
│  │     Media Bucket        │  │     Backup Bucket       │       │
│  │                         │  │                         │       │
│  │  media.example.com      │  │      (Private)          │       │
│  │       (Public)          │  │                         │       │
│  └────────────┬────────────┘  └────────────┬────────────┘       │
└───────────────┼─────────────────────────────┼───────────────────┘
                │                             │
                └──────────────┬──────────────┘
                               │
                 ┌─────────────┴─────────────┐
                 │       RustResort          │
                 │     (Self-hosted)         │
                 │                           │
                 │  ┌───────────────────┐   │
                 │  │      SQLite       │   │
                 │  │   rustresort.db   │   │
                 │  └───────────────────┘   │
                 └───────────────────────────┘
```

## R2 Bucket Configuration

### Bucket Separation by Purpose

| Bucket Name | Purpose | Public Access |
|------------|---------|---------------|
| `rustresort-media` | Media files | Public via Custom Domain |
| `rustresort-backup` | DB backups | Fully private |

## R2 Setup

### 1. Create R2 Buckets

Cloudflare Dashboard → R2 → Create bucket:

```
rustresort-media    ← For media
rustresort-backup   ← For backups
```

### 2. Configure Custom Domain (Media Bucket)

1. R2 → rustresort-media → Settings
2. Public access → Custom Domains → Connect Domain
3. Enter `media.example.com`
4. DNS records are automatically created

### 3. Create R2 API Token

1. R2 → Manage R2 API Tokens → Create API token
2. Permissions: Object Read & Write
3. Specify buckets: rustresort-media, rustresort-backup
4. Save Access Key ID and Secret Access Key

## Configuration File

```toml
[server]
host = "0.0.0.0"
port = 8080
domain = "social.example.com"
protocol = "https"

# Database (SQLite only)
[database]
path = "./data/rustresort.db"

# Media storage (R2)
[storage.media]
bucket = "rustresort-media"
public_url = "https://media.example.com"

# DB backup (R2 separate bucket)
[storage.backup]
enabled = true
bucket = "rustresort-backup"
interval_seconds = 86400  # 24 hours
retention_count = 7       # 7 generations

# Cloudflare R2 authentication
[cloudflare]
account_id = "${CLOUDFLARE_ACCOUNT_ID}"
r2_access_key_id = "${R2_ACCESS_KEY_ID}"
r2_secret_access_key = "${R2_SECRET_ACCESS_KEY}"

[instance]
title = "My RustResort"
description = "Personal Fediverse instance"
contact_email = "admin@example.com"

[logging]
level = "info"
format = "json"
```

## Environment Variables

```bash
# Cloudflare account
export CLOUDFLARE_ACCOUNT_ID="your-account-id"

# R2 access keys
export R2_ACCESS_KEY_ID="your-r2-access-key"
export R2_SECRET_ACCESS_KEY="your-r2-secret-key"
```

## Media Delivery Flow

```
┌──────────┐     ┌──────────────┐     ┌─────────────┐
│  Client  │────▶│  RustResort  │────▶│  R2 Bucket  │
│          │     │              │     │             │
│          │     │  1. Validate │     │  (upload)   │
│          │     │  2. Process  │     └──────┬──────┘
│          │     │  3. Upload   │            │
│          │     └──────────────┘            │
│          │                                 │
│          │     ┌──────────────┐            │
│          │◀────│  media.      │◀───────────┘
│          │     │  example.com │     Custom Domain
│          │     │  (CDN)       │     public access
└──────────┘     └──────────────┘
```

**Key Points:**
- Upload: RustResort → R2 (via API)
- Delivery: Client ← CDN ← R2 (bypasses RustResort)

## Implementation Example

### Media Storage

```rust
use aws_sdk_s3::Client as S3Client;

pub struct MediaStorage {
    client: S3Client,
    media_bucket: String,
    public_url: String,  // https://media.example.com
}

impl MediaStorage {
    /// Upload media to R2
    pub async fn upload(
        &self,
        key: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> Result<String, Error> {
        self.client
            .put_object()
            .bucket(&self.media_bucket)
            .key(key)
            .body(ByteStream::from(data))
            .content_type(content_type)
            .cache_control("public, max-age=31536000")  // 1 year cache
            .send()
            .await?;
        
        // Return public URL via Custom Domain
        Ok(format!("{}/{}", self.public_url, key))
    }
    
    /// Get public URL for media
    pub fn get_public_url(&self, key: &str) -> String {
        format!("{}/{}", self.public_url, key)
    }
}
```

### DB Backup

```rust
pub struct BackupService {
    client: S3Client,
    backup_bucket: String,
    db_path: PathBuf,
}

impl BackupService {
    pub async fn backup(&self) -> Result<String, Error> {
        // 1. SQLite online backup
        let backup_data = self.create_sqlite_backup().await?;
        
        // 2. Upload to R2
        let key = format!(
            "backups/rustresort_{}.db",
            Utc::now().format("%Y%m%d_%H%M%S")
        );
        
        self.client
            .put_object()
            .bucket(&self.backup_bucket)
            .key(&key)
            .body(ByteStream::from(backup_data))
            .content_type("application/x-sqlite3")
            .send()
            .await?;
        
        Ok(key)
    }
}
```

## Pricing Estimate (as of 2024)

| Service | Free Tier | Overage Rate |
|---------|-----------|--------------|
| R2 Storage | 10GB/month | $0.015/GB |
| R2 Class A Ops (Write) | 1M/month | $4.50/1M |
| R2 Class B Ops (Read) | 10M/month | $0.36/1M |
| Egress | **Free** | $0 |

**For personal instances**: Usually stays within free tier.

## Security

### Access Control

- **Media bucket**: Public via Custom Domain (read-only)
- **Backup bucket**: Fully private (API access only)

### Credential Protection

```bash
# Always use environment variables in production
# Never hardcode in configuration files

chmod 600 .env
```

## Troubleshooting

### Upload Failures

1. Check R2 API token permissions
2. Verify bucket name is correct
3. Confirm `aws-sdk-s3` endpoint is correct:
   ```
   https://<account_id>.r2.cloudflarestorage.com
   ```

### Custom Domain Not Accessible

1. Verify DNS record is proxied (orange cloud)
2. Check SSL/TLS mode
3. Purge cache

## Next Steps

- [STORAGE_STRATEGY.md](./STORAGE_STRATEGY.md) - Data persistence strategy
- [BACKUP.md](./BACKUP.md) - Backup details
- [DEVELOPMENT.md](./DEVELOPMENT.md) - Development guide
