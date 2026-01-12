# Metrics Endpoint

RustResort exposes application metrics in Prometheus format via the `/metrics` endpoint.

## Endpoint

- **URL**: `/metrics`
- **Method**: `GET`
- **Authentication**: None (publicly accessible)
- **Content-Type**: `text/plain; version=0.0.4`

## Usage

### Accessing Metrics

Simply make a GET request to the `/metrics` endpoint:

```bash
curl http://localhost:3000/metrics
```

### Prometheus Configuration

Add the following to your `prometheus.yml` configuration:

```yaml
scrape_configs:
  - job_name: 'rustresort'
    static_configs:
      - targets: ['localhost:3000']
    metrics_path: '/metrics'
    scrape_interval: 15s
```

## Available Metrics

### HTTP Metrics

- **`rustresort_http_requests_total`** (Counter)
  - Total number of HTTP requests
  - Labels: `method`, `endpoint`, `status`

- **`rustresort_http_request_duration_seconds`** (Histogram)
  - HTTP request duration in seconds
  - Labels: `method`, `endpoint`
  - Buckets: 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0

### Database Metrics

- **`rustresort_db_queries_total`** (Counter)
  - Total number of database queries
  - Labels: `operation`, `table`

- **`rustresort_db_query_duration_seconds`** (Histogram)
  - Database query duration in seconds
  - Labels: `operation`, `table`
  - Buckets: 0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0

- **`rustresort_db_connections_active`** (Gauge)
  - Current number of active database connections

### Federation Metrics

- **`rustresort_activitypub_activities_received_total`** (Counter)
  - Total number of ActivityPub activities received
  - Labels: `activity_type`

- **`rustresort_activitypub_activities_sent_total`** (Counter)
  - Total number of ActivityPub activities sent
  - Labels: `activity_type`

- **`rustresort_federation_requests_total`** (Counter)
  - Total number of federation requests
  - Labels: `direction`, `status`

- **`rustresort_federation_request_duration_seconds`** (Histogram)
  - Federation request duration in seconds
  - Labels: `direction`
  - Buckets: 0.01, 0.05, 0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0

### Cache Metrics

- **`rustresort_cache_hits_total`** (Counter)
  - Total number of cache hits
  - Labels: `cache_name`

- **`rustresort_cache_misses_total`** (Counter)
  - Total number of cache misses
  - Labels: `cache_name`

- **`rustresort_cache_size`** (Gauge)
  - Current number of items in cache
  - Labels: `cache_name`

### Storage Metrics

- **`rustresort_media_uploads_total`** (Counter)
  - Total number of media uploads

- **`rustresort_media_bytes_uploaded_total`** (Counter)
  - Total bytes of media uploaded

- **`rustresort_backups_total`** (Counter)
  - Total number of backups created
  - Labels: `status`

### Application Metrics

- **`rustresort_app_uptime_seconds`** (Gauge)
  - Application uptime in seconds

- **`rustresort_users_total`** (Gauge)
  - Total number of registered users

- **`rustresort_posts_total`** (Gauge)
  - Total number of posts

- **`rustresort_followers_total`** (Gauge)
  - Total number of followers

- **`rustresort_following_total`** (Gauge)
  - Total number of following

## Example Queries

### PromQL Examples

```promql
# Request rate per endpoint
rate(rustresort_http_requests_total[5m])

# 95th percentile request duration
histogram_quantile(0.95, rate(rustresort_http_request_duration_seconds_bucket[5m]))

# Database query rate
rate(rustresort_db_queries_total[5m])

# Cache hit ratio
sum(rate(rustresort_cache_hits_total[5m])) / 
(sum(rate(rustresort_cache_hits_total[5m])) + sum(rate(rustresort_cache_misses_total[5m])))

# Federation activity rate
rate(rustresort_activitypub_activities_received_total[5m])
```

## Grafana Dashboard

You can create a Grafana dashboard to visualize these metrics. Here are some suggested panels:

1. **HTTP Request Rate**: Graph showing `rate(rustresort_http_requests_total[5m])`
2. **Request Duration**: Heatmap of `rustresort_http_request_duration_seconds`
3. **Database Performance**: Graph showing query rate and duration
4. **Federation Activity**: Graph showing ActivityPub activities sent/received
5. **Cache Performance**: Graph showing cache hit ratio
6. **Application Stats**: Single stat panels for users, posts, followers, following

## Integration with Monitoring Stack

### Docker Compose Example

```yaml
version: '3.8'

services:
  rustresort:
    image: rustresort:latest
    ports:
      - "3000:3000"
    
  prometheus:
    image: prom/prometheus:latest
    ports:
      - "9090:9090"
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
      - prometheus_data:/prometheus
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.path=/prometheus'
    
  grafana:
    image: grafana/grafana:latest
    ports:
      - "3001:3000"
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=admin
    volumes:
      - grafana_data:/var/lib/grafana
    depends_on:
      - prometheus

volumes:
  prometheus_data:
  grafana_data:
```

## Instrumenting Your Code

To add custom metrics to your code, use the exported metrics from the `rustresort::api::metrics` module:

```rust
use rustresort::api::metrics::{HTTP_REQUESTS_TOTAL, HTTP_REQUEST_DURATION_SECONDS};

// Increment a counter
HTTP_REQUESTS_TOTAL
    .with_label_values(&["GET", "/api/v1/statuses", "200"])
    .inc();

// Record a histogram value
let timer = HTTP_REQUEST_DURATION_SECONDS
    .with_label_values(&["GET", "/api/v1/statuses"])
    .start_timer();
// ... do work ...
timer.observe_duration();
```

## Security Considerations

The `/metrics` endpoint is publicly accessible by default. If you need to restrict access:

1. **Firewall Rules**: Configure your firewall to only allow Prometheus servers to access the metrics endpoint
2. **Reverse Proxy**: Use a reverse proxy (nginx, Caddy) to add authentication to the `/metrics` endpoint
3. **Network Isolation**: Run Prometheus in the same private network as RustResort

Example nginx configuration with basic auth:

```nginx
location /metrics {
    auth_basic "Metrics";
    auth_basic_user_file /etc/nginx/.htpasswd;
    proxy_pass http://rustresort:3000;
}
```
