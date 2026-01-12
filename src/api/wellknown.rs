//! Well-known endpoints
//!
//! - /.well-known/webfinger
//! - /.well-known/nodeinfo
//! - /.well-known/host-meta

use axum::{
    Router,
    extract::{Query, State},
    response::Json,
    routing::get,
};
use serde::Deserialize;

use crate::AppState;
use crate::error::AppError;

/// Create well-known router
///
/// Routes:
/// - GET /.well-known/webfinger
/// - GET /.well-known/nodeinfo
/// - GET /.well-known/host-meta
/// - GET /nodeinfo/2.0
pub fn wellknown_router() -> Router<AppState> {
    Router::new()
        .route("/.well-known/webfinger", get(webfinger))
        .route("/.well-known/nodeinfo", get(nodeinfo_links))
        .route("/.well-known/host-meta", get(host_meta))
        .route("/nodeinfo/2.0", get(nodeinfo))
}

/// WebFinger query parameters
#[derive(Debug, Deserialize)]
struct WebFingerQuery {
    resource: String,
}

/// GET /.well-known/webfinger
///
/// Responds to WebFinger queries for local accounts.
///
/// Query: ?resource=acct:user@domain
async fn webfinger(
    State(state): State<AppState>,
    Query(query): Query<WebFingerQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Parse resource (acct:username@domain)
    let resource = &query.resource;

    if !resource.starts_with("acct:") {
        return Err(AppError::Validation(
            "Resource must start with 'acct:'".to_string(),
        ));
    }

    let acct = &resource[5..]; // Remove "acct:" prefix
    let parts: Vec<&str> = acct.split('@').collect();

    if parts.len() != 2 {
        return Err(AppError::Validation("Invalid acct format".to_string()));
    }

    let username = parts[0];
    let domain = parts[1];

    // Verify domain matches local domain
    if domain != state.config.server.domain {
        return Err(AppError::NotFound);
    }

    // Get account from database
    let account = state.db.get_account().await?;

    match account {
        Some(acc) if acc.username == username => {
            // Build WebFinger response (JRD)
            let base_url = state.config.server.base_url();
            let actor_url = format!("{}/users/{}", base_url, username);

            Ok(Json(serde_json::json!({
                "subject": resource,
                "aliases": [actor_url.clone()],
                "links": [
                    {
                        "rel": "self",
                        "type": "application/activity+json",
                        "href": actor_url
                    },
                    {
                        "rel": "http://webfinger.net/rel/profile-page",
                        "type": "text/html",
                        "href": actor_url
                    }
                ]
            })))
        }
        _ => Err(AppError::NotFound),
    }
}

/// GET /.well-known/nodeinfo
///
/// Returns links to nodeinfo documents.
async fn nodeinfo_links(State(state): State<AppState>) -> Json<serde_json::Value> {
    let base_url = state.config.server.base_url();
    Json(serde_json::json!({
        "links": [
            {
                "rel": "http://nodeinfo.diaspora.software/ns/schema/2.0",
                "href": format!("{}/nodeinfo/2.0", base_url)
            }
        ]
    }))
}

/// GET /nodeinfo/2.0
///
/// Returns NodeInfo 2.0 document.
async fn nodeinfo(State(_state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": "2.0",
        "software": {
            "name": "rustresort",
            "version": env!("CARGO_PKG_VERSION")
        },
        "protocols": ["activitypub"],
        "services": {
            "inbound": [],
            "outbound": []
        },
        "openRegistrations": false,
        "usage": {
            "users": {
                "total": 1
            },
            "localPosts": 0
        },
        "metadata": {}
    }))
}

/// GET /.well-known/host-meta
///
/// Returns host-meta XML for WebFinger discovery.
async fn host_meta(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let base_url = state.config.server.base_url();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<XRD xmlns="http://docs.oasis-open.org/ns/xri/xrd-1.0">
  <Link rel="lrdd" template="{}/.well-known/webfinger?resource={{uri}}"/>
</XRD>"#,
        base_url
    );

    ([("Content-Type", "application/xrd+xml")], xml)
}
