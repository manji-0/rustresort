# データベースパフォーマンス分析レポート

## 📊 分析概要

**分析日時**: 2026-01-11 23:29  
**対象**: `src/data/database.rs` および関連API  
**目的**: N+1問題およびパフォーマンスボトルネックの特定

## 🔴 発見された問題

### 1. **N+1問題: 通知取得** (重大)

**場所**: `src/api/mastodon/notifications.rs:get_notifications()`

**問題**:
```rust
for notification in notifications {
    // 各通知ごとにステータスを個別に取得 ← N+1問題
    let status = if let Some(status_uri) = &notification.status_uri {
        state.db.get_status_by_uri(status_uri).await.ok().flatten()
    } else {
        None
    };
}
```

**影響**:
- 20件の通知がある場合、最大21回のクエリ実行（1回の通知取得 + 20回のステータス取得）
- レスポンスタイムの大幅な増加
- データベース負荷の増加

**推奨修正**:
```rust
// 1. すべてのステータスURIを収集
let status_uris: Vec<&str> = notifications
    .iter()
    .filter_map(|n| n.status_uri.as_deref())
    .collect();

// 2. 一括取得（新しいメソッドが必要）
let statuses = state.db.get_statuses_by_uris(&status_uris).await?;
let status_map: HashMap<&str, &Status> = statuses
    .iter()
    .map(|s| (s.uri.as_str(), s))
    .collect();

// 3. マップから取得
for notification in notifications {
    let status = notification.status_uri
        .as_deref()
        .and_then(|uri| status_map.get(uri).copied());
}
```

### 2. **潜在的N+1問題: ブックマーク/お気に入り一覧** (中)

**場所**: `src/api/mastodon/bookmarks.rs`

**問題**:
```rust
// ステータスIDのリストを取得
let status_ids = state.db.get_bookmarked_status_ids(limit).await?;

// 各IDごとにステータスを取得 ← 潜在的N+1
for status_id in status_ids {
    if let Some(status) = state.db.get_status(&status_id).await? {
        // ...
    }
}
```

**推奨修正**:
```rust
// JOINを使用した一括取得
pub async fn get_bookmarked_statuses(&self, limit: usize) -> Result<Vec<Status>, AppError> {
    let statuses = sqlx::query_as::<_, Status>(
        r#"
        SELECT s.* FROM statuses s
        INNER JOIN bookmarks b ON s.id = b.status_id
        ORDER BY b.created_at DESC
        LIMIT ?
        "#
    )
    .bind(limit as i64)
    .fetch_all(&self.pool)
    .await?;
    
    Ok(statuses)
}
```

### 3. **潜在的N+1問題: リストメンバー取得** (低)

**場所**: `src/api/mastodon/lists.rs`

**現状**: アカウントアドレスのリストのみを返すため、現時点では問題なし

**将来的な懸念**: アカウント詳細を返す場合、N+1問題が発生する可能性

## 🟡 最適化の余地がある箇所

### 1. **インデックスの追加検討**

**現在のインデックス**:
```sql
CREATE INDEX IF NOT EXISTS idx_statuses_uri ON statuses(uri);
CREATE INDEX IF NOT EXISTS idx_statuses_created_at ON statuses(created_at DESC);
```

**追加推奨インデックス**:
```sql
-- 複合インデックス: アカウント別のステータス取得用
CREATE INDEX IF NOT EXISTS idx_statuses_account_created 
ON statuses(account_address, created_at DESC);

-- 複合インデックス: ローカルステータスの取得用
CREATE INDEX IF NOT EXISTS idx_statuses_local_created 
ON statuses(is_local, created_at DESC) 
WHERE is_local = 1;

-- 通知の効率的な取得用
CREATE INDEX IF NOT EXISTS idx_notifications_read_created 
ON notifications(read, created_at DESC);
```

### 2. **クエリの最適化**

**タイムライン取得**:
```sql
-- 現在
SELECT * FROM statuses 
WHERE is_local = 1 
ORDER BY created_at DESC 
LIMIT ?

-- 最適化案: 必要なカラムのみ選択
SELECT id, uri, content, visibility, created_at 
FROM statuses 
WHERE is_local = 1 
ORDER BY created_at DESC 
LIMIT ?
```

### 3. **バッチ処理の導入**

**メディア添付ファイルの取得**:
```rust
// 現在: 個別取得の可能性
// 推奨: バッチ取得メソッドの追加
pub async fn get_media_by_status_ids(
    &self, 
    status_ids: &[&str]
) -> Result<HashMap<String, Vec<MediaAttachment>>, AppError> {
    let media = sqlx::query_as::<_, MediaAttachment>(
        "SELECT * FROM media_attachments WHERE status_id IN (?)"
    )
    .bind(status_ids)
    .fetch_all(&self.pool)
    .await?;
    
    // グループ化
    let mut map = HashMap::new();
    for m in media {
        map.entry(m.status_id.clone().unwrap_or_default())
            .or_insert_with(Vec::new)
            .push(m);
    }
    
    Ok(map)
}
```

## 🟢 良好な実装

### 1. **適切なページネーション**

```rust
pub async fn get_local_statuses(
    &self,
    limit: usize,
    max_id: Option<&str>,
) -> Result<Vec<Status>, AppError> {
    // LIMIT句を使用した効率的なページネーション
    // ...
}
```

### 2. **インデックスの活用**

```sql
-- 外部キーインデックス
CREATE INDEX IF NOT EXISTS idx_media_attachments_status_id 
ON media_attachments(status_id);

-- 検索用インデックス
CREATE INDEX IF NOT EXISTS idx_statuses_uri ON statuses(uri);
```

### 3. **FTS5の使用**

```sql
-- 全文検索の効率的な実装
CREATE VIRTUAL TABLE statuses_fts USING fts5(
    status_id UNINDEXED,
    content
);
```

## 📋 推奨アクション

### 優先度: 高

1. **通知取得のN+1問題を修正**
   - `get_statuses_by_uris()` メソッドを追加
   - 通知取得ロジックを一括取得に変更

2. **ブックマーク/お気に入り取得の最適化**
   - JOINを使用した一括取得メソッドを追加
   - `get_bookmarked_statuses()` メソッドを実装
   - `get_favourited_statuses()` メソッドを実装

### 優先度: 中

3. **複合インデックスの追加**
   - `idx_statuses_account_created`
   - `idx_statuses_local_created`
   - `idx_notifications_read_created`

4. **バッチ取得メソッドの追加**
   - `get_media_by_status_ids()`
   - `get_accounts_by_addresses()` (将来的に)

### 優先度: 低

5. **クエリの最適化**
   - SELECT文で必要なカラムのみ取得
   - 不要なデータの転送を削減

6. **キャッシュ戦略の検討**
   - 頻繁にアクセスされるデータのキャッシュ
   - アカウント情報のキャッシュ

## 🔧 実装例

### 1. ステータス一括取得メソッド

```rust
/// Get multiple statuses by URIs (batch operation)
pub async fn get_statuses_by_uris(
    &self,
    uris: &[&str],
) -> Result<Vec<Status>, AppError> {
    if uris.is_empty() {
        return Ok(vec![]);
    }
    
    // SQLiteのIN句には制限があるため、チャンク化
    let mut all_statuses = Vec::new();
    
    for chunk in uris.chunks(100) {
        let placeholders = chunk.iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        
        let query = format!(
            "SELECT * FROM statuses WHERE uri IN ({})",
            placeholders
        );
        
        let mut query_builder = sqlx::query_as::<_, Status>(&query);
        for uri in chunk {
            query_builder = query_builder.bind(uri);
        }
        
        let statuses = query_builder.fetch_all(&self.pool).await?;
        all_statuses.extend(statuses);
    }
    
    Ok(all_statuses)
}
```

### 2. ブックマーク一括取得メソッド

```rust
/// Get bookmarked statuses with JOIN (optimized)
pub async fn get_bookmarked_statuses(
    &self,
    limit: usize,
    max_id: Option<&str>,
) -> Result<Vec<Status>, AppError> {
    let statuses = match max_id {
        Some(max_id) => {
            sqlx::query_as::<_, Status>(
                r#"
                SELECT s.* FROM statuses s
                INNER JOIN bookmarks b ON s.id = b.status_id
                WHERE b.id < ?
                ORDER BY b.created_at DESC
                LIMIT ?
                "#
            )
            .bind(max_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, Status>(
                r#"
                SELECT s.* FROM statuses s
                INNER JOIN bookmarks b ON s.id = b.status_id
                ORDER BY b.created_at DESC
                LIMIT ?
                "#
            )
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        }
    };
    
    Ok(statuses)
}
```

### 3. 複合インデックスの追加

```sql
-- migrations/schema.sql に追加

-- アカウント別ステータス取得の最適化
CREATE INDEX IF NOT EXISTS idx_statuses_account_created 
ON statuses(account_address, created_at DESC);

-- ローカルステータス取得の最適化
CREATE INDEX IF NOT EXISTS idx_statuses_local_created 
ON statuses(is_local, created_at DESC) 
WHERE is_local = 1;

-- 通知取得の最適化
CREATE INDEX IF NOT EXISTS idx_notifications_read_created 
ON notifications(read, created_at DESC);

-- ブックマーク/お気に入り取得の最適化
CREATE INDEX IF NOT EXISTS idx_bookmarks_created 
ON bookmarks(created_at DESC);

CREATE INDEX IF NOT EXISTS idx_favourites_created 
ON favourites(created_at DESC);
```

## 📊 パフォーマンス改善予測

### 通知取得の改善

**改善前**:
- 20件の通知: 21回のクエリ
- 推定時間: ~200ms (10ms/query × 20)

**改善後**:
- 20件の通知: 2回のクエリ（通知取得 + ステータス一括取得）
- 推定時間: ~20ms
- **改善率: 90%削減**

### ブックマーク取得の改善

**改善前**:
- 20件のブックマーク: 21回のクエリ
- 推定時間: ~200ms

**改善後**:
- 20件のブックマーク: 1回のクエリ（JOIN）
- 推定時間: ~10ms
- **改善率: 95%削減**

## ✅ まとめ

### 発見された問題
- ✅ N+1問題: 通知取得（重大）
- ✅ 潜在的N+1問題: ブックマーク/お気に入り（中）
- ✅ インデックス最適化の余地（低）

### 推奨アクション
1. **即座に修正**: 通知取得のN+1問題
2. **短期的に実装**: ブックマーク/お気に入りの最適化
3. **中期的に実装**: 複合インデックスの追加
4. **長期的に検討**: キャッシュ戦略

### 全体評価
- **現状**: 基本的な実装は良好だが、いくつかのN+1問題が存在
- **改善後**: 大幅なパフォーマンス向上が期待できる
- **優先度**: 高（ユーザー体験に直接影響）

---

**分析者**: Antigravity AI  
**日時**: 2026-01-11 23:29  
**ステータス**: 分析完了、修正推奨
