# RustResort データモデル設計

## 概要

このドキュメントでは、RustResortで使用するデータモデルを定義します。
GoToSocialの`gtsmodel`パッケージを参考に、Rustの型システムを活用した設計を行います。

## 設計原則

1. **型安全性**: Rustの型システムを最大限活用
2. **不変条件の強制**: newtypeパターンによる制約
3. **NULL安全性**: `Option<T>`による明示的なnull許容
4. **シリアライゼーション**: serde対応
5. **SQLx互換**: SQLiteスキーマとのマッピング

## 型定義共通

### ID型（ULID）

```rust
use ulid::Ulid;

/// 全エンティティで使用する26文字のULID識別子
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(String);

impl EntityId {
    pub fn new() -> Self {
        Self(Ulid::new().to_string())
    }
    
    pub fn from_string(s: String) -> Result<Self, IdError> {
        if s.len() != 26 {
            return Err(IdError::InvalidLength);
        }
        Ok(Self(s))
    }
    
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
```

### 時刻型

```rust
use chrono::{DateTime, Utc};

/// タイムスタンプ型（UTC）
pub type Timestamp = DateTime<Utc>;
```

## コアモデル

### Account（アカウント）

ローカルおよびリモートのActivityPubアクターを表現。

```rust
/// ActivityPubアクター（ローカル/リモートアカウント）
#[derive(Debug, Clone)]
pub struct Account {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// 更新日時
    pub updated_at: Timestamp,
    
    /// 最後にフェッチした日時（リモートのみ）
    pub fetched_at: Option<Timestamp>,
    
    /// ユーザー名
    pub username: String,
    
    /// ドメイン（ローカルの場合はNone）
    pub domain: Option<String>,
    
    /// 表示名
    pub display_name: Option<String>,
    
    /// プロフィール説明（HTML）
    pub note: Option<String>,
    
    /// プロフィール説明（raw text、ローカルのみ）
    pub note_raw: Option<String>,
    
    /// アバター画像ID
    pub avatar_media_attachment_id: Option<EntityId>,
    
    /// アバターのリモートURL（リモートのみ）
    pub avatar_remote_url: Option<String>,
    
    /// ヘッダー画像ID
    pub header_media_attachment_id: Option<EntityId>,
    
    /// ヘッダーのリモートURL（リモートのみ）
    pub header_remote_url: Option<String>,
    
    /// フォローリクエストの手動承認が必要か
    pub locked: bool,
    
    /// ディスカバリー可能か
    pub discoverable: bool,
    
    /// 検索インデックス可能か
    pub indexable: bool,
    
    /// ActivityPub URI/ID
    pub uri: String,
    
    /// Web表示用URL
    pub url: Option<String>,
    
    /// Inbox URI
    pub inbox_uri: Option<String>,
    
    /// Shared Inbox URI
    pub shared_inbox_uri: Option<String>,
    
    /// Outbox URI
    pub outbox_uri: Option<String>,
    
    /// Following URI
    pub following_uri: Option<String>,
    
    /// Followers URI
    pub followers_uri: Option<String>,
    
    /// Featured (Pinned) Posts URI
    pub featured_collection_uri: Option<String>,
    
    /// アクタータイプ
    pub actor_type: ActorType,
    
    /// 公開鍵（PEM形式）
    pub public_key_pem: String,
    
    /// 公開鍵URI
    pub public_key_uri: String,
    
    /// 秘密鍵（PEM形式、ローカルのみ）
    pub private_key_pem: Option<String>,
    
    /// プロフィールフィールド
    pub fields: Vec<ProfileField>,
    
    /// 使用カスタム絵文字ID
    pub emoji_ids: Vec<EntityId>,
    
    /// Also Known As URIs（アカウント移行用）
    pub also_known_as_uris: Vec<String>,
    
    /// 移行先URI
    pub moved_to_uri: Option<String>,
    
    /// 凍結日時
    pub suspended_at: Option<Timestamp>,
    
    /// サイレンス日時
    pub silenced_at: Option<Timestamp>,
}

impl Account {
    /// ローカルアカウントかどうか
    pub fn is_local(&self) -> bool {
        self.domain.is_none()
    }
    
    /// リモートアカウントかどうか
    pub fn is_remote(&self) -> bool {
        self.domain.is_some()
    }
    
    /// @username@domain 形式の文字列を返す
    pub fn acct(&self) -> String {
        match &self.domain {
            Some(domain) => format!("@{}@{}", self.username, domain),
            None => format!("@{}", self.username),
        }
    }
}

/// ActivityPubアクタータイプ
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorType {
    Person,
    Application,
    Service,
    Group,
    Organization,
}

/// プロフィールフィールド
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileField {
    pub name: String,
    pub value: String,
    pub verified_at: Option<Timestamp>,
}
```

### User（ユーザー）

ローカルアカウントの認証情報。

```rust
/// ローカルユーザーの認証・設定情報
#[derive(Debug, Clone)]
pub struct User {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// 更新日時
    pub updated_at: Timestamp,
    
    /// 関連するアカウントID
    pub account_id: EntityId,
    
    /// メールアドレス
    pub email: String,
    
    /// メール確認済みか
    pub email_verified_at: Option<Timestamp>,
    
    /// パスワードハッシュ（bcrypt/argon2）
    pub encrypted_password: String,
    
    /// ロケール設定
    pub locale: String,
    
    /// 最終ログイン日時
    pub last_signed_in_at: Option<Timestamp>,
    
    /// 管理者フラグ
    pub admin: bool,
    
    /// モデレーターフラグ
    pub moderator: bool,
    
    /// 2FA有効フラグ
    pub two_factor_enabled: bool,
    
    /// 2FA秘密鍵
    pub two_factor_secret: Option<String>,
    
    /// 承認済みか（サインアップフロー用）
    pub approved: bool,
    
    /// 確認トークン
    pub confirmation_token: Option<String>,
    
    /// パスワードリセットトークン
    pub reset_password_token: Option<String>,
}
```

### Status（ステータス/投稿）

投稿を表現。

```rust
/// ステータス/投稿
#[derive(Debug, Clone)]
pub struct Status {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// 編集日時
    pub edited_at: Option<Timestamp>,
    
    /// フェッチ日時（リモートのみ）
    pub fetched_at: Option<Timestamp>,
    
    /// ピン留め日時
    pub pinned_at: Option<Timestamp>,
    
    /// ActivityPub URI
    pub uri: String,
    
    /// Web表示URL
    pub url: Option<String>,
    
    /// コンテンツHTML
    pub content: String,
    
    /// コンテンツraw text（ローカルのみ）
    pub text: Option<String>,
    
    /// コンテンツ警告（CW）
    pub content_warning: Option<String>,
    
    /// 公開範囲
    pub visibility: Visibility,
    
    /// センシティブフラグ
    pub sensitive: bool,
    
    /// 言語（BCP47）
    pub language: Option<String>,
    
    /// 投稿者アカウントID
    pub account_id: EntityId,
    
    /// リプライ先ステータスID
    pub in_reply_to_id: Option<EntityId>,
    
    /// リプライ先ステータスURI
    pub in_reply_to_uri: Option<String>,
    
    /// リプライ先アカウントID
    pub in_reply_to_account_id: Option<EntityId>,
    
    /// ブースト元ステータスID
    pub boost_of_id: Option<EntityId>,
    
    /// ブースト元アカウントID
    pub boost_of_account_id: Option<EntityId>,
    
    /// スレッドID
    pub thread_id: Option<EntityId>,
    
    /// ポールID
    pub poll_id: Option<EntityId>,
    
    /// 添付メディアID
    pub attachment_ids: Vec<EntityId>,
    
    /// タグID
    pub tag_ids: Vec<EntityId>,
    
    /// メンションID
    pub mention_ids: Vec<EntityId>,
    
    /// 絵文字ID
    pub emoji_ids: Vec<EntityId>,
    
    /// ローカル投稿フラグ
    pub local: bool,
    
    /// フェデレーション対象か
    pub federated: bool,
    
    /// ActivityStreamsタイプ（通常はNote）
    pub activity_streams_type: String,
    
    /// 作成アプリケーションID
    pub application_id: Option<EntityId>,
}

/// 投稿の公開範囲
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// 公開（誰でも見える、連合タイムラインに表示）
    Public,
    /// 未収載（誰でも見えるがタイムラインに非表示）
    Unlisted,
    /// フォロワーのみ
    FollowersOnly,
    /// ダイレクト（メンションされたユーザーのみ）
    Direct,
}

impl Visibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Unlisted => "unlisted",
            Visibility::FollowersOnly => "private",
            Visibility::Direct => "direct",
        }
    }
}
```

### MediaAttachment（メディア添付）

```rust
/// メディア添付ファイル
#[derive(Debug, Clone)]
pub struct MediaAttachment {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// 更新日時
    pub updated_at: Timestamp,
    
    /// 所有アカウントID
    pub account_id: EntityId,
    
    /// 添付先ステータスID
    pub status_id: Option<EntityId>,
    
    /// メディアタイプ
    pub media_type: MediaType,
    
    /// MIMEタイプ
    pub content_type: String,
    
    /// ファイルサイズ（バイト）
    pub file_size: i64,
    
    /// ローカルファイルパス
    pub file_path: Option<String>,
    
    /// リモートURL
    pub remote_url: Option<String>,
    
    /// サムネイルパス
    pub thumbnail_path: Option<String>,
    
    /// 代替テキスト
    pub description: Option<String>,
    
    /// ブラーハッシュ
    pub blurhash: Option<String>,
    
    /// メタ情報（幅、高さ、再生時間など）
    pub meta: Option<MediaMeta>,
    
    /// 処理状態
    pub processing_status: ProcessingStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaType {
    Image,
    Video,
    Audio,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaMeta {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<f64>,
    pub fps: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessingStatus {
    Pending,
    Processing,
    Complete,
    Failed,
}
```

### Follow（フォロー関係）

```rust
/// フォロー関係
#[derive(Debug, Clone)]
pub struct Follow {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// 更新日時
    pub updated_at: Timestamp,
    
    /// フォローするアカウントID
    pub account_id: EntityId,
    
    /// フォローされるアカウントID
    pub target_account_id: EntityId,
    
    /// ActivityPub URI
    pub uri: String,
    
    /// リブログを表示するか
    pub show_reblogs: bool,
    
    /// 通知を受け取るか
    pub notify: bool,
}

/// フォローリクエスト
#[derive(Debug, Clone)]
pub struct FollowRequest {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// 更新日時
    pub updated_at: Timestamp,
    
    /// リクエスト元アカウントID
    pub account_id: EntityId,
    
    /// リクエスト先アカウントID
    pub target_account_id: EntityId,
    
    /// ActivityPub URI
    pub uri: String,
    
    /// リブログを表示するか
    pub show_reblogs: bool,
    
    /// 通知を受け取るか
    pub notify: bool,
}
```

### Block（ブロック）

```rust
/// アカウントブロック
#[derive(Debug, Clone)]
pub struct Block {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// 更新日時
    pub updated_at: Timestamp,
    
    /// ブロックするアカウントID
    pub account_id: EntityId,
    
    /// ブロックされるアカウントID
    pub target_account_id: EntityId,
    
    /// ActivityPub URI
    pub uri: String,
}
```

### Notification（通知）

```rust
/// 通知
#[derive(Debug, Clone)]
pub struct Notification {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// 通知タイプ
    pub notification_type: NotificationType,
    
    /// 通知を受けるアカウントID
    pub target_account_id: EntityId,
    
    /// 通知元アカウントID
    pub origin_account_id: EntityId,
    
    /// 関連ステータスID
    pub status_id: Option<EntityId>,
    
    /// 既読フラグ
    pub read: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationType {
    Follow,
    FollowRequest,
    Mention,
    Reblog,
    Favourite,
    Poll,
    Status,
    Update,
    AdminSignUp,
    AdminReport,
}
```

### Emoji（カスタム絵文字）

```rust
/// カスタム絵文字
#[derive(Debug, Clone)]
pub struct Emoji {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// 更新日時
    pub updated_at: Timestamp,
    
    /// ショートコード（:emoji:）
    pub shortcode: String,
    
    /// ドメイン（ローカルの場合はNone）
    pub domain: Option<String>,
    
    /// 画像ファイルパス
    pub image_path: Option<String>,
    
    /// リモートURL
    pub image_remote_url: Option<String>,
    
    /// 静的画像パス
    pub image_static_path: Option<String>,
    
    /// コンテンツタイプ
    pub content_type: String,
    
    /// 有効フラグ
    pub disabled: bool,
    
    /// ActivityPub URI
    pub uri: String,
    
    /// カテゴリID
    pub category_id: Option<EntityId>,
}
```

### Instance（インスタンス/ドメイン情報）

```rust
/// リモートインスタンス情報
#[derive(Debug, Clone)]
pub struct Instance {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// 更新日時
    pub updated_at: Timestamp,
    
    /// ドメイン名
    pub domain: String,
    
    /// インスタンスタイトル
    pub title: Option<String>,
    
    /// 説明
    pub short_description: Option<String>,
    
    /// ソフトウェア名
    pub software: Option<String>,
    
    /// ソフトウェアバージョン
    pub software_version: Option<String>,
    
    /// 連絡先メールアドレス
    pub contact_email: Option<String>,
    
    /// 連絡先アカウントURI
    pub contact_account_uri: Option<String>,
}
```

### DomainBlock/DomainAllow（ドメインブロック・許可）

```rust
/// ドメインブロック
#[derive(Debug, Clone)]
pub struct DomainBlock {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// ドメイン
    pub domain: String,
    
    /// ブロック作成者ID
    pub created_by_account_id: EntityId,
    
    /// 公開コメント
    pub public_comment: Option<String>,
    
    /// プライベートコメント
    pub private_comment: Option<String>,
    
    /// 難読化フラグ
    pub obfuscate: bool,
}

/// ドメイン許可（Allowlistモード用）
#[derive(Debug, Clone)]
pub struct DomainAllow {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// ドメイン
    pub domain: String,
    
    /// 許可作成者ID
    pub created_by_account_id: EntityId,
}
```

## OAuth関連

```rust
/// OAuthアプリケーション
#[derive(Debug, Clone)]
pub struct Application {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// アプリ名
    pub name: String,
    
    /// リダイレクトURIs
    pub redirect_uris: Vec<String>,
    
    /// スコープ
    pub scopes: String,
    
    /// クライアントID
    pub client_id: String,
    
    /// クライアントシークレット
    pub client_secret: String,
    
    /// WebサイトURL
    pub website: Option<String>,
}

/// OAuthトークン
#[derive(Debug, Clone)]
pub struct Token {
    /// データベースID (ULID)
    pub id: EntityId,
    
    /// 作成日時
    pub created_at: Timestamp,
    
    /// アクセストークン
    pub access_token: String,
    
    /// リフレッシュトークン
    pub refresh_token: Option<String>,
    
    /// 有効期限
    pub expires_at: Option<Timestamp>,
    
    /// スコープ
    pub scopes: String,
    
    /// アプリケーションID
    pub application_id: EntityId,
    
    /// ユーザーID
    pub user_id: EntityId,
}
```

## ER図（概要）

```
┌──────────────┐     ┌──────────────┐
│    User      │────◇│   Account    │
└──────────────┘  1:1└──────┬───────┘
                           │
                     ┌─────┴─────┐
                     │           │
              ┌──────┴───┐ ┌─────┴──────┐
              │  Status  │ │   Follow   │
              └────┬─────┘ └────────────┘
                   │
     ┌─────────────┼─────────────┐
     │             │             │
┌────┴────┐  ┌─────┴─────┐ ┌─────┴─────┐
│  Media  │  │  Mention  │ │    Tag    │
└─────────┘  └───────────┘ └───────────┘
```

## マイグレーション戦略

1. **SQLiteのみサポート**（シングルユーザー個人インスタンス向け）
2. SQLXのマイグレーション機能 (`sqlx migrate`) を使用
3. 各テーブルの作成順序は依存関係を考慮
4. コンパイル時クエリ検証 (`sqlx::query!` マクロ) を活用
5. バックアップはS3互換ストレージへ自動アップロード（[BACKUP.md](./BACKUP.md)参照）

### SQLiteの利点（個人インスタンス向け）

- **単一ファイル**: `data/rustresort.db` のみ
- **ゼロ設定**: 外部DBサーバー不要
- **ポータブル**: ファイルコピーで完全移行
- **バックアップ容易**: ファイル単位でS3にアップロード
- **軽量**: メモリ使用量最小

### マイグレーションファイル

```
migrations/
├── 20240101000000_create_account.sql
├── 20240101000001_create_statuses.sql
├── 20240101000002_create_follows.sql
├── 20240101000003_create_followers.sql
└── ...
```

## インデックス設計

主要なインデックス:

- `accounts.username, accounts.domain` (UNIQUE)
- `accounts.uri` (UNIQUE)
- `statuses.uri` (UNIQUE)
- `statuses.account_id`
- `statuses.created_at DESC`
- `statuses.in_reply_to_id`
- `follows.account_id, follows.target_account_id` (UNIQUE)
- `notifications.target_account_id, notifications.created_at DESC`

## 次のステップ

- [API.md](./API.md) - API仕様の詳細
- [FEDERATION.md](./FEDERATION.md) - ActivityPubフェデレーション仕様
