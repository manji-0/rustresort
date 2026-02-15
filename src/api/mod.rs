//! API layer
//!
//! HTTP handlers for:
//! - Mastodon API (for client apps)
//! - ActivityPub (for federation)
//! - Admin API
//! - Metrics (Prometheus)

mod activitypub;
mod admin;
mod converters;
mod dto;
mod mastodon;
pub mod metrics;
mod oauth;
mod wellknown;

pub use converters::*;
pub use dto::*;

pub use activitypub::activitypub_router;
pub use admin::admin_router;
pub use mastodon::mastodon_api_router;
pub use metrics::metrics_router;
pub use oauth::oauth_router;
pub use wellknown::wellknown_router;
