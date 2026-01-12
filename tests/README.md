# RustResort E2E Tests

このディレクトリには、RustResortの主要シナリオに対するEnd-to-End(E2E)テストが含まれています。

## 📁 ディレクトリ構造

```
tests/
├── common/
│   └── mod.rs          # 共通テストユーティリティ(TestServerヘルパー)
├── e2e_health.rs       # ヘルスチェック・基本サーバー機能
├── e2e_wellknown.rs    # .well-knownエンドポイント(WebFinger, NodeInfo)
├── e2e_account.rs      # アカウント操作(取得、更新、フォロー)
├── e2e_status.rs       # ステータス操作(投稿、削除、お気に入り、ブースト)
├── e2e_timeline.rs     # タイムライン(ホーム、公開、ローカル、ハッシュタグ)
└── e2e_activitypub.rs  # ActivityPub連携(Actor, Inbox, Outbox)
```

## 🚀 テスト実行方法

### 全E2Eテスト実行
```bash
cargo test --tests
```

### 特定のテストスイート実行
```bash
# ヘルスチェックテスト
cargo test --test e2e_health

# アカウント関連テスト
cargo test --test e2e_account

# ステータス関連テスト
cargo test --test e2e_status

# タイムライン関連テスト
cargo test --test e2e_timeline

# ActivityPub関連テスト
cargo test --test e2e_activitypub

# .well-known関連テスト
cargo test --test e2e_wellknown
```

### 特定のテストケース実行
```bash
# テスト名で実行
cargo test test_health_check
cargo test test_create_status_with_auth

# パターンマッチで実行
cargo test timeline
cargo test account
```

### 詳細出力付きで実行
```bash
# 標準出力を表示
cargo test --test e2e_health -- --nocapture

# テスト名も表示
cargo test --test e2e_health -- --nocapture --show-output
```

## 🧪 テストスイート詳細

### 1. Health Tests (`e2e_health.rs`)
基本的なサーバー機能のテスト

- ✅ ヘルスチェックエンドポイント
- ✅ サーバー起動確認
- ✅ CORSヘッダー検証
- ✅ 未知のルートで404返却

### 2. WellKnown Tests (`e2e_wellknown.rs`)
Fediverse連携に必要な.well-knownエンドポイントのテスト

- ✅ WebFingerエンドポイント
- ✅ NodeInfo検出
- ✅ host-metaエンドポイント
- ✅ アカウント付きWebFinger

### 3. Account Tests (`e2e_account.rs`)
アカウント管理機能のテスト

- ⚠️ 認証なしでの認証情報確認(401エラー)
- ✅ 認証付き認証情報確認
- ✅ IDによるアカウント取得
- ✅ アカウント情報更新
- ✅ アカウントのステータス一覧
- ✅ フォロワー一覧
- ✅ フォロー中一覧

### 4. Status Tests (`e2e_status.rs`)
ステータス(投稿)管理機能のテスト

- ⚠️ 認証なしでのステータス作成(401エラー)
- ✅ 認証付きステータス作成
- ✅ ステータス取得
- ✅ ステータス削除
- ✅ お気に入り登録
- ✅ ブースト(リブログ)
- ✅ ステータスのコンテキスト取得

### 5. Timeline Tests (`e2e_timeline.rs`)
タイムライン表示機能のテスト

- ⚠️ 認証なしでのホームタイムライン(401エラー)
- ✅ 認証付きホームタイムライン
- ✅ 公開タイムライン
- ✅ ローカルタイムライン
- ✅ ページネーション
- ✅ ハッシュタグタイムライン
- ✅ max_idパラメータ
- ✅ since_idパラメータ

### 6. ActivityPub Tests (`e2e_activitypub.rs`)
ActivityPub連携機能のテスト

- ✅ Actorエンドポイント
- ✅ Inboxエンドポイント
- ✅ Outboxエンドポイント
- ✅ Followersコレクション
- ✅ Followingコレクション
- ✅ ステータスのActivity表現
- ✅ 共有Inbox
- ✅ コンテンツネゴシエーション

## 🛠️ TestServerヘルパー

`tests/common/mod.rs`に実装された共通テストユーティリティ。

### 主な機能

```rust
use common::TestServer;

#[tokio::test]
async fn my_test() {
    // テストサーバーを起動
    let server = TestServer::new().await;
    
    // HTTPリクエストを送信
    let response = server.client
        .get(&server.url("/health"))
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
}
```

### 提供されるメソッド

- `TestServer::new()` - 新しいテストサーバーインスタンスを作成
- `server.url(path)` - 完全なURLを生成
- `server.create_test_account()` - テスト用アカウントを作成
- `server.create_test_token()` - テスト用認証トークンを作成
- `server.state` - AppStateへのアクセス
- `server.client` - HTTPクライアント

### 特徴

- **独立性**: 各テストは独立したサーバーインスタンスを使用
- **一時DB**: テストごとに新しいSQLiteデータベースを作成
- **自動ポート**: OSが自動的に空きポートを割り当て
- **自動クリーンアップ**: テスト終了後に自動的にリソースを解放

## 📊 現在のテスト状況

**総テスト数**: 39  
**成功**: 35 (89.7%)  
**失敗**: 3 (7.7%)  

### 失敗しているテスト

以下の3つのテストは、認証ミドルウェアの実装が完了していないため失敗しています:

1. `test_verify_credentials_without_auth` - 401ではなく404を返す
2. `test_create_status_without_auth` - 401ではなく404を返す
3. `test_home_timeline_without_auth` - 401ではなく404を返す

これらは実装の進捗に伴い、自然に解決される予定です。

## 🔧 テストの追加方法

### 新しいテストケースの追加

既存のテストファイルに追加:

```rust
#[tokio::test]
async fn test_my_new_feature() {
    let server = TestServer::new().await;
    
    // テストロジック
    let response = server.client
        .get(&server.url("/api/v1/my_endpoint"))
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
}
```

### 新しいテストスイートの追加

1. `tests/e2e_myfeature.rs`を作成
2. `mod common;`を追加
3. テストケースを実装

```rust
mod common;

use common::TestServer;

#[tokio::test]
async fn test_my_feature() {
    let server = TestServer::new().await;
    // テストロジック
}
```

## 📈 ベストプラクティス

### テストの独立性
- 各テストは他のテストに依存しない
- テストの実行順序に依存しない
- 共有状態を使用しない

### テストデータ
- テストごとに新しいデータを作成
- ハードコードされたIDを避ける
- 一時的なデータベースを使用

### アサーション
- 明確なアサーションメッセージ
- 複数の条件を個別にテスト
- エッジケースもカバー

### パフォーマンス
- 不要な待機を避ける
- 並列実行可能に保つ
- 重いセットアップは共通化

## 🐛 トラブルシューティング

### テストがタイムアウトする
```bash
# タイムアウト時間を延長
RUST_TEST_TIMEOUT=60 cargo test
```

### ポートが既に使用されている
TestServerは自動的に空きポートを使用するため、通常この問題は発生しません。

### データベースエラー
一時ディレクトリの権限を確認してください。

### 並列実行の問題
```bash
# シーケンシャルに実行
cargo test -- --test-threads=1
```

## 📚 関連ドキュメント

- [E2E Test Report](../docs/E2E_TEST_REPORT.md) - 詳細なテスト実行レポート
- [DEVELOPMENT.md](../docs/DEVELOPMENT.md) - 開発ガイド
- [API.md](../docs/API.md) - API仕様
- [ROADMAP.md](../docs/ROADMAP.md) - 実装ロードマップ

## 🎯 今後の予定

### 短期
- [ ] 認証ミドルウェアの修正
- [ ] OAuth2フローのテスト追加
- [ ] メディアアップロードのテスト追加

### 中期
- [ ] HTTP Signaturesのテスト追加
- [ ] Activity配信のテスト追加
- [ ] パフォーマンステストの追加

### 長期
- [ ] 統合テスト環境の構築
- [ ] CI/CDパイプラインへの統合
- [ ] カバレッジレポートの自動生成
