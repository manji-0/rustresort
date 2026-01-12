# Metrics Implementation Summary

## 概要

RustResortの主要なAPIハンドラーにPrometheusメトリクスを実装しました。これにより、アプリケーションのパフォーマンス、使用状況、エラー率などをリアルタイムで監視できるようになりました。

## 実装されたハンドラー

### 1. **Mastodon API - Statuses** (`src/api/mastodon/statuses.rs`)

#### `create_status` (POST /api/v1/statuses)
- **HTTPメトリクス**: リクエスト数、処理時間
- **DBメトリクス**: 
  - アカウント取得 (SELECT accounts)
  - ステータス挿入 (INSERT statuses)
  - メディア取得 (SELECT media)
  - メディア添付 (INSERT media_attachments)
- **アプリケーションメトリクス**: 投稿総数 (POSTS_TOTAL) をインクリメント

#### `get_status` (GET /api/v1/statuses/:id)
- **HTTPメトリクス**: リクエスト数、処理時間
- **DBメトリクス**:
  - ステータス取得 (SELECT statuses)
  - アカウント取得 (SELECT accounts)

#### `delete_status` (DELETE /api/v1/statuses/:id)
- **HTTPメトリクス**: リクエスト数、処理時間
- **DBメトリクス**:
  - ステータス取得 (SELECT statuses)
  - アカウント取得 (SELECT accounts)
  - ステータス削除 (DELETE statuses)
- **アプリケーションメトリクス**: 投稿総数 (POSTS_TOTAL) をデクリメント

### 2. **Mastodon API - Accounts** (`src/api/mastodon/accounts.rs`)

#### `verify_credentials` (GET /api/v1/accounts/verify_credentials)
- **HTTPメトリクス**: リクエスト数、処理時間
- **DBメトリクス**:
  - アカウント取得 (SELECT accounts)
  - フォロワー取得 (SELECT followers)
  - フォロー中取得 (SELECT follows)
- **アプリケーションメトリクス**:
  - フォロワー総数 (FOLLOWERS_TOTAL) を更新
  - フォロー中総数 (FOLLOWING_TOTAL) を更新

### 3. **Mastodon API - Media** (`src/api/mastodon/media.rs`)

#### `upload_media` (POST /api/v1/media)
- **HTTPメトリクス**: リクエスト数、処理時間
- **DBメトリクス**:
  - メディア挿入 (INSERT media)
- **ストレージメトリクス**:
  - メディアアップロード総数 (MEDIA_UPLOADS_TOTAL) をインクリメント
  - アップロードバイト数 (MEDIA_BYTES_UPLOADED) を加算

### 4. **ActivityPub** (`src/api/activitypub.rs`)

#### `actor` (GET /users/:username)
- **HTTPメトリクス**: リクエスト数、処理時間

#### `inbox` (POST /users/:username/inbox)
- **HTTPメトリクス**: リクエスト数、処理時間
- **Federationメトリクス**:
  - リクエスト処理時間 (FEDERATION_REQUEST_DURATION_SECONDS)
  - リクエスト総数 (FEDERATION_REQUESTS_TOTAL) - 成功/失敗を区別
  - 受信アクティビティ (ACTIVITYPUB_ACTIVITIES_RECEIVED) - アクティビティタイプ別

## メトリクスの種類

### HTTPメトリクス
- `rustresort_http_requests_total{method, endpoint, status}` - HTTPリクエスト総数
- `rustresort_http_request_duration_seconds{method, endpoint}` - リクエスト処理時間

### データベースメトリクス
- `rustresort_db_queries_total{operation, table}` - データベースクエリ総数
- `rustresort_db_query_duration_seconds{operation, table}` - クエリ実行時間

### Federationメトリクス
- `rustresort_activitypub_activities_received_total{activity_type}` - 受信したActivityPubアクティビティ
- `rustresort_federation_requests_total{direction, status}` - Federationリクエスト総数
- `rustresort_federation_request_duration_seconds{direction}` - Federationリクエスト処理時間

### ストレージメトリクス
- `rustresort_media_uploads_total` - メディアアップロード総数
- `rustresort_media_bytes_uploaded_total` - アップロードされたバイト数

### アプリケーションメトリクス
- `rustresort_posts_total` - 投稿総数
- `rustresort_followers_total` - フォロワー総数
- `rustresort_following_total` - フォロー中総数

## 実装パターン

### 基本的な使用方法

```rust
use crate::api::metrics::{HTTP_REQUESTS_TOTAL, HTTP_REQUEST_DURATION_SECONDS};

async fn handler() -> Result<Response, Error> {
    // リクエスト処理時間の計測開始
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["GET", "/api/v1/endpoint"])
        .start_timer();

    // ... 処理 ...

    // 成功時のカウント
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/v1/endpoint", "200"])
        .inc();

    Ok(response)
}
```

### データベースクエリの計測

```rust
use crate::api::metrics::{DB_QUERIES_TOTAL, DB_QUERY_DURATION_SECONDS};

// クエリ実行時間の計測
let db_timer = DB_QUERY_DURATION_SECONDS
    .with_label_values(&["SELECT", "statuses"])
    .start_timer();
let result = state.db.get_status(&id).await?;
DB_QUERIES_TOTAL.with_label_values(&["SELECT", "statuses"]).inc();
db_timer.observe_duration();
```

### カウンターとゲージの更新

```rust
use crate::api::metrics::{POSTS_TOTAL, MEDIA_UPLOADS_TOTAL, MEDIA_BYTES_UPLOADED};

// カウンターのインクリメント
POSTS_TOTAL.inc();
MEDIA_UPLOADS_TOTAL.inc();

// カウンターのデクリメント
POSTS_TOTAL.dec();

// カウンターに値を加算
MEDIA_BYTES_UPLOADED.inc_by(file_size as f64);

// ゲージに値を設定
FOLLOWERS_TOTAL.set(followers_count as i64);
```

## メトリクスの確認方法

### ローカルでの確認

```bash
# メトリクスエンドポイントにアクセス
curl http://localhost:3000/metrics

# 特定のメトリクスをフィルタ
curl http://localhost:3000/metrics | grep rustresort_http_requests_total
```

### Prometheusでの確認

```yaml
# prometheus.yml
scrape_configs:
  - job_name: 'rustresort'
    static_configs:
      - targets: ['localhost:3000']
    metrics_path: '/metrics'
    scrape_interval: 10s
```

### 便利なPromQLクエリ

```promql
# リクエストレート（5分間の平均）
rate(rustresort_http_requests_total[5m])

# エンドポイント別のリクエストレート
sum by (endpoint) (rate(rustresort_http_requests_total[5m]))

# 95パーセンタイルのレスポンス時間
histogram_quantile(0.95, rate(rustresort_http_request_duration_seconds_bucket[5m]))

# データベースクエリレート
rate(rustresort_db_queries_total[5m])

# Federationアクティビティレート
rate(rustresort_activitypub_activities_received_total[5m])

# メディアアップロード速度（バイト/秒）
rate(rustresort_media_bytes_uploaded_total[5m])
```

## 今後の拡張

### 追加すべきメトリクス

1. **キャッシュメトリクス**
   - キャッシュヒット/ミス率
   - キャッシュサイズ

2. **エラーメトリクス**
   - エラータイプ別のカウント
   - エラーレート

3. **バックグラウンドタスクメトリクス**
   - バックアップ成功/失敗数
   - バックアップ処理時間

4. **その他のハンドラー**
   - Timelines
   - Notifications
   - Search
   - Instance情報

### ミドルウェアの実装

すべてのエンドポイントに自動的にメトリクスを追加するミドルウェアを実装することも検討できます：

```rust
// examples/metrics_instrumentation.rs に例があります
pub async fn metrics_middleware<B>(
    req: axum::http::Request<B>,
    next: axum::middleware::Next<B>,
) -> impl IntoResponse {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();

    let timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&[&method, &path])
        .start_timer();

    let response = next.run(req).await;

    let status = response.status().as_u16().to_string();
    HTTP_REQUESTS_TOTAL
        .with_label_values(&[&method, &path, &status])
        .inc();

    timer.observe_duration();

    response
}
```

## 参考資料

- [Prometheus公式ドキュメント](https://prometheus.io/docs/)
- [Prometheusベストプラクティス](https://prometheus.io/docs/practices/naming/)
- `docs/METRICS.md` - メトリクスエンドポイントの詳細ドキュメント
- `examples/metrics_instrumentation.rs` - メトリクスの使用例
- `examples/prometheus.yml` - Prometheus設定例
