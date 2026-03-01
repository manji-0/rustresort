//! Prometheus metrics registry and instruments.
//!
//! This module is framework-agnostic and can be used from any layer.

use prometheus::{
    Counter, Gauge, HistogramOpts, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts,
    Registry, core::Collector,
};
use std::sync::{LazyLock, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Global Prometheus registry
pub static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

fn register_metric<M>(metric: M, metric_name: &'static str) -> M
where
    M: Collector + Clone + 'static,
{
    REGISTRY
        .register(Box::new(metric.clone()))
        .unwrap_or_else(|error| panic!("failed to register metric {metric_name}: {error}"));
    metric
}

macro_rules! registered_metric {
    ($name:ident, $metric:expr) => {
        LazyLock::new(|| register_metric($metric, stringify!($name)))
    };
}

// HTTP Metrics
pub static HTTP_REQUESTS_TOTAL: LazyLock<IntCounterVec> = registered_metric!(
    HTTP_REQUESTS_TOTAL,
    IntCounterVec::new(
        Opts::new(
            "rustresort_http_requests_total",
            "Total number of HTTP requests",
        ),
        &["method", "endpoint", "status"],
    )
    .expect("metric can be created")
);
pub static HTTP_REQUEST_DURATION_SECONDS: LazyLock<prometheus::HistogramVec> = registered_metric!(
    HTTP_REQUEST_DURATION_SECONDS,
    prometheus::HistogramVec::new(
        HistogramOpts::new(
            "rustresort_http_request_duration_seconds",
            "HTTP request duration in seconds",
        )
        .buckets(vec![
            0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
        ]),
        &["method", "endpoint"],
    )
    .expect("metric can be created")
);

// Database Metrics
pub static DB_QUERIES_TOTAL: LazyLock<IntCounterVec> = registered_metric!(
    DB_QUERIES_TOTAL,
    IntCounterVec::new(
        Opts::new(
            "rustresort_db_queries_total",
            "Total number of database queries",
        ),
        &["operation", "table"],
    )
    .expect("metric can be created")
);
pub static DB_QUERY_DURATION_SECONDS: LazyLock<prometheus::HistogramVec> = registered_metric!(
    DB_QUERY_DURATION_SECONDS,
    prometheus::HistogramVec::new(
        HistogramOpts::new(
            "rustresort_db_query_duration_seconds",
            "Database query duration in seconds",
        )
        .buckets(vec![
            0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0,
        ]),
        &["operation", "table"],
    )
    .expect("metric can be created")
);
pub static DB_CONNECTIONS_ACTIVE: LazyLock<IntGauge> = registered_metric!(
    DB_CONNECTIONS_ACTIVE,
    IntGauge::new(
        "rustresort_db_connections_active",
        "Current number of active database connections",
    )
    .expect("metric can be created")
);
pub static DB_SYNC_TOTAL: LazyLock<IntCounterVec> = registered_metric!(
    DB_SYNC_TOTAL,
    IntCounterVec::new(
        Opts::new(
            "rustresort_db_sync_total",
            "Total number of database sync cycles",
        ),
        &["backend", "status"],
    )
    .expect("metric can be created")
);
pub static DB_SYNC_DURATION_SECONDS: LazyLock<prometheus::HistogramVec> = registered_metric!(
    DB_SYNC_DURATION_SECONDS,
    prometheus::HistogramVec::new(
        HistogramOpts::new(
            "rustresort_db_sync_duration_seconds",
            "Database sync cycle duration in seconds",
        )
        .buckets(vec![
            0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
        ]),
        &["backend", "status"],
    )
    .expect("metric can be created")
);
pub static DB_SYNC_LAST_SUCCESS_UNIX_SECONDS: LazyLock<Gauge> = registered_metric!(
    DB_SYNC_LAST_SUCCESS_UNIX_SECONDS,
    Gauge::new(
        "rustresort_db_sync_last_success_unix_seconds",
        "Unix timestamp of the most recent successful database sync",
    )
    .expect("metric can be created")
);

// Federation Metrics
pub static ACTIVITYPUB_ACTIVITIES_RECEIVED: LazyLock<IntCounterVec> = registered_metric!(
    ACTIVITYPUB_ACTIVITIES_RECEIVED,
    IntCounterVec::new(
        Opts::new(
            "rustresort_activitypub_activities_received_total",
            "Total number of ActivityPub activities received",
        ),
        &["activity_type"],
    )
    .expect("metric can be created")
);
pub static ACTIVITYPUB_ACTIVITIES_SENT: LazyLock<IntCounterVec> = registered_metric!(
    ACTIVITYPUB_ACTIVITIES_SENT,
    IntCounterVec::new(
        Opts::new(
            "rustresort_activitypub_activities_sent_total",
            "Total number of ActivityPub activities sent",
        ),
        &["activity_type"],
    )
    .expect("metric can be created")
);
pub static FEDERATION_REQUESTS_TOTAL: LazyLock<IntCounterVec> = registered_metric!(
    FEDERATION_REQUESTS_TOTAL,
    IntCounterVec::new(
        Opts::new(
            "rustresort_federation_requests_total",
            "Total number of federation requests",
        ),
        &["direction", "status"],
    )
    .expect("metric can be created")
);
pub static FEDERATION_REQUEST_DURATION_SECONDS: LazyLock<prometheus::HistogramVec> = registered_metric!(
    FEDERATION_REQUEST_DURATION_SECONDS,
    prometheus::HistogramVec::new(
        HistogramOpts::new(
            "rustresort_federation_request_duration_seconds",
            "Federation request duration in seconds",
        )
        .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0]),
        &["direction"],
    )
    .expect("metric can be created")
);

// Cache Metrics
pub static CACHE_HITS_TOTAL: LazyLock<IntCounterVec> = registered_metric!(
    CACHE_HITS_TOTAL,
    IntCounterVec::new(
        Opts::new("rustresort_cache_hits_total", "Total number of cache hits"),
        &["cache_name"],
    )
    .expect("metric can be created")
);
pub static CACHE_MISSES_TOTAL: LazyLock<IntCounterVec> = registered_metric!(
    CACHE_MISSES_TOTAL,
    IntCounterVec::new(
        Opts::new(
            "rustresort_cache_misses_total",
            "Total number of cache misses",
        ),
        &["cache_name"],
    )
    .expect("metric can be created")
);
pub static CACHE_SIZE: LazyLock<IntGaugeVec> = registered_metric!(
    CACHE_SIZE,
    IntGaugeVec::new(
        Opts::new("rustresort_cache_size", "Current number of items in cache"),
        &["cache_name"],
    )
    .expect("metric can be created")
);

// Storage Metrics
pub static MEDIA_UPLOADS_TOTAL: LazyLock<IntCounter> = registered_metric!(
    MEDIA_UPLOADS_TOTAL,
    IntCounter::new(
        "rustresort_media_uploads_total",
        "Total number of media uploads",
    )
    .expect("metric can be created")
);
pub static MEDIA_BYTES_UPLOADED: LazyLock<Counter> = registered_metric!(
    MEDIA_BYTES_UPLOADED,
    Counter::new(
        "rustresort_media_bytes_uploaded_total",
        "Total bytes of media uploaded",
    )
    .expect("metric can be created")
);
pub static BACKUPS_TOTAL: LazyLock<IntCounterVec> = registered_metric!(
    BACKUPS_TOTAL,
    IntCounterVec::new(
        Opts::new(
            "rustresort_backups_total",
            "Total number of backups created",
        ),
        &["status"],
    )
    .expect("metric can be created")
);

// Application Metrics
pub static APP_UPTIME_SECONDS: LazyLock<Gauge> = registered_metric!(
    APP_UPTIME_SECONDS,
    Gauge::new(
        "rustresort_app_uptime_seconds",
        "Application uptime in seconds",
    )
    .expect("metric can be created")
);
pub static USERS_TOTAL: LazyLock<IntGauge> = registered_metric!(
    USERS_TOTAL,
    IntGauge::new("rustresort_users_total", "Total number of registered users")
        .expect("metric can be created")
);
pub static POSTS_TOTAL: LazyLock<IntGauge> = registered_metric!(
    POSTS_TOTAL,
    IntGauge::new("rustresort_posts_total", "Total number of posts")
        .expect("metric can be created")
);
pub static FOLLOWERS_TOTAL: LazyLock<IntGauge> = registered_metric!(
    FOLLOWERS_TOTAL,
    IntGauge::new("rustresort_followers_total", "Total number of followers")
        .expect("metric can be created")
);
pub static FOLLOWING_TOTAL: LazyLock<IntGauge> = registered_metric!(
    FOLLOWING_TOTAL,
    IntGauge::new("rustresort_following_total", "Total number of following")
        .expect("metric can be created")
);

// Error Metrics
pub static ERRORS_TOTAL: LazyLock<IntCounterVec> = registered_metric!(
    ERRORS_TOTAL,
    IntCounterVec::new(
        Opts::new("rustresort_errors_total", "Total number of errors"),
        &["error_type", "endpoint"],
    )
    .expect("metric can be created")
);

static METRICS_INITIALIZED: OnceLock<()> = OnceLock::new();

/// Initialize metrics registry.
pub fn init_metrics() {
    METRICS_INITIALIZED.get_or_init(|| {
        macro_rules! initialize_metrics {
            ($($metric:ident),+ $(,)?) => {
                $(
                    LazyLock::force(&$metric);
                )+
            };
        }

        initialize_metrics!(
            HTTP_REQUESTS_TOTAL,
            HTTP_REQUEST_DURATION_SECONDS,
            DB_QUERIES_TOTAL,
            DB_QUERY_DURATION_SECONDS,
            DB_CONNECTIONS_ACTIVE,
            DB_SYNC_TOTAL,
            DB_SYNC_DURATION_SECONDS,
            DB_SYNC_LAST_SUCCESS_UNIX_SECONDS,
            ACTIVITYPUB_ACTIVITIES_RECEIVED,
            ACTIVITYPUB_ACTIVITIES_SENT,
            FEDERATION_REQUESTS_TOTAL,
            FEDERATION_REQUEST_DURATION_SECONDS,
            CACHE_HITS_TOTAL,
            CACHE_MISSES_TOTAL,
            CACHE_SIZE,
            MEDIA_UPLOADS_TOTAL,
            MEDIA_BYTES_UPLOADED,
            BACKUPS_TOTAL,
            APP_UPTIME_SECONDS,
            USERS_TOTAL,
            POSTS_TOTAL,
            FOLLOWERS_TOTAL,
            FOLLOWING_TOTAL,
            ERRORS_TOTAL,
        );

        tracing::info!("Metrics registry initialized");
    });
}

/// Record a database sync cycle result.
pub fn observe_db_sync(backend: &str, status: &str, duration: Duration) {
    DB_SYNC_TOTAL.with_label_values(&[backend, status]).inc();
    DB_SYNC_DURATION_SECONDS
        .with_label_values(&[backend, status])
        .observe(duration.as_secs_f64());

    if status == "success" || status == "duplicate" {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        DB_SYNC_LAST_SUCCESS_UNIX_SECONDS.set(now);
    }
}
