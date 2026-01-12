# RustResort 開発ガイド

## 概要

このドキュメントでは、RustResortの開発環境セットアップと開発ワークフローについて説明します。

## 前提条件

### 必須ソフトウェア

- **Rust**: 1.82以上（2024 Edition対応）
- **SQLite**: 3.35以上
- **Git**: 2.30以上

### 推奨ツール

- **cargo-watch**: ファイル変更時の自動リビルド
- **cargo-audit**: セキュリティ監査
- **sqlx-cli**: マイグレーション管理・コンパイル時クエリ検証

## セットアップ

### 1. リポジトリクローン

```bash
git clone https://github.com/yourusername/rustresort.git
cd rustresort
```

### 2. Rustツールチェインのインストール

```bash
# Rust本体のインストール（未インストールの場合）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2024 Editionのサポート確認
rustup update stable
rustc --version  # 1.82.0以上を確認
```

### 3. 開発ツールのインストール

```bash
# cargo-watch（ホットリロード用）
cargo install cargo-watch

# cargo-audit（セキュリティ監査）
cargo install cargo-audit

# SQLx CLI（マイグレーション管理 + オフラインモード用）
cargo install sqlx-cli --no-default-features --features "sqlite,postgres,rustls"
```

### 4. 環境設定

設定ファイルを作成：

```bash
cp config/default.toml.example config/local.toml
```

`config/local.toml`を編集：

```toml
[server]
host = "127.0.0.1"
port = 8080
domain = "localhost:8080"
protocol = "http"

# データベース（SQLite）
[database]
path = "./data/rustresort.db"

# メディアストレージ（Cloudflare R2）
[storage.media]
bucket = "rustresort-media"
public_url = "http://localhost:9000/rustresort-media"  # 開発用
# 本番: public_url = "https://media.example.com"

# DBバックアップ（別のR2バケット、SQLite使用時のみ）
[storage.backup]
enabled = false
bucket = "rustresort-backup"
interval_seconds = 86400

# Cloudflare認証（環境変数推奨）
[cloudflare]
account_id = "${CLOUDFLARE_ACCOUNT_ID}"
r2_access_key_id = "${R2_ACCESS_KEY_ID}"
r2_secret_access_key = "${R2_SECRET_ACCESS_KEY}"

[instance]
title = "RustResort Dev"
description = "Development instance"
contact_email = "admin@localhost"

[logging]
level = "debug"
format = "pretty"  # pretty | json
```

### ローカル開発（MinIOでR2をエミュレート）

開発環境ではMinIOを使用してR2をエミュレートします：

```bash
# MinIOをDockerで起動
docker run -d \
  --name minio \
  -p 9000:9000 \
  -p 9001:9001 \
  -e MINIO_ROOT_USER=minioadmin \
  -e MINIO_ROOT_PASSWORD=minioadmin \
  minio/minio server /data --console-address ":9001"

# バケットを作成
mc alias set local http://localhost:9000 minioadmin minioadmin
mc mb local/rustresort-media
mc mb local/rustresort-backup

# 環境変数を設定（R2互換）
export CLOUDFLARE_ACCOUNT_ID=local
export R2_ACCESS_KEY_ID=minioadmin
export R2_SECRET_ACCESS_KEY=minioadmin
```

本番環境でのCloudflare設定は [CLOUDFLARE.md](./CLOUDFLARE.md) を参照。

### 5. データベースセットアップ

```bash
# データベースを作成（SQLiteの場合は自動作成）
sqlx database create

# マイグレーションの実行
sqlx migrate run

# オフラインモード用のクエリメタデータを準備（CI用）
cargo sqlx prepare
```

### 6. 開発サーバー起動

```bash
# 通常起動
cargo run

# ホットリロード付き起動
cargo watch -x run

# リリースビルドで起動
cargo run --release
```

## プロジェクト構造

```
rustresort/
├── Cargo.toml              # ワークスペース/依存関係
├── config/                 # 設定ファイル
│   ├── default.toml        # デフォルト設定
│   └── local.toml.example  # ローカル設定テンプレート
├── docs/                   # ドキュメント
├── migrations/             # DBマイグレーション
├── src/
│   ├── main.rs             # エントリーポイント
│   ├── lib.rs              # ライブラリルート
│   ├── config/             # 設定モジュール
│   ├── models/             # データモデル
│   ├── db/                 # データベース層
│   ├── cache/              # キャッシュ層
│   ├── api/                # API層
│   ├── service/            # サービス層
│   ├── federation/         # フェデレーション層
│   ├── transport/          # HTTPトランスポート
│   ├── media/              # メディア処理
│   ├── queue/              # バックグラウンドジョブ
│   └── util/               # ユーティリティ
└── tests/                  # テスト
    ├── integration/        # 統合テスト
    └── fixtures/           # テストデータ
```

## 開発ワークフロー

### ブランチ戦略

```
main
├── develop           # 開発ブランチ
│   ├── feature/*     # 機能開発
│   ├── fix/*         # バグ修正
│   └── refactor/*    # リファクタリング
└── release/*         # リリースブランチ
```

### コミットメッセージ規約

[Conventional Commits](https://www.conventionalcommits.org/)に従う：

```
feat: add user registration endpoint
fix: resolve timeline pagination issue
docs: update API documentation
refactor: extract http signature to module
test: add federation integration tests
chore: update dependencies
```

### コードスタイル

```bash
# フォーマット
cargo fmt

# リント
cargo clippy -- -D warnings

# 全チェック実行
cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

## テスト

### ユニットテスト

```bash
# 全ユニットテスト実行
cargo test

# 特定モジュールのテスト
cargo test models::

# テストカバレッジ（要 cargo-tarpaulin）
cargo tarpaulin --out Html
```

### 統合テスト

```bash
# 統合テスト実行
cargo test --test integration

# 特定のテストのみ
cargo test --test integration test_create_status
```

### テストの書き方

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[tokio::test]
    async fn test_create_status() {
        // Arrange
        let state = test_app_state().await;
        let account = create_test_account(&state).await;
        
        // Act
        let status = state.service.status.create(
            &account.id,
            CreateStatusRequest {
                content: "Hello, world!".to_string(),
                visibility: Visibility::Public,
                ..Default::default()
            },
        ).await.unwrap();
        
        // Assert
        assert_eq!(status.content, "<p>Hello, world!</p>");
        assert!(status.local);
    }
}
```

### フェデレーションテスト

ローカルでの複数インスタンステスト：

```bash
# インスタンス1を起動
PORT=8081 DOMAIN=instance1.localhost cargo run &

# インスタンス2を起動
PORT=8082 DOMAIN=instance2.localhost cargo run &

# テストスクリプト実行
./scripts/test-federation.sh
```

## デバッグ

### ログ設定

```toml
# config/local.toml
[logging]
level = "debug"  # trace, debug, info, warn, error
format = "pretty"
```

### リクエストトレース

```rust
// HTTPリクエストのトレース
#[tracing::instrument]
async fn handle_inbox(
    State(state): State<AppState>,
    body: String,
) -> Result<impl IntoResponse, AppError> {
    tracing::debug!(%body, "Received inbox request");
    // ...
}
```

### VSCode設定

`.vscode/launch.json`:
```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "type": "lldb",
            "request": "launch",
            "name": "Debug RustResort",
            "cargo": {
                "args": ["build", "--bin=rustresort"],
                "filter": {
                    "name": "rustresort",
                    "kind": "bin"
                }
            },
            "args": [],
            "cwd": "${workspaceFolder}",
            "env": {
                "RUST_LOG": "debug"
            }
        }
    ]
}
```

## 依存関係管理

### 依存関係の追加

```bash
# 依存関係追加
cargo add serde --features derive

# 開発依存関係
cargo add --dev tokio-test
```

### 依存関係の更新

```bash
# 更新可能なクレートを確認
cargo outdated

# 依存関係更新
cargo update

# セキュリティ監査
cargo audit
```

### Cargo.toml例

```toml
[package]
name = "rustresort"
version = "0.1.0"
edition = "2024"
rust-version = "1.82"

[dependencies]
# Webフレームワーク
axum = { version = "0.7", features = ["macros"] }
tokio = { version = "1", features = ["full"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "trace"] }

# シリアライゼーション
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# データベース（SQLx - SQLiteのみ）
sqlx = { version = "0.7", features = [
    "runtime-tokio",
    "sqlite",
    "chrono",
    "migrate",
] }

# SQLiteオンラインバックアップ
rusqlite = { version = "0.31", features = ["bundled", "backup"] }

# S3バックアップ
aws-sdk-s3 = "1.0"
aws-config = "1.0"

# キャッシュ
moka = { version = "0.12", features = ["future"] }

# 暗号/署名
rsa = "0.9"
sha2 = "0.10"
base64 = "0.21"

# HTTP
reqwest = { version = "0.11", features = ["json", "rustls-tls"] }

# ユーティリティ
ulid = "1"
chrono = { version = "0.4", features = ["serde"] }
thiserror = "1"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
config = "0.13"
url = { version = "2", features = ["serde"] }

[dev-dependencies]
tokio-test = "0.4"
mockall = "0.11"
sqlx = { version = "0.7", features = ["runtime-tokio", "sqlite"] }
```

### SQLxの使用パターン

```rust
use sqlx::{Pool, Sqlite, FromRow};

#[derive(Debug, FromRow)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub domain: Option<String>,
    // ...
}

// コンパイル時検証付きクエリ
pub async fn get_account_by_id(
    pool: &Pool<Sqlite>,
    id: &str,
) -> Result<Option<Account>, sqlx::Error> {
    sqlx::query_as!(Account,
        r#"
        SELECT id, username, domain
        FROM accounts
        WHERE id = ?
        "#,
        id
    )
    .fetch_optional(pool)
    .await
}

// 動的クエリ
pub async fn search_accounts(
    pool: &Pool<Sqlite>,
    query: &str,
    limit: i64,
) -> Result<Vec<Account>, sqlx::Error> {
    sqlx::query_as::<_, Account>(
        "SELECT id, username, domain FROM accounts WHERE username LIKE ? LIMIT ?"
    )
    .bind(format!("%{}%", query))
    .bind(limit)
    .fetch_all(pool)
    .await
}
```

## ビルドとデプロイ

### リリースビルド

```bash
# 最適化ビルド
cargo build --release

# バイナリサイズ最適化
RUSTFLAGS="-C link-arg=-s" cargo build --release
```

### Dockerビルド

`Dockerfile`:
```dockerfile
# ビルドステージ
FROM rust:1.82-slim AS builder

WORKDIR /app
COPY . .

RUN cargo build --release

# 実行ステージ
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/rustresort /usr/local/bin/

EXPOSE 8080

ENTRYPOINT ["rustresort"]
```

```bash
# Dockerイメージビルド
docker build -t rustresort:latest .

# 実行
docker run -p 8080:8080 -v ./config:/config -v ./data:/data rustresort:latest
```

## 貢献ガイドライン

### PRチェックリスト

- [ ] フォーマット: `cargo fmt --check`
- [ ] リント: `cargo clippy -- -D warnings`
- [ ] テスト: `cargo test`
- [ ] ドキュメント: 必要に応じて更新
- [ ] コミットメッセージ: Conventional Commits形式

### イシューテンプレート

```markdown
## 概要
簡潔な説明

## 再現手順
1. ...
2. ...
3. ...

## 期待される動作
何が起こるべきか

## 実際の動作
何が起こったか

## 環境
- RustResort version:
- OS:
- Rust version:
```

## 参考リソース

### ActivityPub/Federation

- [ActivityPub Spec](https://www.w3.org/TR/activitypub/)
- [ActivityStreams 2.0](https://www.w3.org/TR/activitystreams-core/)
- [GoToSocial Federation Docs](https://docs.gotosocial.org/en/latest/federation/)
- [Mastodon Federation](https://docs.joinmastodon.org/spec/activitypub/)

### Rust

- [The Rust Book](https://doc.rust-lang.org/book/)
- [Async Book](https://rust-lang.github.io/async-book/)
- [Axum Documentation](https://docs.rs/axum/latest/axum/)
- [Tokio Tutorial](https://tokio.rs/tokio/tutorial)

## トラブルシューティング

### ビルドエラー

```bash
# 依存関係キャッシュクリア
cargo clean

# ロックファイル再生成
rm Cargo.lock
cargo build
```

### DB接続エラー

```bash
# SQLiteファイルの権限確認
ls -la data/

# PostgreSQL接続テスト
psql -h localhost -U user -d rustresort
```

### Federation問題

1. HTTP Signature検証に失敗する場合：
   - サーバー時刻の同期を確認
   - `Date`ヘッダーの形式を確認

2. リモートアクターが取得できない場合：
   - WebFinger応答を確認
   - Accept: application/activity+jsonヘッダーを確認
