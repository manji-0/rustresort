# Advanced Metrics Implementation Summary

## 概要

RustResortに以下の高度なメトリクス機能を実装しました：

1. **追加のエンドポイントへのメトリクス実装**
2. **キャッシュメトリクス**
3. **エラーメトリクス**

## 1. 追加のエンドポイントへのメトリクス実装

### Timelines (`src/api/mastodon/timelines.rs`)

#### `home_timeline` (GET /api/v1/timelines/home)
- **HTTPメトリクス**: リクエスト数、処理時間
- **DBメトリクス**:
  - アカウント取得 (SELECT accounts)
  - ステータス取得 (SELECT statuses)

#### `public_timeline` (GET /api/v1/timelines/public)
- **HTTPメトリクス**: リクエスト数、処理時間
- **DBメトリクス**:
  - アカウント取得 (SELECT accounts)
  - ステータス取得 (SELECT statuses)

## 2. キャッシュメトリクス

### Timeline Cache (`src/data/cache.rs`)

#### `TimelineCache::get`
- **キャッシュヒット/ミス**: `rustresort_cache_hits_total{cache_name="timeline"}` / `rustresort_cache_misses_total{cache_name="timeline"}`
- キャッシュにステータスが存在する場合はヒット、存在しない場合はミスとして記録

#### `TimelineCache::insert`
- **キャッシュサイズ**: `rustresort_cache_size{cache_name="timeline"}`
- 新しいステータスを挿入するたびにキャッシュサイズを更新

### Profile Cache (`src/data/cache.rs`)

#### `ProfileCache::get`
- **キャッシュヒット/ミス**: `rustresort_cache_hits_total{cache_name="profile"}` / `rustresort_cache_misses_total{cache_name="profile"}`
- プロフィールキャッシュのヒット/ミスを追跡

#### `ProfileCache::insert`
- **キャッシュサイズ**: `rustresort_cache_size{cache_name="profile"}`
- プロフィールキャッシュのサイズを追跡

### キャッシュメトリクスの使用例

```promql
# キャッシュヒット率
sum(rate(rustresort_cache_hits_total[5m])) / 
(sum(rate(rustresort_cache_hits_total[5m])) + sum(rate(rustresort_cache_misses_total[5m])))

# タイムラインキャッシュのヒット率
sum(rate(rustresort_cache_hits_total{cache_name="timeline"}[5m])) / 
(sum(rate(rustresort_cache_hits_total{cache_name="timeline"}[5m])) + 
 sum(rate(rustresort_cache_misses_total{cache_name="timeline"}[5m])))

# キャッシュサイズの推移
rustresort_cache_size{cache_name="timeline"}
rustresort_cache_size{cache_name="profile"}
```

## 3. エラーメトリクス

### Error Tracking (`src/error.rs`)

すべてのエラーが`AppError::into_response`で自動的に記録されます。

#### エラータイプ

- `not_found` - リソースが見つからない (404)
- `unauthorized` - 認証が必要 (401)
- `invalid_signature` - 署名検証失敗 (401)
- `forbidden` - アクセス拒否 (403)
- `validation` - バリデーションエラー (400)
- `rate_limited` - レート制限超過 (429)
- `federation` - Federation エラー (502)
- `http_client` - HTTPクライアントエラー (502)
- `database` - データベースエラー (500)
- `storage` - ストレージエラー (500)
- `config` - 設定エラー (500)
- `encryption` - 暗号化エラー (500)
- `internal` - 内部サーバーエラー (500)

#### メトリクス

- **エラー総数**: `rustresort_errors_total{error_type, endpoint}`
  - `error_type`: エラーの種類
  - `endpoint`: エラーが発生したエンドポイント（現在は"unknown"）

### エラーメトリクスの使用例

```promql
# エラーレート（全体）
rate(rustresort_errors_total[5m])

# エラータイプ別のレート
sum by (error_type) (rate(rustresort_errors_total[5m]))

# 最も多いエラータイプ
topk(5, sum by (error_type) (rate(rustresort_errors_total[5m])))

# データベースエラーの割合
sum(rate(rustresort_errors_total{error_type="database"}[5m])) / 
sum(rate(rustresort_errors_total[5m]))

# 4xx vs 5xx エラー
sum(rate(rustresort_errors_total{error_type=~"not_found|unauthorized|forbidden|validation|rate_limited"}[5m])) # 4xx
sum(rate(rustresort_errors_total{error_type=~"database|storage|config|encryption|internal|federation|http_client"}[5m])) # 5xx
```

## 新しいメトリクス一覧

### キャッシュメトリクス

| メトリクス名 | タイプ | ラベル | 説明 |
|------------|--------|--------|------|
| `rustresort_cache_hits_total` | Counter | `cache_name` | キャッシュヒット総数 |
| `rustresort_cache_misses_total` | Counter | `cache_name` | キャッシュミス総数 |
| `rustresort_cache_size` | Gauge | `cache_name` | キャッシュ内のアイテム数 |

**cache_name の値:**
- `timeline` - タイムラインキャッシュ
- `profile` - プロフィールキャッシュ

### エラーメトリクス

| メトリクス名 | タイプ | ラベル | 説明 |
|------------|--------|--------|------|
| `rustresort_errors_total` | Counter | `error_type`, `endpoint` | エラー総数 |

**error_type の値:**
- `not_found`, `unauthorized`, `invalid_signature`, `forbidden`, `validation`, `rate_limited`, `federation`, `http_client`, `database`, `storage`, `config`, `encryption`, `internal`

## アーキテクチャ上の変更

### モジュールの可視性

- `src/api/metrics.rs` を `pub mod` に変更
  - 他のモジュール（`data`, `error`）からメトリクスにアクセスできるようにするため

### 自動メトリクス収集

#### キャッシュ
- キャッシュの`get`と`insert`操作で自動的にメトリクスを記録
- パフォーマンスへの影響は最小限（カウンターのインクリメントのみ）

#### エラー
- すべてのエラーが`into_response`を通過するため、自動的に記録される
- エラーハンドリングのコードを変更する必要なし

## パフォーマンスへの影響

### キャッシュメトリクス
- **オーバーヘッド**: 非常に低い
  - カウンターのインクリメント: ~10ns
  - ゲージの更新: ~10ns
- **メモリ**: ラベルごとに約100バイト

### エラーメトリクス
- **オーバーヘッド**: 低い
  - エラー発生時のみ実行される
  - カウンターのインクリメント: ~10ns
- **メモリ**: エラータイプごとに約100バイト

## 監視とアラート

### 推奨アラート

#### キャッシュヒット率が低い
```yaml
- alert: LowCacheHitRate
  expr: |
    sum(rate(rustresort_cache_hits_total[5m])) / 
    (sum(rate(rustresort_cache_hits_total[5m])) + sum(rate(rustresort_cache_misses_total[5m]))) < 0.5
  for: 10m
  annotations:
    summary: "Cache hit rate is below 50%"
```

#### エラーレートが高い
```yaml
- alert: HighErrorRate
  expr: rate(rustresort_errors_total[5m]) > 1
  for: 5m
  annotations:
    summary: "Error rate is above 1 per second"
```

#### データベースエラーが多い
```yaml
- alert: HighDatabaseErrorRate
  expr: rate(rustresort_errors_total{error_type="database"}[5m]) > 0.1
  for: 5m
  annotations:
    summary: "Database error rate is above 0.1 per second"
```

## Grafanaダッシュボード例

### キャッシュパネル

```json
{
  "title": "Cache Performance",
  "targets": [
    {
      "expr": "sum(rate(rustresort_cache_hits_total[5m])) / (sum(rate(rustresort_cache_hits_total[5m])) + sum(rate(rustresort_cache_misses_total[5m])))",
      "legendFormat": "Hit Rate"
    },
    {
      "expr": "rustresort_cache_size",
      "legendFormat": "{{cache_name}} Size"
    }
  ]
}
```

### エラーパネル

```json
{
  "title": "Error Rate by Type",
  "targets": [
    {
      "expr": "sum by (error_type) (rate(rustresort_errors_total[5m]))",
      "legendFormat": "{{error_type}}"
    }
  ]
}
```

## 今後の改善案

### 1. エンドポイント別エラー追跡

現在、エラーメトリクスの`endpoint`ラベルは"unknown"です。これを実際のエンドポイントに変更するには：

```rust
// エラーコンテキストを追加
pub struct ErrorContext {
    pub endpoint: String,
}

// エラーに追加
impl AppError {
    pub fn with_context(self, endpoint: &str) -> Self {
        // エンドポイント情報を保持
    }
}
```

### 2. キャッシュ有効期限メトリクス

キャッシュアイテムの平均有効期限を追跡：

```rust
pub static ref CACHE_TTL_SECONDS: HistogramVec = HistogramVec::new(
    HistogramOpts::new("rustresort_cache_ttl_seconds", "Cache item TTL"),
    &["cache_name"]
).expect("metric can be created");
```

### 3. キャッシュ退避メトリクス

LRU退避の頻度を追跡：

```rust
pub static ref CACHE_EVICTIONS_TOTAL: IntCounterVec = IntCounterVec::new(
    Opts::new("rustresort_cache_evictions_total", "Cache evictions"),
    &["cache_name", "reason"]
).expect("metric can be created");
```

## まとめ

この実装により、以下が可能になりました：

✅ **キャッシュパフォーマンスの可視化**
- ヒット率の監視
- キャッシュサイズの追跡
- パフォーマンスボトルネックの特定

✅ **エラーパターンの分析**
- エラータイプ別の集計
- エラーレートの監視
- 問題の早期発見

✅ **自動メトリクス収集**
- コード変更不要
- 低オーバーヘッド
- 包括的なカバレッジ

これらのメトリクスにより、アプリケーションの健全性を継続的に監視し、問題を早期に発見・対処できるようになりました。
