# RustResort Cloudflare R2 設定

## 概要

RustResortはCloudflare R2をメディアストレージとして使用します。

| コンポーネント | サービス | 用途 |
|---------------|---------|------|
| データベース | **SQLite** | メインDB（ローカル） |
| メディアストレージ | **Cloudflare R2** | 画像・動画保存 |
| DBバックアップ | **Cloudflare R2** (別バケット) | SQLite自動バックアップ |
| CDN | **Cloudflare CDN** | Custom Domain経由でメディア配信 |

## アーキテクチャ

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
│  │  media.example.com 公開 │  │      (非公開)           │       │
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

## R2バケット構成

### 用途別バケット分離

| バケット名 | 用途 | 公開設定 |
|-----------|------|---------|
| `rustresort-media` | メディアファイル | Custom Domain公開 |
| `rustresort-backup` | DBバックアップ | 完全非公開 |

## R2設定

### 1. R2バケットの作成

Cloudflare Dashboard → R2 → Create bucket:

```
rustresort-media    ← メディア用
rustresort-backup   ← バックアップ用
```

### 2. Custom Domainの設定（メディアバケット）

1. R2 → rustresort-media → Settings
2. Public access → Custom Domains → Connect Domain
3. `media.example.com` を入力
4. DNSレコードが自動作成される

### 3. R2 APIトークンの作成

1. R2 → Manage R2 API Tokens → Create API token
2. 権限: Object Read & Write
3. バケット指定: rustresort-media, rustresort-backup
4. Access Key ID と Secret Access Key をメモ

## 設定ファイル

```toml
[server]
host = "0.0.0.0"
port = 8080
domain = "social.example.com"
protocol = "https"

# データベース（SQLiteのみ）
[database]
path = "./data/rustresort.db"

# メディアストレージ（R2）
[storage.media]
bucket = "rustresort-media"
public_url = "https://media.example.com"

# DBバックアップ（R2別バケット）
[storage.backup]
enabled = true
bucket = "rustresort-backup"
interval_seconds = 86400  # 24時間
retention_count = 7       # 7世代

# Cloudflare R2認証
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

## 環境変数

```bash
# Cloudflareアカウント
export CLOUDFLARE_ACCOUNT_ID="your-account-id"

# R2アクセスキー
export R2_ACCESS_KEY_ID="your-r2-access-key"
export R2_SECRET_ACCESS_KEY="your-r2-secret-key"
```

## メディア配信フロー

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
│          │     │  (CDN)       │     経由で公開
└──────────┘     └──────────────┘
```

**ポイント:**
- アップロード: RustResort → R2 (API経由)
- 配信: Client ← CDN ← R2 (RustResortを経由しない)

## 実装例

### メディアストレージ

```rust
use aws_sdk_s3::Client as S3Client;

pub struct MediaStorage {
    client: S3Client,
    media_bucket: String,
    public_url: String,  // https://media.example.com
}

impl MediaStorage {
    /// メディアをR2にアップロード
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
            .cache_control("public, max-age=31536000")  // 1年キャッシュ
            .send()
            .await?;
        
        // Custom Domain経由の公開URLを返す
        Ok(format!("{}/{}", self.public_url, key))
    }
    
    /// メディアの公開URLを取得
    pub fn get_public_url(&self, key: &str) -> String {
        format!("{}/{}", self.public_url, key)
    }
}
```

### DBバックアップ

```rust
pub struct BackupService {
    client: S3Client,
    backup_bucket: String,
    db_path: PathBuf,
}

impl BackupService {
    pub async fn backup(&self) -> Result<String, Error> {
        // 1. SQLiteオンラインバックアップ
        let backup_data = self.create_sqlite_backup().await?;
        
        // 2. R2にアップロード
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

## 料金目安（2024年時点）

| サービス | 無料枠 | 超過料金 |
|---------|-------|---------|
| R2 ストレージ | 10GB/月 | $0.015/GB |
| R2 Class A Ops (Write) | 1M/月 | $4.50/1M |
| R2 Class B Ops (Read) | 10M/月 | $0.36/1M |
| 転送量 | **無料** | $0 |

**個人インスタンスの場合**: 無料枠内で収まることがほとんど。

## セキュリティ

### アクセス制御

- **メディアバケット**: Custom Domain経由で公開（読み取りのみ）
- **バックアップバケット**: 完全プライベート（APIアクセスのみ）

### 認証情報の保護

```bash
# 本番環境では必ず環境変数を使用
# 設定ファイルに直接書かない

chmod 600 .env
```

## トラブルシューティング

### アップロードが失敗する

1. R2 APIトークンの権限を確認
2. バケット名が正しいか確認
3. `aws-sdk-s3`のエンドポイントが正しいか確認:
   ```
   https://<account_id>.r2.cloudflarestorage.com
   ```

### Custom Domainでアクセスできない

1. DNSレコードがプロキシ（オレンジ雲）になっているか確認
2. SSL/TLSモードを確認
3. キャッシュをパージ

## 次のステップ

- [STORAGE_STRATEGY.md](./STORAGE_STRATEGY.md) - データ永続化戦略
- [BACKUP.md](./BACKUP.md) - バックアップ詳細
- [DEVELOPMENT.md](./DEVELOPMENT.md) - 開発ガイド
