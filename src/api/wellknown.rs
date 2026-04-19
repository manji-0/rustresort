//! Well-known endpoints
//!
//! - /.well-known/webfinger
//! - /.well-known/nodeinfo
//! - /.well-known/host-meta
//! - /.well-known/oauth-authorization-server

use axum::{
    Router,
    extract::{FromRef, Query, State},
    response::Json,
    routing::get,
};
use serde::Deserialize;

use crate::WellKnownState;
use crate::error::AppError;

/// Create well-known router
///
/// Routes:
/// - GET /.well-known/webfinger
/// - GET /.well-known/nodeinfo
/// - GET /.well-known/host-meta
/// - GET /nodeinfo/2.0
pub fn wellknown_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    WellKnownState: FromRef<S>,
{
    Router::new()
        .route("/.well-known/webfinger", get(webfinger))
        .route("/.well-known/nodeinfo", get(nodeinfo_links))
        .route("/.well-known/host-meta", get(host_meta))
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_authorization_server),
        )
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
    State(state): State<WellKnownState>,
    Query(query): Query<WebFingerQuery>,
) -> Result<Json<crate::federation::WebFingerResponse>, AppError> {
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
            // Build WebFinger response (JRD) from shared federation helper.
            let response = crate::federation::generate_webfinger_response(
                &acc.username,
                &state.config.server.domain,
                &state.config.server.base_url(),
            );
            Ok(Json(response))
        }
        _ => Err(AppError::NotFound),
    }
}

/// GET /.well-known/nodeinfo
///
/// Returns links to nodeinfo documents.
async fn nodeinfo_links(State(state): State<WellKnownState>) -> Json<serde_json::Value> {
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
async fn nodeinfo(State(state): State<WellKnownState>) -> Json<serde_json::Value> {
    let local_posts = state.db.count_local_statuses().await.unwrap_or(0);
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
            "localPosts": local_posts
        },
        "metadata": {}
    }))
}

/// GET /.well-known/host-meta
///
/// Returns host-meta XML for WebFinger discovery.
async fn host_meta(State(state): State<WellKnownState>) -> impl axum::response::IntoResponse {
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

/// GET /.well-known/oauth-authorization-server
///
/// Returns OAuth 2 Authorization Server Metadata (RFC 8414).
async fn oauth_authorization_server(
    State(state): State<WellKnownState>,
) -> Json<serde_json::Value> {
    let base_url = state.config.server.base_url();
    Json(serde_json::json!({
        "issuer": format!("{}/", base_url.trim_end_matches('/')),
        "service_documentation": "https://docs.joinmastodon.org/",
        "authorization_endpoint": format!("{}/oauth/authorize", base_url),
        "token_endpoint": format!("{}/oauth/token", base_url),
        "app_registration_endpoint": format!("{}/api/v1/apps", base_url),
        "revocation_endpoint": format!("{}/oauth/revoke", base_url),
        "scopes_supported": [
            "read",
            "write",
            "write:accounts",
            "write:blocks",
            "write:bookmarks",
            "write:favourites",
            "write:filters",
            "write:follows",
            "write:lists",
            "write:media",
            "write:mutes",
            "write:notifications",
            "write:reports",
            "write:statuses",
            "read:accounts",
            "read:blocks",
            "read:bookmarks",
            "read:favourites",
            "read:filters",
            "read:follows",
            "read:lists",
            "read:mutes",
            "read:notifications",
            "read:search",
            "read:statuses",
            "follow",
            "push",
            "profile",
            "admin:read",
            "admin:read:accounts",
            "admin:read:reports",
            "admin:read:domain_blocks",
            "admin:write",
            "admin:write:accounts",
            "admin:write:reports",
            "admin:write:domain_blocks"
        ],
        "response_types_supported": ["code"],
        "response_modes_supported": ["query", "fragment", "form_post"],
        "code_challenge_methods_supported": ["S256"],
        "grant_types_supported": ["authorization_code", "client_credentials", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"]
    }))
}
