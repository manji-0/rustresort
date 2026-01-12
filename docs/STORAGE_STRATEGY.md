# RustResort データ永続化戦略

## 概要

RustResortは**シングルユーザーインスタンス**を前提とし、「自分視点の情報のみをDBに永続化する」という設計思想を採用しています。この戦略により、ストレージ使用量を最小限に抑えつつ、Fediverseとの完全な相互運用性を維持します。

## 設計思想

### コアコンセプト: 「自分視点」ストレージ

従来のActivityPubサーバーは、フェデレーション経由で受信した全てのデータをDBに保存します。これは複数ユーザーインスタンスでは必要ですが、シングルユーザーインスタンスでは過剰です。

RustResortでは以下の原則を採用します：

```
┌─────────────────────────────────────────────────────────────┐
│                    データ永続化の原則                        │
├─────────────────────────────────────────────────────────────┤
│  ✓ 自分が作成したコンテンツ → DB永続化                      │
│  ✓ 自分がアクションしたコンテンツ → DB永続化                │
│     (Repost, Fav, Bookmark)                                 │
│  ✓ フォロー関係のアドレス → DB永続化                        │
│  ✗ 他者のタイムラインtoot → メモリキャッシュのみ（揮発性）   │
│  ✗ 他者のプロフィール全文 → メモリキャッシュのみ（揮発性）   │
└─────────────────────────────────────────────────────────────┘
```

### シングルユーザー前提

- インスタンスには**adminユーザーのみ**が存在
- ユーザー登録・追加機能は実装しない
- 全ての操作は単一ユーザーの視点から行われる

## データ分類とストレージ戦略

### 1. 永続化データ（DB保存）

以下のデータはSQLiteに永続保存されます：

| データ種別 | 説明 | 理由 |
|-----------|------|------|
| 自分のStatus | 自分が作成した投稿 | コア資産 |
| メディアメタデータ | S3上のファイルへの参照情報 | コア資産（実ファイルはS3） |
| Repostした他者のStatus | ブースト対象 | 自分のアクション |
| Favした他者のStatus | お気に入り対象 | 自分のアクション |
| Bookmarkした他者のStatus | ブックマーク対象 | 自分のアクション |
| フォロー関係アドレス | `user@domain` 形式 | 関係性の維持 |
| **通知** | メンション、Like、ブースト等 | 履歴保持 |
| ドメインブロック | ブロックしたドメイン | モデレーション設定 |
| インスタンス設定 | 各種設定値 | 動作に必要 |

### 2. 揮発性データ（メモリキャッシュ）

以下のデータはメモリにのみ保持され、再起動で消失します：

| データ種別 | キャッシュサイズ | ライフサイクル |
|-----------|-----------------|---------------|
| タイムラインtoot | 最新2000件 | LRUで自動削除 |
| フォロイー/フォロワープロフィール | 全件 | 起動時取得、Federationで更新 |
| リモートアクターの公開鍵 | LRU 1000件 | 署名検証時に取得 |

### 3. オブジェクトストレージ（Cloudflare R2）

メディアファイルはCloudflare R2に保存され、Custom Domain経由で公開されます：

| データ種別 | 保存先 | 公開URL例 |
|-----------|--------|---------|
| アバター画像 | R2 | `https://media.example.com/avatars/{id}.webp` |
| ヘッダー画像 | R2 | `https://media.example.com/headers/{id}.webp` |
| 投稿添付メディア | R2 | `https://media.example.com/attachments/{id}.webp` |
| サムネイル | R2 | `https://media.example.com/thumbnails/{id}.webp` |

**メディア配信フロー:**
1. ユーザーがメディアをアップロード → RustResortがR2に保存
2. メディアURLは `https://media.example.com/...` (R2 Custom Domain)
3. クライアントはCDN経由でR2から直接取得（RustResortを経由しない）

詳細は [CLOUDFLARE.md](./CLOUDFLARE.md) を参照。

## アーキテクチャ

```
┌─────────────────────────────────────────────────────────────────┐
│                         Federation                               │
│                    (Incoming Activities)                         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Activity Router                             │
│         Create / Announce / Like / Follow / Update ...          │
└─────────────────────────────────────────────────────────────────┘
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
│  Persist Path   │ │   Cache Path    │ │  Ignore Path    │
│  (自分に関係)    │ │  (参照のみ)     │ │  (関係なし)      │
└────────┬────────┘ └────────┬────────┘ └─────────────────┘
         │                   │
         ▼                   ▼
┌─────────────────┐ ┌─────────────────┐
│     SQLite      │ │  Memory Cache   │
│   (Permanent)   │ │   (Volatile)    │
└─────────────────┘ └─────────────────┘
```

## 詳細設計

### タイムラインキャッシュ

```rust
use moka::future::Cache;
use std::sync::Arc;

/// タイムラインキャッシュ（最大2000件、LRU）
pub struct TimelineCache {
    /// Status ID -> CachedStatus
    statuses: Cache<String, Arc<CachedStatus>>,
    /// 最大保持件数
    max_items: usize,
}

/// キャッシュ用Status（軽量版）
#[derive(Debug, Clone)]
pub struct CachedStatus {
    pub id: String,
    pub uri: String,
    pub content: String,
    pub account_address: String,  // user@domain
    pub created_at: DateTime<Utc>,
    pub visibility: Visibility,
    pub attachments: Vec<CachedAttachment>,
    pub reply_to_uri: Option<String>,
    pub boost_of_uri: Option<String>,
    // Note: アカウント詳細は含まない（別キャッシュ参照）
}

impl TimelineCache {
    pub fn new(max_items: usize) -> Self {
        Self {
            statuses: Cache::builder()
                .max_capacity(max_items as u64)
                .time_to_live(Duration::from_secs(3600 * 24 * 7)) // 7日
                .build(),
            max_items,
        }
    }
    
    /// タイムラインに追加（自動でLRU削除）
    pub async fn insert(&self, status: CachedStatus) {
        self.statuses.insert(status.id.clone(), Arc::new(status)).await;
    }
    
    /// ホームタイムライン取得
    pub async fn get_home_timeline(
        &self,
        followee_addresses: &HashSet<String>,
        limit: usize,
        max_id: Option<&str>,
    ) -> Vec<Arc<CachedStatus>> {
        // フォロイーのStatusのみをフィルタして返す
        // ...
    }
}
```

### プロフィールキャッシュ

```rust
/// フォロイー/フォロワーのプロフィールキャッシュ
pub struct ProfileCache {
    /// user@domain -> CachedProfile
    profiles: Cache<String, Arc<CachedProfile>>,
}

#[derive(Debug, Clone)]
pub struct CachedProfile {
    pub address: String,           // user@domain
    pub uri: String,               // ActivityPub URI
    pub display_name: Option<String>,
    pub note: Option<String>,
    pub avatar_url: Option<String>,
    pub header_url: Option<String>,
    pub public_key_pem: String,
    pub inbox_uri: String,
    pub outbox_uri: Option<String>,
    pub followers_count: Option<u64>,
    pub following_count: Option<u64>,
    pub fetched_at: DateTime<Utc>,
}

impl ProfileCache {
    /// 起動時にDB保存のフォロー関係からプロフィールを一括取得
    pub async fn initialize_from_follows(
        &self,
        follow_addresses: Vec<String>,
        http_client: &HttpClient,
    ) {
        for address in follow_addresses {
            match self.fetch_profile(&address, http_client).await {
                Ok(profile) => {
                    self.profiles.insert(address, Arc::new(profile)).await;
                }
                Err(e) => {
                    tracing::warn!(%address, error = %e, "Failed to fetch profile at startup");
                }
            }
        }
    }
    
    /// Federation経由のUpdate Activityで更新
    pub async fn update_from_activity(&self, actor: &ActivityActor) {
        let address = format!("{}@{}", actor.preferred_username, actor.domain);
        if let Some(existing) = self.profiles.get(&address).await {
            let updated = CachedProfile {
                display_name: actor.name.clone(),
                note: actor.summary.clone(),
                avatar_url: actor.icon_url(),
                header_url: actor.image_url(),
                fetched_at: Utc::now(),
                ..(*existing).clone()
            };
            self.profiles.insert(address, Arc::new(updated)).await;
        }
    }
}
```

### 永続化判定ロジック

```rust
/// Activityの永続化判定
pub enum PersistenceDecision {
    /// DBに永続保存
    Persist,
    /// メモリキャッシュのみ
    CacheOnly,
    /// 保存しない
    Ignore,
}

impl ActivityProcessor {
    /// 受信したActivityの永続化判定
    pub fn decide_persistence(&self, activity: &Activity) -> PersistenceDecision {
        match &activity.activity_type {
            // 自分へのメンション → 通知をDB保存 + 元Statusはキャッシュ
            ActivityType::Create if self.mentions_me(&activity) => {
                // 通知はDBに永続化、Status自体はキャッシュ
                PersistenceDecision::Persist  // 通知部分
            }
            
            // フォロイーの投稿 → タイムラインキャッシュのみ
            ActivityType::Create if self.is_followee(&activity.actor) => {
                PersistenceDecision::CacheOnly
            }
            
            // 誰かが自分をフォロー → フォロワーアドレス + 通知をDB保存
            ActivityType::Follow if self.targets_me(&activity) => {
                PersistenceDecision::Persist
            }
            
            // 自分の投稿へのLike → 通知をDB保存
            ActivityType::Like if self.is_my_status(&activity.object) => {
                PersistenceDecision::Persist  // 通知として永続化
            }
            
            // 自分の投稿のブースト → 通知をDB保存
            ActivityType::Announce if self.is_my_status(&activity.object) => {
                PersistenceDecision::Persist  // 通知として永続化
            }
            
            // その他 → 無視
            _ => PersistenceDecision::Ignore,
        }
    }
}
```

### ユーザーアクションによる永続化

```rust
/// ユーザーアクションによる他者Statusの永続化
impl StatusService {
    /// Repost（ブースト）
    pub async fn repost(&self, status_uri: &str) -> Result<Status, Error> {
        // 1. キャッシュからStatusを取得
        let cached = self.timeline_cache.get_by_uri(status_uri).await
            .ok_or(Error::NotFound)?;
        
        // 2. 他者のStatusをDBに永続化（まだ保存されていない場合）
        let persisted = self.persist_remote_status(&cached).await?;
        
        // 3. Repost関係をDBに保存
        self.db.insert_repost(&self.my_account_id, &persisted.id).await?;
        
        // 4. Announce Activityを配信
        self.federation.send_announce(&persisted).await?;
        
        Ok(persisted)
    }
    
    /// お気に入り
    pub async fn favourite(&self, status_uri: &str) -> Result<(), Error> {
        let cached = self.timeline_cache.get_by_uri(status_uri).await
            .ok_or(Error::NotFound)?;
        
        // 他者のStatusをDBに永続化
        let persisted = self.persist_remote_status(&cached).await?;
        
        // Favourite関係をDBに保存
        self.db.insert_favourite(&self.my_account_id, &persisted.id).await?;
        
        // Like Activityを配信
        self.federation.send_like(&persisted).await?;
        
        Ok(())
    }
    
    /// ブックマーク（ローカルのみ、Federationなし）
    pub async fn bookmark(&self, status_uri: &str) -> Result<(), Error> {
        let cached = self.timeline_cache.get_by_uri(status_uri).await
            .ok_or(Error::NotFound)?;
        
        // 他者のStatusをDBに永続化
        let persisted = self.persist_remote_status(&cached).await?;
        
        // Bookmark関係をDBに保存
        self.db.insert_bookmark(&self.my_account_id, &persisted.id).await?;
        
        Ok(())
    }
    
    /// キャッシュStatusをDBに永続化
    async fn persist_remote_status(&self, cached: &CachedStatus) -> Result<Status, Error> {
        // 既にDBにある場合はそれを返す
        if let Some(existing) = self.db.get_status_by_uri(&cached.uri).await? {
            return Ok(existing);
        }
        
        // 新規保存
        let status = Status {
            id: EntityId::new(),
            uri: cached.uri.clone(),
            content: cached.content.clone(),
            account_address: cached.account_address.clone(),
            // ...
            persisted_reason: PersistedReason::UserAction,
        };
        
        self.db.insert_status(&status).await?;
        Ok(status)
    }
}

/// Status永続化の理由
#[derive(Debug, Clone, PartialEq)]
pub enum PersistedReason {
    /// 自分が作成
    OwnContent,
    /// Repost対象
    Reposted,
    /// お気に入り対象
    Favourited,
    /// ブックマーク対象
    Bookmarked,
    /// 自分の投稿へのリプライ（コンテキスト保持用）
    ReplyToOwn,
}
```

### フォロー関係のDB設計

```rust
/// フォロー関係（アドレスのみ保存）
#[derive(Debug, Clone)]
pub struct Follow {
    pub id: EntityId,
    pub created_at: DateTime<Utc>,
    /// フォロー先アドレス（user@domain）
    pub target_address: String,
    /// ActivityPub URI（Accept/Undo用）
    pub uri: String,
}

/// フォロワー（アドレスのみ保存）
#[derive(Debug, Clone)]
pub struct Follower {
    pub id: EntityId,
    pub created_at: DateTime<Utc>,
    /// フォロワーのアドレス（user@domain）
    pub follower_address: String,
    /// ActivityPub URI
    pub uri: String,
}
```

SQLマイグレーション:
```sql
-- フォロー関係（自分がフォローしている相手）
CREATE TABLE follows (
    id TEXT PRIMARY KEY,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    target_address TEXT NOT NULL UNIQUE,  -- user@domain
    uri TEXT NOT NULL UNIQUE
);

-- フォロワー（自分をフォローしている相手）
CREATE TABLE followers (
    id TEXT PRIMARY KEY,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    follower_address TEXT NOT NULL UNIQUE,  -- user@domain
    inbox_uri TEXT NOT NULL,  -- 配信先（プロフィールキャッシュにもあるが配信確実性のため保持）
    uri TEXT NOT NULL UNIQUE
);
```

### 起動時の初期化フロー

```rust
impl AppState {
    pub async fn initialize() -> Result<Self, Error> {
        // 1. DB接続
        let db = Database::connect(&config.database_url).await?;
        
        // 2. キャッシュ初期化
        let timeline_cache = TimelineCache::new(2000);
        let profile_cache = ProfileCache::new();
        
        // 3. フォロー関係をDBから読み込み
        let follow_addresses = db.get_all_follow_addresses().await?;
        let follower_addresses = db.get_all_follower_addresses().await?;
        
        // 4. フォロイー/フォロワーのプロフィールを並列取得
        let http_client = HttpClient::new();
        
        tokio::join!(
            profile_cache.initialize_from_addresses(&follow_addresses, &http_client),
            profile_cache.initialize_from_addresses(&follower_addresses, &http_client),
        );
        
        tracing::info!(
            follows = follow_addresses.len(),
            followers = follower_addresses.len(),
            "Initialized profile cache"
        );
        
        // 5. タイムラインは空の状態で開始
        // → Federationでリアルタイムに受信するか、
        //   フォロイーのOutboxから最新を取得するオプションあり
        
        Ok(Self {
            db: Arc::new(db),
            timeline_cache: Arc::new(timeline_cache),
            profile_cache: Arc::new(profile_cache),
            http_client: Arc::new(http_client),
            // ...
        })
    }
}
```

## DBスキーマ（最小構成）

```sql
-- 自分のアカウント情報（1レコードのみ）
CREATE TABLE account (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT,
    note TEXT,
    avatar_s3_key TEXT,     -- S3上のキー
    header_s3_key TEXT,     -- S3上のキー
    private_key_pem TEXT NOT NULL,
    public_key_pem TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 自分の投稿 + 永続化された他者の投稿
CREATE TABLE statuses (
    id TEXT PRIMARY KEY,
    uri TEXT NOT NULL UNIQUE,
    content TEXT NOT NULL,
    content_warning TEXT,
    visibility TEXT NOT NULL,
    language TEXT,
    account_address TEXT NOT NULL,  -- 自分の場合は空文字列
    is_local INTEGER NOT NULL DEFAULT 0,
    in_reply_to_uri TEXT,
    boost_of_uri TEXT,
    persisted_reason TEXT NOT NULL,  -- own/reposted/favourited/bookmarked/reply_to_own
    created_at TIMESTAMP NOT NULL,
    fetched_at TIMESTAMP
);

-- メディア添付（S3キーを保存、実ファイルはS3上）
CREATE TABLE media_attachments (
    id TEXT PRIMARY KEY,
    status_id TEXT,
    s3_key TEXT NOT NULL,           -- S3オブジェクトキー
    thumbnail_s3_key TEXT,          -- サムネイルのS3キー
    content_type TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    description TEXT,
    blurhash TEXT,
    width INTEGER,
    height INTEGER,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (status_id) REFERENCES statuses(id)
);

-- 通知（永続化）
CREATE TABLE notifications (
    id TEXT PRIMARY KEY,
    notification_type TEXT NOT NULL,  -- mention/favourite/reblog/follow/follow_request
    origin_account_address TEXT NOT NULL,  -- user@domain
    status_uri TEXT,                  -- 関連StatusのURI（あれば）
    read INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_notifications_created_at ON notifications(created_at DESC);
CREATE INDEX idx_notifications_read ON notifications(read);

-- フォロー関係
CREATE TABLE follows (
    id TEXT PRIMARY KEY,
    target_address TEXT NOT NULL UNIQUE,
    uri TEXT NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- フォロワー
CREATE TABLE followers (
    id TEXT PRIMARY KEY,
    follower_address TEXT NOT NULL UNIQUE,
    inbox_uri TEXT NOT NULL,
    uri TEXT NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- お気に入り
CREATE TABLE favourites (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (status_id) REFERENCES statuses(id),
    UNIQUE (status_id)
);

-- ブックマーク
CREATE TABLE bookmarks (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (status_id) REFERENCES statuses(id),
    UNIQUE (status_id)
);

-- Repost関係
CREATE TABLE reposts (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    uri TEXT NOT NULL UNIQUE,  -- Announce Activity URI
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (status_id) REFERENCES statuses(id),
    UNIQUE (status_id)
);

-- ドメインブロック
CREATE TABLE domain_blocks (
    id TEXT PRIMARY KEY,
    domain TEXT NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- インスタンス設定
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- インデックス
CREATE INDEX idx_statuses_created_at ON statuses(created_at DESC);
CREATE INDEX idx_statuses_account_address ON statuses(account_address);
CREATE INDEX idx_statuses_persisted_reason ON statuses(persisted_reason);
```

## 利点と制約

### 利点

| 利点 | 説明 |
|------|------|
| 💾 ストレージ節約 | DBサイズが劇的に小さくなる |
| ⚡ 高速起動 | DBからの読み込みが最小限 |
| 🔒 プライバシー | 他者のデータを保持しない |
| 🧹 管理不要 | 自動クリーンアップが不要 |
| 🎯 シンプル | シングルユーザー特化で複雑さ削減 |

### 制約

| 制約 | 説明 | 対処 |
|------|------|------|
| タイムラインの履歴 | 再起動で消失 | 重要なものはBookmark |
| 検索機能 | キャッシュ内のみ | 自分の投稿は全文検索可能 |
| オフライン時 | タイムライン空 | 起動時にOutbox取得オプション |
| S3必須 | メディア保存にS3が必要 | MinIO等のセルフホストも可 |

## 設定オプション

```toml
[cache]
# タイムラインキャッシュの最大件数
timeline_max_items = 2000

# プロフィールキャッシュTTL（秒）
profile_ttl = 86400  # 24時間

[storage]
# S3互換ストレージ（必須）
endpoint = "https://s3.amazonaws.com"
bucket = "my-rustresort-media"
region = "ap-northeast-1"
# access_key と secret_key は環境変数から

[startup]
# 起動時にフォロイーのOutboxから最新投稿を取得するか
fetch_followee_outbox = true

# Outboxから取得する最大件数
outbox_fetch_limit = 50
```

## 次のステップ

- [DATA_MODEL.md](./DATA_MODEL.md) - 詳細なデータモデル
- [FEDERATION.md](./FEDERATION.md) - フェデレーション処理
