//! Example of how to instrument code with Prometheus metrics
//!
//! This file demonstrates best practices for adding metrics to your handlers.

use axum::{Json, http::StatusCode, response::IntoResponse};
use rustresort::metrics::{
    ACTIVITYPUB_ACTIVITIES_RECEIVED, ACTIVITYPUB_ACTIVITIES_SENT, BACKUPS_TOTAL, CACHE_HITS_TOTAL,
    CACHE_MISSES_TOTAL, DB_QUERIES_TOTAL, DB_QUERY_DURATION_SECONDS, FEDERATION_REQUESTS_TOTAL,
    FOLLOWERS_TOTAL, FOLLOWING_TOTAL, HTTP_REQUEST_DURATION_SECONDS, HTTP_REQUESTS_TOTAL,
    MEDIA_BYTES_UPLOADED, MEDIA_UPLOADS_TOTAL, POSTS_TOTAL, USERS_TOTAL,
};
use serde_json::json;

/// Example handler showing metrics instrumentation
///
/// This demonstrates:
/// 1. Recording HTTP request metrics
/// 2. Timing request duration
/// 3. Recording database query metrics
/// 4. Updating gauge metrics
pub async fn example_handler_with_metrics() -> impl IntoResponse {
    // Start timing the request
    let timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["GET", "/example"])
        .start_timer();

    // Simulate some work
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Record the request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/example", "200"])
        .inc();

    // Stop the timer (automatically records the duration)
    timer.observe_duration();

    (StatusCode::OK, Json(json!({"status": "ok"})))
}

/// Example of database query instrumentation
pub async fn example_db_query_with_metrics() {
    // Start timing the query
    let timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "users"])
        .start_timer();

    // Simulate database query
    tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;

    // Record the query
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "users"])
        .inc();

    // Stop the timer
    timer.observe_duration();
}

/// Example of cache instrumentation
pub async fn example_cache_with_metrics(cache_name: &str, hit: bool) {
    if hit {
        CACHE_HITS_TOTAL.with_label_values(&[cache_name]).inc();
    } else {
        CACHE_MISSES_TOTAL.with_label_values(&[cache_name]).inc();
    }
}

/// Example of federation activity instrumentation
pub async fn example_federation_activity_with_metrics(activity_type: &str, direction: &str) {
    match direction {
        "inbound" => {
            ACTIVITYPUB_ACTIVITIES_RECEIVED
                .with_label_values(&[activity_type])
                .inc();
        }
        "outbound" => {
            ACTIVITYPUB_ACTIVITIES_SENT
                .with_label_values(&[activity_type])
                .inc();
        }
        _ => {}
    }

    // Also record the request
    FEDERATION_REQUESTS_TOTAL
        .with_label_values(&[direction, "success"])
        .inc();
}

/// Example of media upload instrumentation
pub async fn example_media_upload_with_metrics(bytes: u64) {
    MEDIA_UPLOADS_TOTAL.inc();
    MEDIA_BYTES_UPLOADED.inc_by(bytes as f64);
}

/// Example of backup instrumentation
pub async fn example_backup_with_metrics(success: bool) {
    let status = if success { "success" } else { "failure" };
    BACKUPS_TOTAL.with_label_values(&[status]).inc();
}

/// Example of updating application stats
pub async fn update_app_stats(users: i64, posts: i64, followers: i64, following: i64) {
    USERS_TOTAL.set(users);
    POSTS_TOTAL.set(posts);
    FOLLOWERS_TOTAL.set(followers);
    FOLLOWING_TOTAL.set(following);
}

/// Example middleware for automatic HTTP metrics
///
/// This can be used as an Axum middleware to automatically record metrics for all requests
pub async fn metrics_middleware(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
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

#[tokio::main]
async fn main() {
    rustresort::metrics::init_metrics();

    example_db_query_with_metrics().await;
    example_cache_with_metrics("timeline", true).await;
    example_federation_activity_with_metrics("Create", "inbound").await;
    example_media_upload_with_metrics(1024).await;
    example_backup_with_metrics(true).await;
    update_app_stats(1, 1, 0, 0).await;

    let _ = example_handler_with_metrics().await;
}
