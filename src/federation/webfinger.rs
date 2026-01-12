//! WebFinger protocol implementation
//!
//! Used to discover ActivityPub actor URIs from addresses.

use serde::Deserialize;

use crate::error::AppError;

/// WebFinger result
#[derive(Debug, Clone)]
pub struct WebFingerResult {
    /// Subject (acct:user@domain)
    pub subject: String,
    /// ActivityPub actor URI
    pub actor_uri: String,
    /// Profile page URL (optional)
    pub profile_url: Option<String>,
}

/// Resolve an address to ActivityPub actor
///
/// # Arguments
/// * `address` - Account address (user@domain)
/// * `http_client` - HTTP client
///
/// # Returns
/// WebFinger result with actor URI
///
/// # Example
/// ```ignore
/// let result = resolve_webfinger("user@mastodon.social", &client).await?;
/// println!("Actor: {}", result.actor_uri);
/// ```
pub async fn resolve_webfinger(
    _address: &str,
    _http_client: &reqwest::Client,
) -> Result<WebFingerResult, AppError> {
    // TODO:
    // 1. Parse address into user and domain
    // 2. Build WebFinger URL: https://{domain}/.well-known/webfinger?resource=acct:{address}
    // 3. Fetch with Accept: application/jrd+json
    // 4. Parse response
    // 5. Find link with type application/activity+json
    // 6. Return result
    todo!()
}

/// WebFinger JRD response
#[derive(Debug, Clone, Deserialize)]
pub struct WebFingerResponse {
    pub subject: String,
    pub aliases: Option<Vec<String>>,
    pub links: Vec<WebFingerLink>,
}

/// WebFinger link
#[derive(Debug, Clone, Deserialize)]
pub struct WebFingerLink {
    pub rel: String,
    #[serde(rename = "type")]
    pub link_type: Option<String>,
    pub href: Option<String>,
    pub template: Option<String>,
}

/// Generate WebFinger response for local account
///
/// Used by /.well-known/webfinger endpoint.
///
/// # Arguments
/// * `username` - Local username
/// * `domain` - Instance domain
///
/// # Returns
/// JRD response for the account
pub fn generate_webfinger_response(_username: &str, _domain: &str) -> WebFingerResponse {
    // TODO:
    // 1. Build subject: acct:{username}@{domain}
    // 2. Add aliases
    // 3. Add links (self, profile page)
    todo!()
}

/// Fetch actor document
///
/// # Arguments
/// * `actor_uri` - ActivityPub actor URI
/// * `http_client` - HTTP client
///
/// # Returns
/// Actor JSON document
pub async fn fetch_actor(
    _actor_uri: &str,
    _http_client: &reqwest::Client,
) -> Result<serde_json::Value, AppError> {
    // TODO:
    // 1. GET actor_uri with Accept: application/activity+json
    // 2. Parse response
    todo!()
}

/// Extract relevant data from actor document
///
/// # Arguments
/// * `actor` - Actor JSON
///
/// # Returns
/// Parsed actor data
pub fn parse_actor(_actor: &serde_json::Value) -> Result<ParsedActor, AppError> {
    // TODO:
    // 1. Extract id, preferredUsername, name, summary
    // 2. Extract icon, image URLs
    // 3. Extract inbox, outbox, followers, following
    // 4. Extract publicKey
    todo!()
}

/// Parsed actor data
#[derive(Debug, Clone)]
pub struct ParsedActor {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub summary: Option<String>,
    pub avatar_url: Option<String>,
    pub header_url: Option<String>,
    pub inbox: String,
    pub outbox: Option<String>,
    pub followers: Option<String>,
    pub following: Option<String>,
    pub public_key_id: String,
    pub public_key_pem: String,
}
