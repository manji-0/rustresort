# RustResort 実装ロードマップ

## 概要

このドキュメントでは、RustResortの実装フェーズとマイルストーンを定義します。
GoToSocialのロードマップを参考に、段階的な機能実装を計画します。

## フェーズ概要

```
Phase 0: Foundation (基盤)
    ↓
Phase 1: Local Instance (ローカルインスタンス)
    ↓
Phase 2: Federation (フェデレーション)
    ↓
Phase 3: Polish (洗練)
    ↓
Phase 4: Advanced (高度な機能)
    ↓
Beta Release
    ↓
Stable Release
```

---

## Phase 0: Foundation（基盤）

**目標**: プロジェクトの基盤を構築

### 0.1 プロジェクト初期化
- [ ] Cargo.tomlセットアップ
- [ ] ワークスペース構成（必要に応じて）
- [ ] 基本ディレクトリ構造の作成
- [ ] CI/CD設定（GitHub Actions）

### 0.2 設定管理
- [ ] `config-rs`による設定読み込み
- [ ] 環境変数サポート
- [ ] 設定ファイルバリデーション
- [ ] 設定構造体の型定義

### 0.3 ロギング/トレーシング
- [ ] `tracing`セットアップ
- [ ] 構造化ログ出力
- [ ] ログレベル設定

### 0.4 データベース基盤
- [ ] Diesel/SeaORMセットアップ
- [ ] コネクションプール設定
- [ ] マイグレーションシステム

### 0.5 HTTPサーバー基盤
- [ ] Axumサーバー初期化
- [ ] ミドルウェア設定（CORS, Trace, etc.）
- [ ] ヘルスチェックエンドポイント
- [ ] エラーハンドリング基盤

---

## Phase 1: Local Instance（ローカルインスタンス）

**目標**: ローカルのみで動作するMastodon互換API

### 1.1 コアモデル実装
- [ ] Account モデル
- [ ] User モデル
- [ ] Status モデル
- [ ] MediaAttachment モデル
- [ ] Follow モデル
- [ ] Notification モデル
- [ ] Application/Token モデル

### 1.2 データベースリポジトリ
- [ ] AccountRepository
- [ ] UserRepository
- [ ] StatusRepository
- [ ] MediaRepository
- [ ] FollowRepository
- [ ] NotificationRepository
- [ ] マイグレーションスクリプト

### 1.3 認証システム
- [ ] OAuth2アプリ登録 (`POST /api/v1/apps`)
- [ ] 認可フロー (`GET /oauth/authorize`)
- [ ] トークン発行 (`POST /oauth/token`)
- [ ] トークン検証ミドルウェア
- [ ] パスワードハッシュ（Argon2）

### 1.4 アカウントAPI
- [ ] アカウント取得 (`GET /api/v1/accounts/:id`)
- [ ] 認証情報確認 (`GET /api/v1/accounts/verify_credentials`)
- [ ] プロフィール更新 (`PATCH /api/v1/accounts/update_credentials`)
- [ ] アカウント検索
- [ ] フォロー/アンフォロー

### 1.5 ステータスAPI
- [ ] ステータス作成 (`POST /api/v1/statuses`)
- [ ] ステータス取得 (`GET /api/v1/statuses/:id`)
- [ ] ステータス削除 (`DELETE /api/v1/statuses/:id`)
- [ ] コンテキスト取得
- [ ] お気に入り/ブースト

### 1.6 タイムラインAPI
- [ ] ホームタイムライン (`GET /api/v1/timelines/home`)
- [ ] ローカルタイムライン
- [ ] ページネーション

### 1.7 メディア処理
- [ ] 画像アップロード
- [ ] サムネイル生成
- [ ] ファイルストレージ

### 1.8 通知
- [ ] 通知一覧取得
- [ ] 通知作成（内部）
- [ ] 通知クリア

### 1.9 Well-known エンドポイント
- [ ] WebFinger (`/.well-known/webfinger`)
- [ ] NodeInfo (`/.well-known/nodeinfo`, `/nodeinfo/2.0`)
- [ ] host-meta

### 1.10 インスタンス情報
- [ ] インスタンス情報 (`GET /api/v1/instance`)
- [ ] インスタンスルール
- [ ] カスタム絵文字

---

## Phase 2: Federation（フェデレーション）

**目標**: Fediverseとの連携

### 2.1 HTTP Signatures
- [ ] 署名生成
- [ ] 署名検証
- [ ] 公開鍵キャッシュ

### 2.2 ActivityPubアクター
- [ ] アクター表現 (`GET /users/{username}`)
- [ ] Inbox (`POST /users/{username}/inbox`)
- [ ] Outbox (`GET /users/{username}/outbox`)
- [ ] Followers/Following コレクション

### 2.3 Dereferencing
- [ ] リモートアクター取得
- [ ] リモートステータス取得
- [ ] WebFingerルックアップ

### 2.4 Activity受信
- [ ] Create (Note) 処理
- [ ] Follow 処理
- [ ] Accept/Reject 処理
- [ ] Undo 処理
- [ ] Like 処理
- [ ] Announce 処理
- [ ] Delete 処理

### 2.5 Activity配信
- [ ] 配信キュー
- [ ] ワーカー実装
- [ ] SharedInbox最適化
- [ ] リトライ処理

### 2.6 ドメイン管理
- [ ] ドメインブロック
- [ ] ドメイン許可（Allowlist）
- [ ] インスタンス情報取得

### 2.7 リモートメディア
- [ ] リモート画像プロキシ
- [ ] メディアキャッシュ

---

## Phase 3: Polish（洗練）

**目標**: 品質向上と機能拡充

### 3.1 サインアップフロー
- [ ] ユーザー登録
- [ ] メール確認
- [ ] 管理者承認

### 3.2 管理機能
- [ ] 管理者ダッシュボード
- [ ] レポート機能
- [ ] アカウント凍結/停止
- [ ] アナウンス

### 3.3 高度な投稿機能
- [ ] ステータス編集
- [ ] ポール（投票）
- [ ] スケジュール投稿

### 3.4 ユーザー詳細設定
- [ ] ミュート
- [ ] フィルター
- [ ] ブックマーク
- [ ] リスト

### 3.5 検索
- [ ] ローカル検索
- [ ] ハッシュタグ検索
- [ ] アカウント検索

### 3.6 パフォーマンス最適化
- [ ] キャッシュ戦略見直し
- [ ] データベースインデックス最適化
- [ ] バッチ処理改善

---

## Phase 4: Advanced（高度な機能）

**目標**: 差別化機能と高度なセキュリティ

### 4.1 二要素認証
- [ ] TOTP実装
- [ ] バックアップコード

### 4.2 アカウント移行
- [ ] Move Activity
- [ ] alsoKnownAs設定
- [ ] フォロワー移行

### 4.3 Streaming API
- [ ] WebSocket接続
- [ ] ユーザーストリーム
- [ ] 公開タイムラインストリーム

### 4.4 Web Push
- [ ] Push通知登録
- [ ] 通知配信

### 4.5 S3ストレージ
- [ ] S3互換APIサポート
- [ ] メディア移行ツール

### 4.6 インポート/エクスポート
- [ ] フォロー/ブロックリストのインポート
- [ ] アカウントデータエクスポート

---

## Beta Release

**基準**:
- Phase 0-3 が完了
- Phase 4 の一部が完了
- 主要なMastodonクライアントで動作確認
- GoToSocial, Mastodon との相互運用性確認
- セキュリティ監査

---

## Stable Release

**追加基準**:
- Phase 4 完了
- ドキュメント完備
- API安定性保証
- マイグレーションパス提供

---

## 優先度付きタスク（Phase 1-2向け）

### 高優先度
1. 設定管理とロギング
2. データベース基盤
3. 認証システム（OAuth2）
4. ステータスCRUD
5. タイムライン
6. HTTP Signatures
7. Inbox処理

### 中優先度
1. メディアアップロード
2. 通知システム
3. フォロー関係
4. Activity配信
5. Dereferencing

### 低優先度（Phase 3以降）
1. 検索機能
2. 管理機能
3. 高度な投稿機能

---

## 予想タイムライン

| Phase | 期間 | 備考 |
|-------|------|------|
| Phase 0 | 1-2週間 | 基盤構築 |
| Phase 1 | 4-6週間 | ローカル機能 |
| Phase 2 | 4-6週間 | フェデレーション |
| Phase 3 | 4-6週間 | 品質向上 |
| Phase 4 | 4-6週間 | 高度機能 |
| Beta | - | 継続的改善 |
| Stable | - | - |

**注**: これは1人のフルタイム開発者を想定した見積もり。

---

## 成功指標

### 機能完成度
- [ ] Mastodon公式Webクライアントで動作
- [ ] Tuskで動作
- [ ] Tusky (Android)で動作
- [ ] Ice Cubes (iOS)で動作

### フェデレーション
- [ ] Mastodonとフォロー/投稿連携
- [ ] GoToSocialと相互運用
- [ ] Misskey/Pleromaと基本連携

### 品質
- [ ] テストカバレッジ70%以上
- [ ] 重大なセキュリティ脆弱性なし
- [ ] ドキュメント完備

---

## 参考

- [GoToSocial Roadmap](https://github.com/superseriousbusiness/gotosocial/blob/main/ROADMAP.md)
- [Mastodon API Documentation](https://docs.joinmastodon.org/api/)
- [ActivityPub Specification](https://www.w3.org/TR/activitypub/)
