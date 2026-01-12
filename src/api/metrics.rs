//! Prometheus metrics endpoint
//!
//! Exposes application metrics in Prometheus format for monitoring and observability.

use axum::{
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use lazy_static::lazy_static;
use prometheus::{
    Counter, Gauge, HistogramOpts, IntCounter, IntCounterVec, IntGauge, IntGaugeVec,
    Opts, Registry, TextEncoder, Encoder,
};

lazy_static! {
    /// Global Prometheus registry
    pub static ref REGISTRY: Registry = Registry::new();

    // HTTP Metrics
    /// Total number of HTTP requests received
    pub static ref HTTP_REQUESTS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("rustresort_http_requests_total", "Total number of HTTP requests"),
        &["method", "endpoint", "status"]
    ).expect("metric can be created");

    /// HTTP request duration in seconds
    pub static ref HTTP_REQUEST_DURATION_SECONDS: prometheus::HistogramVec = prometheus::HistogramVec::new(
        HistogramOpts::new(
            "rustresort_http_request_duration_seconds",
            "HTTP request duration in seconds"
        ).buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
        &["method", "endpoint"]
    ).expect("metric can be created");

    // Database Metrics
    /// Total number of database queries
    pub static ref DB_QUERIES_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("rustresort_db_queries_total", "Total number of database queries"),
        &["operation", "table"]
    ).expect("metric can be created");

    /// Database query duration in seconds
    pub static ref DB_QUERY_DURATION_SECONDS: prometheus::HistogramVec = prometheus::HistogramVec::new(
        HistogramOpts::new(
            "rustresort_db_query_duration_seconds",
            "Database query duration in seconds"
        ).buckets(vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0]),
        &["operation", "table"]
    ).expect("metric can be created");

    /// Current number of active database connections
    pub static ref DB_CONNECTIONS_ACTIVE: IntGauge = IntGauge::new(
        "rustresort_db_connections_active",
        "Current number of active database connections"
    ).expect("metric can be created");

    // Federation Metrics
    /// Total number of ActivityPub activities received
    pub static ref ACTIVITYPUB_ACTIVITIES_RECEIVED: IntCounterVec = IntCounterVec::new(
        Opts::new("rustresort_activitypub_activities_received_total", "Total number of ActivityPub activities received"),
        &["activity_type"]
    ).expect("metric can be created");

    /// Total number of ActivityPub activities sent
    pub static ref ACTIVITYPUB_ACTIVITIES_SENT: IntCounterVec = IntCounterVec::new(
        Opts::new("rustresort_activitypub_activities_sent_total", "Total number of ActivityPub activities sent"),
        &["activity_type"]
    ).expect("metric can be created");

    /// Total number of federation requests
    pub static ref FEDERATION_REQUESTS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("rustresort_federation_requests_total", "Total number of federation requests"),
        &["direction", "status"]
    ).expect("metric can be created");

    /// Federation request duration in seconds
    pub static ref FEDERATION_REQUEST_DURATION_SECONDS: prometheus::HistogramVec = prometheus::HistogramVec::new(
        HistogramOpts::new(
            "rustresort_federation_request_duration_seconds",
            "Federation request duration in seconds"
        ).buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0]),
        &["direction"]
    ).expect("metric can be created");

    // Cache Metrics
    /// Total number of cache hits
    pub static ref CACHE_HITS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("rustresort_cache_hits_total", "Total number of cache hits"),
        &["cache_name"]
    ).expect("metric can be created");

    /// Total number of cache misses
    pub static ref CACHE_MISSES_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("rustresort_cache_misses_total", "Total number of cache misses"),
        &["cache_name"]
    ).expect("metric can be created");

    /// Current number of items in cache
    pub static ref CACHE_SIZE: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rustresort_cache_size", "Current number of items in cache"),
        &["cache_name"]
    ).expect("metric can be created");

    // Storage Metrics
    /// Total number of media uploads
    pub static ref MEDIA_UPLOADS_TOTAL: IntCounter = IntCounter::new(
        "rustresort_media_uploads_total",
        "Total number of media uploads"
    ).expect("metric can be created");

    /// Total bytes of media uploaded
    pub static ref MEDIA_BYTES_UPLOADED: Counter = Counter::new(
        "rustresort_media_bytes_uploaded_total",
        "Total bytes of media uploaded"
    ).expect("metric can be created");

    /// Total number of backups created
    pub static ref BACKUPS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("rustresort_backups_total", "Total number of backups created"),
        &["status"]
    ).expect("metric can be created");

    // Application Metrics
    /// Application uptime in seconds
    pub static ref APP_UPTIME_SECONDS: Gauge = Gauge::new(
        "rustresort_app_uptime_seconds",
        "Application uptime in seconds"
    ).expect("metric can be created");

    /// Total number of registered users
    pub static ref USERS_TOTAL: IntGauge = IntGauge::new(
        "rustresort_users_total",
        "Total number of registered users"
    ).expect("metric can be created");

    /// Total number of posts
    pub static ref POSTS_TOTAL: IntGauge = IntGauge::new(
        "rustresort_posts_total",
        "Total number of posts"
    ).expect("metric can be created");

    /// Total number of followers
    pub static ref FOLLOWERS_TOTAL: IntGauge = IntGauge::new(
        "rustresort_followers_total",
        "Total number of followers"
    ).expect("metric can be created");

    /// Total number of following
    pub static ref FOLLOWING_TOTAL: IntGauge = IntGauge::new(
        "rustresort_following_total",
        "Total number of following"
    ).expect("metric can be created");

    // Error Metrics
    /// Total number of errors by type
    pub static ref ERRORS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("rustresort_errors_total", "Total number of errors"),
        &["error_type", "endpoint"]
    ).expect("metric can be created");
}

/// Initialize metrics registry
///
/// This should be called once at application startup to register all metrics.
pub fn init_metrics() {
    // Register HTTP metrics
    REGISTRY
        .register(Box::new(HTTP_REQUESTS_TOTAL.clone()))
        .expect("HTTP_REQUESTS_TOTAL can be registered");
    REGISTRY
        .register(Box::new(HTTP_REQUEST_DURATION_SECONDS.clone()))
        .expect("HTTP_REQUEST_DURATION_SECONDS can be registered");

    // Register database metrics
    REGISTRY
        .register(Box::new(DB_QUERIES_TOTAL.clone()))
        .expect("DB_QUERIES_TOTAL can be registered");
    REGISTRY
        .register(Box::new(DB_QUERY_DURATION_SECONDS.clone()))
        .expect("DB_QUERY_DURATION_SECONDS can be registered");
    REGISTRY
        .register(Box::new(DB_CONNECTIONS_ACTIVE.clone()))
        .expect("DB_CONNECTIONS_ACTIVE can be registered");

    // Register federation metrics
    REGISTRY
        .register(Box::new(ACTIVITYPUB_ACTIVITIES_RECEIVED.clone()))
        .expect("ACTIVITYPUB_ACTIVITIES_RECEIVED can be registered");
    REGISTRY
        .register(Box::new(ACTIVITYPUB_ACTIVITIES_SENT.clone()))
        .expect("ACTIVITYPUB_ACTIVITIES_SENT can be registered");
    REGISTRY
        .register(Box::new(FEDERATION_REQUESTS_TOTAL.clone()))
        .expect("FEDERATION_REQUESTS_TOTAL can be registered");
    REGISTRY
        .register(Box::new(FEDERATION_REQUEST_DURATION_SECONDS.clone()))
        .expect("FEDERATION_REQUEST_DURATION_SECONDS can be registered");

    // Register cache metrics
    REGISTRY
        .register(Box::new(CACHE_HITS_TOTAL.clone()))
        .expect("CACHE_HITS_TOTAL can be registered");
    REGISTRY
        .register(Box::new(CACHE_MISSES_TOTAL.clone()))
        .expect("CACHE_MISSES_TOTAL can be registered");
    REGISTRY
        .register(Box::new(CACHE_SIZE.clone()))
        .expect("CACHE_SIZE can be registered");

    // Register storage metrics
    REGISTRY
        .register(Box::new(MEDIA_UPLOADS_TOTAL.clone()))
        .expect("MEDIA_UPLOADS_TOTAL can be registered");
    REGISTRY
        .register(Box::new(MEDIA_BYTES_UPLOADED.clone()))
        .expect("MEDIA_BYTES_UPLOADED can be registered");
    REGISTRY
        .register(Box::new(BACKUPS_TOTAL.clone()))
        .expect("BACKUPS_TOTAL can be registered");

    // Register application metrics
    REGISTRY
        .register(Box::new(APP_UPTIME_SECONDS.clone()))
        .expect("APP_UPTIME_SECONDS can be registered");
    REGISTRY
        .register(Box::new(USERS_TOTAL.clone()))
        .expect("USERS_TOTAL can be registered");
    REGISTRY
        .register(Box::new(POSTS_TOTAL.clone()))
        .expect("POSTS_TOTAL can be registered");
    REGISTRY
        .register(Box::new(FOLLOWERS_TOTAL.clone()))
        .expect("FOLLOWERS_TOTAL can be registered");
    REGISTRY
        .register(Box::new(FOLLOWING_TOTAL.clone()))
        .expect("FOLLOWING_TOTAL can be registered");

    // Register error metrics
    REGISTRY
        .register(Box::new(ERRORS_TOTAL.clone()))
        .expect("ERRORS_TOTAL can be registered");

    tracing::info!("Metrics registry initialized");
}

/// Metrics endpoint handler
///
/// Returns all metrics in Prometheus text format
async fn metrics_handler() -> Response {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    
    match encoder.encode_to_string(&metric_families) {
        Ok(metrics_text) => {
            (
                axum::http::StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, encoder.format_type())],
                metrics_text,
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to encode metrics");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to encode metrics",
            )
                .into_response()
        }
    }
}

/// Create metrics router
///
/// Exposes the `/metrics` endpoint publicly
pub fn metrics_router() -> Router {
    Router::new().route("/metrics", get(metrics_handler))
}
