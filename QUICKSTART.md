# RustResort - Quick Start Guide

## 🚀 クイックスタート

### 1. 前提条件

- Rust 1.75以上
- SQLite 3.x
- (オプション) MinIO（ローカルS3互換ストレージ）

### 2. セットアップ

```bash
# 依存関係のインストール
cargo build

# 設定ファイルのコピー
cp config/local.toml.example config/local.toml

# 設定ファイルを編集（必要に応じて）
vim config/local.toml
```

### 3. データベース初期化

データベースは初回起動時に自動的に作成・マイグレーションされます。

### 4. サーバー起動

```bash
# 開発モード
cargo run

# リリースモード
cargo run --release
```

サーバーは `http://localhost:3000` で起動します。

### 5. 動作確認

```bash
# ヘルスチェック
curl http://localhost:3000/health

# NodeInfo
curl http://localhost:3000/.well-known/nodeinfo

# NodeInfo 2.0
curl http://localhost:3000/nodeinfo/2.0
```

## 📋 主要エンドポイント

### Well-known

- `GET /.well-known/webfinger?resource=acct:user@domain` - WebFinger
- `GET /.well-known/nodeinfo` - NodeInfo links
- `GET /.well-known/host-meta` - Host metadata

### ActivityPub

- `GET /users/:username` - Actor document
- `GET /users/:username/outbox` - Outbox collection
- `GET /users/:username/inbox` - Inbox (POST for federation)
- `GET /users/:username/followers` - Followers collection
- `GET /users/:username/following` - Following collection

### Mastodon API

- `GET /api/v1/instance` - Instance information
- その他のエンドポイントは実装中

## 🧪 テスト

```bash
# ユニットテスト
cargo test --lib

# 統合テスト
cargo test

# 特定のテスト
cargo test test_database_connection
```

## 🔧 開発

### ログレベルの変更

```bash
# 環境変数で設定
RUSTRESORT__LOGGING__LEVEL=debug cargo run

# または config/local.toml で設定
[logging]
level = "debug"
```

### MinIOの起動（ローカル開発用）

```bash
# Docker
docker run -p 9000:9000 -p 9001:9001 \
  -e MINIO_ROOT_USER=minioadmin \
  -e MINIO_ROOT_PASSWORD=minioadmin \
  minio/minio server /data --console-address ":9001"

# バケット作成
mc alias set local http://localhost:9000 minioadmin minioadmin
mc mb local/rustresort-media
mc mb local/rustresort-backup
```

## 📚 ドキュメント

- [ARCHITECTURE.md](docs/ARCHITECTURE.md) - アーキテクチャ概要
- [DEVELOPMENT.md](docs/DEVELOPMENT.md) - 開発ガイド
- [FEDERATION.md](docs/FEDERATION.md) - フェデレーション仕様
- [API_IMPLEMENTATION.md](API_IMPLEMENTATION.md) - API実装詳細

## 🐛 トラブルシューティング

### データベースエラー

```bash
# データベースを削除して再作成
rm data/rustresort.db
cargo run
```

### マイグレーションエラー

```bash
# SQLxマイグレーションの確認
sqlx migrate info --database-url sqlite:data/rustresort.db
```

### ポート競合

```bash
# 別のポートで起動
RUSTRESORT__SERVER__PORT=3001 cargo run
```

## 📝 設定

設定は以下の順序で読み込まれます（後の設定が優先）：

1. `config/default.toml` - デフォルト設定
2. `config/local.toml` - ローカル設定
3. 環境変数 `RUSTRESORT__*` - 環境変数による上書き

### 環境変数の例

```bash
# サーバー設定
export RUSTRESORT__SERVER__HOST=0.0.0.0
export RUSTRESORT__SERVER__PORT=3000
export RUSTRESORT__SERVER__DOMAIN=example.com
export RUSTRESORT__SERVER__PROTOCOL=https

# データベース
export RUSTRESORT__DATABASE__PATH=data/rustresort.db

# ログ
export RUSTRESORT__LOGGING__LEVEL=debug
export RUSTRESORT__LOGGING__FORMAT=json
```

## 🚢 デプロイ

### ビルド

```bash
# リリースビルド
cargo build --release

# バイナリは target/release/rustresort に生成されます
```

### systemdサービス（例）

```ini
[Unit]
Description=RustResort ActivityPub Server
After=network.target

[Service]
Type=simple
User=rustresort
WorkingDirectory=/opt/rustresort
ExecStart=/opt/rustresort/rustresort
Restart=always
Environment="RUSTRESORT__SERVER__DOMAIN=your-domain.com"
Environment="RUSTRESORT__SERVER__PROTOCOL=https"

[Install]
WantedBy=multi-user.target
```

## 📊 現在の実装状況

- ✅ Phase 0 (Foundation): 100%
- ✅ Phase 1 (API): 85%
- ⏳ Phase 2 (Federation): 30%
- ⏳ Phase 3 (Client): 0%

詳細は [IMPLEMENTATION_FINAL.md](IMPLEMENTATION_FINAL.md) を参照してください。
