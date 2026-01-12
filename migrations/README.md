# Database Schema

## Overview

RustResortは単一のSQLiteデータベースを使用します。すべてのテーブル定義は`schema.sql`に統合されています。

## Schema File

- **`schema.sql`** - 統合されたデータベーススキーマ（すべてのテーブル、インデックス、トリガー、ビューを含む）

## Archive

以前のphase分けされたマイグレーションファイルは`archive/`ディレクトリに保存されています:

- `001_initial.sql` - 初期スキーマ
- `002_oauth.sql` - OAuth関連テーブル
- `003_social_features.sql` - ソーシャル機能
- `004_phase3_features.sql` - 投票、予約投稿、会話
- `005_search_features.sql` - 検索とハッシュタグ

## Tables

### Core Tables
- `account` - アカウント情報（シングルユーザー）
- `statuses` - ステータス（投稿）
- `media_attachments` - メディア添付ファイル

### Social Relationships
- `follows` - フォロー関係
- `followers` - フォロワー
- `follow_requests` - フォローリクエスト
- `account_blocks` - アカウントブロック
- `account_mutes` - アカウントミュート

### User Interactions
- `notifications` - 通知
- `favourites` - お気に入り
- `bookmarks` - ブックマーク
- `reposts` - ブースト（リポスト）

### Lists
- `lists` - リスト
- `list_accounts` - リストメンバー

### Filters
- `filters` - フィルター（v1 API）
- `filter_keywords` - フィルターキーワード（v2 API）

### Polls
- `polls` - 投票
- `poll_options` - 投票選択肢
- `poll_votes` - 投票記録

### Scheduled Statuses
- `scheduled_statuses` - 予約投稿

### Conversations
- `conversations` - 会話
- `conversation_participants` - 会話参加者
- `conversation_statuses` - 会話ステータス

### Search & Hashtags
- `hashtags` - ハッシュタグ
- `status_hashtags` - ステータス-ハッシュタグ関連
- `statuses_fts` - 全文検索インデックス（FTS5）
- `hashtag_stats` - ハッシュタグ統計（ビュー）

### OAuth & Authentication
- `oauth_apps` - OAuthアプリ
- `oauth_tokens` - OAuthトークン

### Moderation
- `domain_blocks` - ドメインブロック

### Settings
- `settings` - 設定（キー・バリュー）

## Full-Text Search

SQLiteのFTS5を使用してステータスの全文検索を実装しています。

- `statuses_fts` - FTS5仮想テーブル
- トリガーで自動的にインデックスを更新

## Views

- `hashtag_stats` - ハッシュタグの使用統計

## Indexes

すべてのテーブルに適切なインデックスが設定されています:

- 外部キー
- 検索に使用されるカラム
- ソートに使用されるカラム

## Migration Strategy

### 開発環境

開発環境では、`schema.sql`を直接実行してデータベースを初期化します:

```bash
sqlite3 data/rustresort.db < migrations/schema.sql
```

### 本番環境

本番環境では、SQLxのマイグレーション機能を使用します:

```rust
sqlx::migrate!("./migrations")
    .run(&pool)
    .await?;
```

## Schema Updates

スキーマを更新する場合:

1. `schema.sql`を直接編集
2. 既存のデータベースに対する変更の場合は、ALTER TABLE文を含む新しいマイグレーションファイルを作成
3. テストを実行して互換性を確認

## Notes

- すべてのテーブルは`IF NOT EXISTS`を使用して作成されるため、冪等性があります
- 外部キー制約は`ON DELETE CASCADE`を使用して自動的に関連データを削除します
- 日時は`TEXT`型でISO 8601形式（RFC 3339）で保存されます
- IDはULIDを使用して生成されます
