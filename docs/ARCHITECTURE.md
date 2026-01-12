# RustResort - Rust製ActivityPub Twitterライクサービス アーキテクチャ設計

## 概要

RustResortは、GoToSocialを参考にしたRust製の軽量ActivityPubサーバーです。
Twitter/Mastodonライクなマイクロブログ機能を提供し、Fediverseとの相互運用性を重視した設計となっています。

## プロジェクト目標

1. **軽量性**: 低リソース環境（VPS、SBCなど）での動作
2. **安全性**: Rustの型システムによるメモリ安全性とセキュリティ
3. **相互運用性**: ActivityPub/ActivityStreams準拠によるFediverse連携
4. **シンプルさ**: 個人〜小規模インスタンス向けの管理しやすい設計
5. **パフォーマンス**: Rustの非同期処理による高スループット

## ハイレベルアーキテクチャ

```
┌─────────────────────────────────────────────────────────────────┐
│                         RustResort                               │
├─────────────────────────────────────────────────────────────────┤
│  ┌───────────────┐  ┌───────────────┐  ┌───────────────────┐   │
│  │   Web Client  │  │  Mastodon API │  │   ActivityPub     │   │
│  │   (Optional)  │  │   Compat      │  │   Federation      │   │
│  └───────┬───────┘  └───────┬───────┘  └─────────┬─────────┘   │
│          │                  │                    │              │
│          └──────────────────┼────────────────────┘              │
│                             │                                   │
│  ┌──────────────────────────┴──────────────────────────┐       │
│  │                  API Router (Axum)                   │       │
│  └──────────────────────────┬──────────────────────────┘       │
│                             │                                   │
│  ┌──────────────────────────┴──────────────────────────┐       │
│  │              Processing Layer (Service)              │       │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌───────────┐  │       │
│  │  │ Account │ │ Status  │ │ Media   │ │ Timeline  │  │       │
│  │  │ Service │ │ Service │ │ Service │ │ Service   │  │       │
│  │  └─────────┘ └─────────┘ └─────────┘ └───────────┘  │       │
│  └──────────────────────────┬──────────────────────────┘       │
│                             │                                   │
│  ┌──────────────────────────┴──────────────────────────┐       │
│  │              Federation Layer                        │       │
│  │  ┌────────────┐ ┌────────────┐ ┌──────────────────┐ │       │
│  │  │ Federator  │ │ HTTP Sigs  │ │ Activity Worker  │ │       │
│  │  └────────────┘ └────────────┘ └──────────────────┘ │       │
│  └──────────────────────────┬──────────────────────────┘       │
│                             │                                   │
│  ┌──────────────────────────┴──────────────────────────┐       │
│  │                   Data Layer                         │       │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌───────────┐  │       │
│  │  │   DB    │ │  Cache  │ │ Storage │ │   Queue   │  │       │
│  │  │ (SQLx)  │ │ (Moka)  │ │ (S3/FS) │ │ (Tokio)   │  │       │
│  │  └─────────┘ └─────────┘ └─────────┘ └───────────┘  │       │
│  └─────────────────────────────────────────────────────┘       │
└─────────────────────────────────────────────────────────────────┘
```

## 技術スタック

### コア

| カテゴリ | 技術 | 理由 |
|---------|------|------|
| 言語 | Rust 2024 Edition | メモリ安全性、パフォーマンス |
| 非同期ランタイム | Tokio | 業界標準、成熟度 |
| Webフレームワーク | Axum | Tower統合、型安全性 |
| データベース | **SQLite** | シングルユーザー特化、ゼロ設定 |
| SQLライブラリ | SQLx | コンパイル時クエリ検証、非同期ファースト |
| キャッシュ | Moka | 高性能インメモリキャッシュ |
| シリアライゼーション | serde | 業界標準 |

### ActivityPub関連

| カテゴリ | 技術 | 理由 |
|---------|------|------|
| HTTP Signatures | http-signature-normalization | ActivityPub必須 |
| JSON-LD | json-ld (crate) | ActivityStreams処理 |
| WebFinger | カスタム実装 | ユーザー発見 |

### インフラ（Cloudflare）

| カテゴリ | 技術 | 理由 |
|---------|------|------|
| 設定管理 | config-rs | 柔軟な設定読み込み |
| ログ | tracing | 構造化ログ |
| メディアストレージ | **Cloudflare R2** | Custom Domain経由で公開 |
| DBバックアップ | **Cloudflare R2** | 別バケットに保存 |
| TLS | rustls | メモリ安全なTLS |

詳細は [CLOUDFLARE.md](./CLOUDFLARE.md) を参照。

## モジュール構成

```
rustresort/
├── Cargo.toml
├── config/
│   └── default.toml          # デフォルト設定
├── docs/
│   ├── ARCHITECTURE.md       # このファイル
│   ├── DATA_MODEL.md         # データモデル設計
│   ├── API.md                # API仕様
│   ├── FEDERATION.md         # フェデレーション仕様
│   └── DEVELOPMENT.md        # 開発ガイド
├── migrations/               # DBマイグレーション
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── config/              # 設定管理
│   │   ├── mod.rs
│   │   └── settings.rs
│   ├── models/              # データモデル
│   │   ├── mod.rs
│   │   ├── account.rs
│   │   ├── status.rs
│   │   ├── media.rs
│   │   ├── notification.rs
│   │   ├── follow.rs
│   │   └── ...
│   ├── db/                  # データベース層
│   │   ├── mod.rs
│   │   ├── repository.rs    # Repositoryパターン
│   │   ├── account.rs
│   │   ├── status.rs
│   │   └── ...
│   ├── cache/               # キャッシュ層
│   │   ├── mod.rs
│   │   └── account.rs
│   ├── api/                 # API層
│   │   ├── mod.rs
│   │   ├── router.rs
│   │   ├── client/          # Mastodon互換API
│   │   │   ├── mod.rs
│   │   │   ├── accounts.rs
│   │   │   ├── statuses.rs
│   │   │   ├── timelines.rs
│   │   │   └── ...
│   │   ├── activitypub/     # ActivityPub API
│   │   │   ├── mod.rs
│   │   │   ├── inbox.rs
│   │   │   ├── outbox.rs
│   │   │   ├── actor.rs
│   │   │   └── ...
│   │   ├── wellknown/       # Well-known endpoints
│   │   │   ├── mod.rs
│   │   │   ├── webfinger.rs
│   │   │   ├── nodeinfo.rs
│   │   │   └── hostmeta.rs
│   │   ├── auth/            # 認証関連
│   │   │   ├── mod.rs
│   │   │   ├── oauth.rs
│   │   │   └── middleware.rs
│   │   └── model/           # APIレスポンスモデル
│   │       ├── mod.rs
│   │       └── ...
│   ├── service/             # ビジネスロジック層
│   │   ├── mod.rs
│   │   ├── account.rs
│   │   ├── status.rs
│   │   ├── timeline.rs
│   │   ├── media.rs
│   │   ├── notification.rs
│   │   └── ...
│   ├── federation/          # フェデレーション層
│   │   ├── mod.rs
│   │   ├── federator.rs     # フェデレーション管理
│   │   ├── dereferencing/   # リモートリソース取得
│   │   │   ├── mod.rs
│   │   │   ├── account.rs
│   │   │   └── status.rs
│   │   ├── delivery/        # アクティビティ配信
│   │   │   ├── mod.rs
│   │   │   └── worker.rs
│   │   └── protocol/        # ActivityPubプロトコル
│   │       ├── mod.rs
│   │       ├── activities.rs
│   │       ├── actors.rs
│   │       └── objects.rs
│   ├── transport/           # HTTPトランスポート
│   │   ├── mod.rs
│   │   ├── client.rs        # HTTPクライアント
│   │   └── signature.rs     # HTTP署名
│   ├── media/               # メディア処理
│   │   ├── mod.rs
│   │   ├── processor.rs
│   │   └── storage.rs
│   ├── queue/               # バックグラウンドジョブ
│   │   ├── mod.rs
│   │   └── worker.rs
│   ├── state/               # アプリケーション状態
│   │   └── mod.rs
│   └── util/                # ユーティリティ
│       ├── mod.rs
│       ├── id.rs            # ULID生成
│       └── time.rs
└── tests/
    ├── integration/
    └── fixtures/
```

## レイヤー責務

### 1. API Layer (`src/api/`)

- HTTPリクエストのルーティングと処理
- リクエストバリデーション
- 認証・認可チェック
- レスポンスシリアライゼーション

**サブモジュール:**
- `client/`: Mastodon API互換エンドポイント
- `activitypub/`: ActivityPubプロトコルエンドポイント
- `wellknown/`: `.well-known`エンドポイント
- `auth/`: OAuth2認証

### 2. Service Layer (`src/service/`)

- ビジネスロジックの実装
- トランザクション管理
- 複数リポジトリの調整
- イベント発行

### 3. Federation Layer (`src/federation/`)

- ActivityPubプロトコル処理
- リモートアクター/オブジェクトの取得（dereferencing）
- アクティビティの配信
- フェデレーションポリシー適用

### 4. Data Layer (`src/db/`, `src/cache/`)

- データの永続化
- キャッシュ管理
- クエリの最適化

### 5. Transport Layer (`src/transport/`)

- HTTP通信
- HTTP Signatures
- リトライ処理

## 依存性注入とステート管理

```rust
/// アプリケーション全体の共有状態
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Arc<DbPool>,
    pub cache: Arc<Cache>,
    pub storage: Arc<dyn MediaStorage>,
    pub http_client: Arc<HttpClient>,
    pub federator: Arc<Federator>,
    pub queue: Arc<Queue>,
}
```

Axumの`State`エクストラクターを使用して各ハンドラに注入。

## エラーハンドリング

```rust
/// アプリケーションエラー型
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),
    
    #[error("Unauthorized")]
    Unauthorized,
    
    #[error("Forbidden")]
    Forbidden,
    
    #[error("Bad request: {0}")]
    BadRequest(String),
    
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
    
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    
    #[error("Federation error: {0}")]
    Federation(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // 適切なHTTPステータスコードとJSON bodyに変換
    }
}
```

## 非同期処理モデル

GoToSocialのworkerパターンを参考に、Tokioベースのバックグラウンドジョブシステムを実装:

```rust
/// ワーカータスクの種類
pub enum WorkerTask {
    /// アクティビティを配信
    DeliverActivity {
        activity: Activity,
        inbox_urls: Vec<String>,
    },
    /// リモートアカウントを取得・更新
    FetchRemoteAccount {
        uri: String,
    },
    /// メディアを処理
    ProcessMedia {
        attachment_id: String,
    },
}
```

## セキュリティ考慮事項

1. **HTTP Signatures**: 全てのActivityPubリクエストに署名を要求
2. **Input Validation**: 全入力の厳格なバリデーション
3. **Rate Limiting**: Tower middlewareによるレート制限
4. **CORS**: 適切なCORS設定
5. **CSP**: コンテンツセキュリティポリシー

## パフォーマンス最適化

1. **コネクションプーリング**: DBコネクションプール
2. **キャッシング**: 頻繁にアクセスされるデータのメモリキャッシュ
3. **遅延読み込み**: 必要時のみ関連データを読み込み
4. **バッチ処理**: 配信の一括処理
5. **非同期I/O**: 全I/O操作の非同期化

## 次のステップ

1. [STORAGE_STRATEGY.md](./STORAGE_STRATEGY.md) - データ永続化戦略（重要）
2. [DATA_MODEL.md](./DATA_MODEL.md) - データモデルの詳細設計
3. [API.md](./API.md) - API仕様
4. [FEDERATION.md](./FEDERATION.md) - フェデレーション仕様
5. [DEVELOPMENT.md](./DEVELOPMENT.md) - 開発環境セットアップ
