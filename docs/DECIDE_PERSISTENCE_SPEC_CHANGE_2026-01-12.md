# decide_persistence 仕様変更レポート

**Date**: 2026-01-12  
**変更内容**: メンションと引用ブーストの処理を改善

## 変更概要

`decide_persistence`関数と`handle_announce`関数を更新し、以下の仕様を実装しました：

1. **他者から自分へのメンションは必ずpersist**
2. **引用ブースト（quote boost）でメンションされた場合もpersist**

## 実装の詳細

### 1. decide_persistence の変更

#### Before:
```rust
Some(ActivityType::Like) | Some(ActivityType::Announce) => {
    // Like/Announce of our status -> Persist (creates notification)
    PersistenceDecision::Persist
}
```

#### After:
```rust
Some(ActivityType::Like) => {
    // Like of our status -> Persist (creates notification)
    // The handler will check if it's actually our status
    PersistenceDecision::Persist
}
Some(ActivityType::Announce) => {
    // Check if it's a quote boost (has content) or regular boost
    if let Some(object) = activity.get("object") {
        // Quote boost: Announce activity with embedded Note/Article
        if object.is_object() && object.get("type").is_some() {
            // Check if the quote mentions us
            if self.mentions_local_user(object) {
                // Quote boost mentioning us -> Persist
                return PersistenceDecision::Persist;
            }
        } else if let Some(object_uri) = object.as_str() {
            // Regular boost: just a URI reference
            // Check if it's our status being boosted
            if self.is_local_status(object_uri) {
                // Boost of our status -> Persist (creates notification)
                return PersistenceDecision::Persist;
            }
        }
    }
    // Boost of someone else's status -> Ignore
    PersistenceDecision::Ignore
}
```

**変更点:**
- `Like`と`Announce`を分離
- `Announce`で引用ブースト（embedded object）と通常のブースト（URI reference）を区別
- 引用ブーストでメンションされた場合は`Persist`
- 自分の投稿がブーストされた場合のみ`Persist`
- それ以外のブーストは`Ignore`

### 2. Create アクティビティの改善

#### Before:
```rust
Some(ActivityType::Create) => {
    if let Some(object) = activity.get("object") {
        if self.mentions_local_user(object) {
            // Create with mention -> Persist (notification)
            return PersistenceDecision::Persist;
        }
    }
    PersistenceDecision::Ignore
}
```

#### After:
```rust
Some(ActivityType::Create) => {
    if let Some(object) = activity.get("object") {
        if self.mentions_local_user(object) {
            // Create with mention from others -> Persist (notification)
            return PersistenceDecision::Persist;
        }
        // Check if it's a reply to our post
        if let Some(in_reply_to) = object.get("inReplyTo").and_then(|r| r.as_str()) {
            if self.is_local_status(in_reply_to) {
                // Reply to our post -> Persist (notification)
                return PersistenceDecision::Persist;
            }
        }
    }
    PersistenceDecision::Ignore
}
```

**変更点:**
- リプライの検出を追加
- 自分の投稿へのリプライも`Persist`

### 3. handle_announce の変更

#### Before:
```rust
async fn handle_announce(...) -> Result<(), AppError> {
    let object = activity
        .get("object")
        .and_then(|o| o.as_str())  // 常にURIとして扱っていた
        .ok_or_else(...)?;

    if !self.is_local_status(object) {
        return Ok(());
    }

    // Create reblog notification
    ...
}
```

#### After:
```rust
async fn handle_announce(...) -> Result<(), AppError> {
    let object = activity
        .get("object")
        .ok_or_else(...)?;

    let actor_address = self.extract_actor_address(actor_uri);

    // Check if it's a quote boost (embedded object) or regular boost (URI)
    if object.is_object() {
        // Quote boost: Announce with embedded Note/Article
        if self.mentions_local_user(object) {
            // Create mention notification for quote boost
            ...
        }
    } else if let Some(object_uri) = object.as_str() {
        // Regular boost: just a URI reference
        if self.is_local_status(object_uri) {
            // Create reblog notification for boost of our status
            ...
        }
    }

    Ok(())
}
```

**変更点:**
- `object`がオブジェクトか文字列かを判定
- 引用ブースト（embedded object）の場合:
  - メンションをチェック
  - メンションがあれば`mention`通知を作成
- 通常のブースト（URI reference）の場合:
  - 自分の投稿かチェック
  - 自分の投稿なら`reblog`通知を作成

## 処理フロー

### 引用ブーストの場合

```
Announce Activity
└── object (embedded Note/Article)
    ├── type: "Note"
    ├── content: "引用コメント @localuser@example.com"
    ├── tag: [{ type: "Mention", href: "..." }]
    └── ...

↓

mentions_local_user() → true

↓

通知作成: type = "mention"
```

### 通常のブーストの場合

```
Announce Activity
└── object: "https://example.com/users/localuser/statuses/123"

↓

is_local_status() → true

↓

通知作成: type = "reblog"
```

## テスト結果

すべてのテストがパスしています：

```
test result: ok. 18 passed; 0 failed; 2 ignored; 0 measured
```

## 影響範囲

### 変更されたファイル:
- `src/federation/activity.rs`
  - `decide_persistence()` - 判定ロジックの改善
  - `handle_announce()` - 引用ブースト対応

### 影響を受ける機能:
- ✅ 通常のブースト通知 - 引き続き動作
- ✅ 引用ブースト通知 - 新規対応
- ✅ メンション通知 - 引き続き動作
- ✅ リプライ通知 - 改善（明示的に検出）

## ActivityPub仕様との整合性

### 通常のAnnounce（ブースト）
```json
{
  "@context": "https://www.w3.org/ns/activitystreams",
  "type": "Announce",
  "actor": "https://remote.example/users/alice",
  "object": "https://local.example/users/bob/statuses/123"
}
```

### 引用ブースト（Quote Post）
```json
{
  "@context": "https://www.w3.org/ns/activitystreams",
  "type": "Announce",
  "actor": "https://remote.example/users/alice",
  "object": {
    "type": "Note",
    "id": "https://remote.example/users/alice/statuses/456",
    "content": "引用コメント @bob@local.example",
    "tag": [
      {
        "type": "Mention",
        "href": "https://local.example/users/bob",
        "name": "@bob@local.example"
      }
    ],
    "quoteUrl": "https://local.example/users/bob/statuses/123"
  }
}
```

## まとめ

### 実装された仕様:
1. ✅ **他者から自分へのメンションは必ずpersist**
   - Create アクティビティのメンション検出
   - 引用ブーストのメンション検出
   - リプライの検出

2. ✅ **引用ブーストでメンションされた場合もpersist**
   - Announce アクティビティで embedded object を検出
   - embedded object 内のメンションをチェック
   - メンション通知を作成

### 通知の種類:
- `follow` - フォロー通知
- `mention` - メンション通知（投稿、リプライ、引用ブースト）
- `reblog` - ブースト通知（通常のブースト）
- `favourite` - いいね通知

すべての変更は後方互換性を保ちながら実装されており、既存の機能に影響を与えません。
