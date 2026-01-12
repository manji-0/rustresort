# RustResort API設計

## 概要

RustResortは2種類のAPIを提供します：

1. **Mastodon互換API** - 既存のMastodonクライアントとの互換性
2. **ActivityPub API** - Fediverse連携用

## ベースURL構成

```
https://example.com/
├── api/v1/          # Mastodon互換API
├── api/v2/          # Mastodon v2互換API
├── oauth/           # OAuth認証
├── users/{username} # ActivityPubアクター
├── statuses/{id}    # ActivityPubオブジェクト
├── .well-known/     # Well-known endpoints
└── nodeinfo/        # NodeInfo
```

## 認証

### OAuth 2.0フロー

```
1. アプリ登録: POST /api/v1/apps
2. 認可要求:   GET /oauth/authorize
3. トークン取得: POST /oauth/token
4. APIアクセス: Authorization: Bearer <token>
```

### スコープ

| スコープ | 説明 |
|---------|------|
| `read` | 読み取り全般 |
| `read:accounts` | アカウント情報の読み取り |
| `read:statuses` | 投稿の読み取り |
| `read:notifications` | 通知の読み取り |
| `write` | 書き込み全般 |
| `write:statuses` | 投稿の作成 |
| `write:media` | メディアアップロード |
| `write:favourites` | お気に入り操作 |
| `follow` | フォロー関係の管理 |
| `push` | WebPushの管理 |

## Mastodon互換API

### アカウント

#### GET /api/v1/accounts/:id
アカウント情報を取得。

**レスポンス例:**
```json
{
  "id": "01H8Y3VXPQM5JNABCDEFGHIJK",
  "username": "alice",
  "acct": "alice",
  "display_name": "Alice",
  "locked": false,
  "bot": false,
  "created_at": "2024-01-01T00:00:00.000Z",
  "note": "<p>Hello, world!</p>",
  "url": "https://example.com/@alice",
  "avatar": "https://example.com/media/avatars/alice.jpg",
  "avatar_static": "https://example.com/media/avatars/alice.jpg",
  "header": "https://example.com/media/headers/alice.jpg",
  "header_static": "https://example.com/media/headers/alice.jpg",
  "followers_count": 42,
  "following_count": 23,
  "statuses_count": 100,
  "fields": [
    {
      "name": "Website",
      "value": "<a href=\"https://alice.example.com\">alice.example.com</a>",
      "verified_at": "2024-01-01T00:00:00.000Z"
    }
  ],
  "emojis": []
}
```

#### GET /api/v1/accounts/verify_credentials
認証済みユーザー自身の情報を取得。

#### PATCH /api/v1/accounts/update_credentials
プロフィールを更新。

**パラメータ:**
- `display_name` - 表示名
- `note` - 自己紹介
- `avatar` - アバター画像
- `header` - ヘッダー画像
- `locked` - フォロー承認制
- `fields_attributes` - プロフィールフィールド

#### GET /api/v1/accounts/:id/statuses
アカウントの投稿一覧を取得。

#### GET /api/v1/accounts/:id/followers
フォロワー一覧を取得。

#### GET /api/v1/accounts/:id/following
フォロー中一覧を取得。

#### POST /api/v1/accounts/:id/follow
アカウントをフォロー。

#### POST /api/v1/accounts/:id/unfollow
フォロー解除。

#### POST /api/v1/accounts/:id/block
ブロック。

#### POST /api/v1/accounts/:id/unblock
ブロック解除。

#### POST /api/v1/accounts/:id/mute
ミュート。

#### POST /api/v1/accounts/:id/unmute
ミュート解除。

#### GET /api/v1/accounts/relationships
複数アカウントとの関係性を取得。

### ステータス（投稿）

#### POST /api/v1/statuses
新規投稿を作成。

**パラメータ:**
```json
{
  "status": "Hello, Fediverse!",
  "media_ids": ["01H8Y3VXPQM5JNABCDEFGHIJK"],
  "poll": {
    "options": ["Option A", "Option B"],
    "expires_in": 86400,
    "multiple": false
  },
  "in_reply_to_id": "01H8Y3VXPQM5JNABCDEFGHIJK",
  "sensitive": false,
  "spoiler_text": "",
  "visibility": "public",
  "language": "ja",
  "scheduled_at": null
}
```

**レスポンス例:**
```json
{
  "id": "01H8Y3VXPQM5JNABCDEFGHIJK",
  "uri": "https://example.com/statuses/01H8Y3VXPQM5JNABCDEFGHIJK",
  "url": "https://example.com/@alice/01H8Y3VXPQM5JNABCDEFGHIJK",
  "created_at": "2024-01-01T12:00:00.000Z",
  "account": { ... },
  "content": "<p>Hello, Fediverse!</p>",
  "visibility": "public",
  "sensitive": false,
  "spoiler_text": "",
  "media_attachments": [],
  "mentions": [],
  "tags": [],
  "emojis": [],
  "reblogs_count": 0,
  "favourites_count": 0,
  "replies_count": 0,
  "application": {
    "name": "Web",
    "website": null
  },
  "language": "ja",
  "favourited": false,
  "reblogged": false,
  "muted": false,
  "bookmarked": false,
  "pinned": false
}
```

#### GET /api/v1/statuses/:id
投稿を取得。

#### DELETE /api/v1/statuses/:id
投稿を削除。

#### PUT /api/v1/statuses/:id
投稿を編集。

#### GET /api/v1/statuses/:id/context
投稿のコンテキスト（リプライツリー）を取得。

#### POST /api/v1/statuses/:id/reblog
ブースト（リブログ）。

#### POST /api/v1/statuses/:id/unreblog
ブースト解除。

#### POST /api/v1/statuses/:id/favourite
お気に入り。

#### POST /api/v1/statuses/:id/unfavourite
お気に入り解除。

#### POST /api/v1/statuses/:id/bookmark
ブックマーク。

#### POST /api/v1/statuses/:id/unbookmark
ブックマーク解除。

#### POST /api/v1/statuses/:id/pin
プロフィールにピン留め。

#### POST /api/v1/statuses/:id/unpin
ピン解除。

### タイムライン

#### GET /api/v1/timelines/home
ホームタイムライン。

**パラメータ:**
- `max_id` - この投稿IDより古いものを取得
- `since_id` - この投稿IDより新しいものを取得
- `min_id` - この投稿ID以降を取得（逆順）
- `limit` - 取得件数（デフォルト20、最大40）

#### GET /api/v1/timelines/public
連合タイムライン。

**パラメータ:**
- `local` - ローカルのみ
- `remote` - リモートのみ
- `only_media` - メディア付きのみ

#### GET /api/v1/timelines/tag/:hashtag
ハッシュタグタイムライン。

### 通知

#### GET /api/v1/notifications
通知一覧を取得。

**パラメータ:**
- `types[]` - 取得する通知タイプ
- `exclude_types[]` - 除外する通知タイプ

#### GET /api/v1/notifications/:id
特定の通知を取得。

#### POST /api/v1/notifications/clear
全通知をクリア。

#### POST /api/v1/notifications/:id/dismiss
特定の通知を削除。

### メディア

#### POST /api/v1/media
メディアをアップロード。

**パラメータ:**
- `file` - ファイル（multipart/form-data）
- `description` - 代替テキスト
- `focus` - フォーカスポイント（x,y）

#### POST /api/v2/media
メディアを非同期でアップロード（処理中は202を返す）。

#### GET /api/v1/media/:id
メディア情報を取得。

#### PUT /api/v1/media/:id
メディア情報を更新。

### 検索

#### GET /api/v2/search
検索を実行。

**パラメータ:**
- `q` - 検索クエリ
- `type` - 検索タイプ（accounts, hashtags, statuses）
- `resolve` - WebFingerで解決を試みるか
- `limit` - 取得件数

### インスタンス情報

#### GET /api/v1/instance
インスタンス情報を取得。

```json
{
  "uri": "example.com",
  "title": "Example Instance",
  "short_description": "A friendly instance",
  "description": "Full description here...",
  "email": "admin@example.com",
  "version": "0.1.0",
  "urls": {
    "streaming_api": "wss://example.com"
  },
  "stats": {
    "user_count": 100,
    "status_count": 1000,
    "domain_count": 50
  },
  "thumbnail": "https://example.com/thumbnail.png",
  "languages": ["ja", "en"],
  "registrations": true,
  "approval_required": false,
  "invites_enabled": false,
  "configuration": {
    "statuses": {
      "max_characters": 5000,
      "max_media_attachments": 6
    },
    "media_attachments": {
      "supported_mime_types": ["image/jpeg", "image/png", "image/gif", "video/mp4"],
      "image_size_limit": 10485760,
      "video_size_limit": 41943040
    },
    "polls": {
      "max_options": 4,
      "max_characters_per_option": 50,
      "min_expiration": 300,
      "max_expiration": 2629746
    }
  },
  "contact_account": { ... },
  "rules": []
}
```

#### GET /api/v2/instance
v2形式のインスタンス情報。

### アプリ登録

#### POST /api/v1/apps
クライアントアプリを登録。

**パラメータ:**
```json
{
  "client_name": "My App",
  "redirect_uris": "https://myapp.example.com/callback",
  "scopes": "read write follow push",
  "website": "https://myapp.example.com"
}
```

## Well-known Endpoints

### GET /.well-known/webfinger
WebFinger。ユーザー発見用。

**パラメータ:**
- `resource` - `acct:username@domain`形式

**レスポンス例:**
```json
{
  "subject": "acct:alice@example.com",
  "aliases": [
    "https://example.com/@alice",
    "https://example.com/users/alice"
  ],
  "links": [
    {
      "rel": "http://webfinger.net/rel/profile-page",
      "type": "text/html",
      "href": "https://example.com/@alice"
    },
    {
      "rel": "self",
      "type": "application/activity+json",
      "href": "https://example.com/users/alice"
    }
  ]
}
```

### GET /.well-known/nodeinfo
NodeInfo発見。

**レスポンス:**
```json
{
  "links": [
    {
      "rel": "http://nodeinfo.diaspora.software/ns/schema/2.0",
      "href": "https://example.com/nodeinfo/2.0"
    }
  ]
}
```

### GET /.well-known/host-meta
host-meta（XML）。

## NodeInfo

### GET /nodeinfo/2.0
NodeInfo 2.0形式。

```json
{
  "version": "2.0",
  "software": {
    "name": "rustresort",
    "version": "0.1.0"
  },
  "protocols": ["activitypub"],
  "usage": {
    "users": {
      "total": 100,
      "activeMonth": 50,
      "activeHalfyear": 80
    },
    "localPosts": 1000
  },
  "openRegistrations": true
}
```

## ActivityPub API

### Actor

#### GET /users/{username}
アクターオブジェクトを取得。

**Accept:** `application/activity+json`, `application/ld+json`

```json
{
  "@context": [
    "https://www.w3.org/ns/activitystreams",
    "https://w3id.org/security/v1"
  ],
  "id": "https://example.com/users/alice",
  "type": "Person",
  "preferredUsername": "alice",
  "name": "Alice",
  "summary": "<p>Hello, world!</p>",
  "inbox": "https://example.com/users/alice/inbox",
  "outbox": "https://example.com/users/alice/outbox",
  "followers": "https://example.com/users/alice/followers",
  "following": "https://example.com/users/alice/following",
  "featured": "https://example.com/users/alice/collections/featured",
  "url": "https://example.com/@alice",
  "manuallyApprovesFollowers": false,
  "discoverable": true,
  "published": "2024-01-01T00:00:00Z",
  "icon": {
    "type": "Image",
    "mediaType": "image/jpeg",
    "url": "https://example.com/media/avatars/alice.jpg"
  },
  "image": {
    "type": "Image",
    "mediaType": "image/jpeg",
    "url": "https://example.com/media/headers/alice.jpg"
  },
  "publicKey": {
    "id": "https://example.com/users/alice#main-key",
    "owner": "https://example.com/users/alice",
    "publicKeyPem": "-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----"
  },
  "attachment": [
    {
      "type": "PropertyValue",
      "name": "Website",
      "value": "<a href=\"https://alice.example.com\">alice.example.com</a>"
    }
  ],
  "endpoints": {
    "sharedInbox": "https://example.com/inbox"
  }
}
```

### Inbox

#### POST /users/{username}/inbox
アクターのInboxにActivityを送信。

**必須:** HTTP Signature

**サポートするActivity:**
- `Create` - 投稿作成
- `Update` - 投稿・プロフィール更新
- `Delete` - 投稿削除
- `Follow` - フォロー
- `Accept` - フォロー承認
- `Reject` - フォロー拒否
- `Undo` - アクション取り消し
- `Announce` - ブースト
- `Like` - お気に入り
- `Block` - ブロック
- `Move` - アカウント移行

#### POST /inbox
共有Inbox。

### Outbox

#### GET /users/{username}/outbox
アクターのOutboxを取得（OrderedCollection）。

### Collections

#### GET /users/{username}/followers
フォロワーコレクション。

#### GET /users/{username}/following
フォロー中コレクション。

#### GET /users/{username}/collections/featured
ピン留め投稿コレクション。

### Object

#### GET /statuses/{id}
Noteオブジェクトを取得。

```json
{
  "@context": "https://www.w3.org/ns/activitystreams",
  "id": "https://example.com/statuses/01H8Y3VXPQM5JNABCDEFGHIJK",
  "type": "Note",
  "summary": null,
  "inReplyTo": null,
  "published": "2024-01-01T12:00:00Z",
  "url": "https://example.com/@alice/01H8Y3VXPQM5JNABCDEFGHIJK",
  "attributedTo": "https://example.com/users/alice",
  "to": ["https://www.w3.org/ns/activitystreams#Public"],
  "cc": ["https://example.com/users/alice/followers"],
  "sensitive": false,
  "content": "<p>Hello, Fediverse!</p>",
  "contentMap": {
    "ja": "<p>Hello, Fediverse!</p>"
  },
  "attachment": [],
  "tag": [],
  "replies": {
    "id": "https://example.com/statuses/01H8Y3VXPQM5JNABCDEFGHIJK/replies",
    "type": "Collection",
    "first": {
      "type": "CollectionPage",
      "items": []
    }
  }
}
```

## エラーレスポンス

全APIで統一されたエラー形式:

```json
{
  "error": "Record not found",
  "error_description": "The requested resource could not be found"
}
```

### HTTPステータスコード

| コード | 意味 |
|--------|------|
| 200 | 成功 |
| 201 | 作成成功 |
| 202 | 受理（非同期処理中） |
| 400 | 不正なリクエスト |
| 401 | 認証エラー |
| 403 | 権限エラー |
| 404 | リソースが見つからない |
| 410 | リソースが削除済み |
| 422 | バリデーションエラー |
| 429 | レート制限 |
| 500 | サーバーエラー |
| 503 | サービス利用不可 |

## レート制限

| エンドポイント | 制限 |
|---------------|------|
| 一般API | 300 req/5min |
| 認証エンドポイント | 30 req/5min |
| メディアアップロード | 30 req/30min |
| 投稿作成 | 30 req/30min |

レスポンスヘッダー:
- `X-RateLimit-Limit`: 制限値
- `X-RateLimit-Remaining`: 残り回数
- `X-RateLimit-Reset`: リセット時刻

## ページネーション

Link ヘッダーによるページネーション:

```
Link: <https://example.com/api/v1/timelines/home?max_id=123>; rel="next",
      <https://example.com/api/v1/timelines/home?min_id=456>; rel="prev"
```

## Streaming API（将来実装）

WebSocket接続:
- `wss://example.com/api/v1/streaming`

ストリーム:
- `user` - ユーザー通知
- `public` - 公開タイムライン
- `public:local` - ローカルタイムライン
- `hashtag` - ハッシュタグ
- `direct` - ダイレクトメッセージ

## 次のステップ

- [FEDERATION.md](./FEDERATION.md) - フェデレーション詳細仕様
- [DEVELOPMENT.md](./DEVELOPMENT.md) - 開発ガイド
