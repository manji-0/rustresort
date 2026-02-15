//! Prometheus metrics endpoint
//!
//! Exposes application metrics in Prometheus format for monitoring and observability.

use axum::{
    Router,
    response::{IntoResponse, Response},
    routing::get,
};
use prometheus::{Encoder, TextEncoder};

use crate::metrics::REGISTRY;

/// Metrics endpoint handler
///
/// Returns all metrics in Prometheus text format.
async fn metrics_handler() -> Response {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();

    match encoder.encode_to_string(&metric_families) {
        Ok(metrics_text) => (
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, encoder.format_type())],
            metrics_text,
        )
            .into_response(),
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
/// Exposes the `/metrics` endpoint publicly.
pub fn metrics_router() -> Router {
    Router::new().route("/metrics", get(metrics_handler))
}
