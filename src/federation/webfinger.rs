//! WebFinger protocol implementation
//!
//! Used to discover ActivityPub actor URIs from addresses.

use serde::{Deserialize, Serialize};

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
    Err(AppError::NotImplemented(
        "WebFinger resolution is not implemented yet".to_string(),
    ))
}

/// WebFinger JRD response
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebFingerResponse {
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    pub links: Vec<WebFingerLink>,
}

/// WebFinger link
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebFingerLink {
    pub rel: String,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
}

/// Generate WebFinger response for a local account.
///
/// # Arguments
/// * `username` - Local username
/// * `domain` - Instance domain
/// * `base_url` - Instance base URL (includes protocol)
///
/// # Returns
/// JRD response for the account
pub fn generate_webfinger_response(username: &str, domain: &str, base_url: &str) -> WebFingerResponse {
    let subject = format!("acct:{}@{}", username, domain);
    let actor_url = format!("{}/users/{}", base_url.trim_end_matches('/'), username);

    WebFingerResponse {
        subject,
        aliases: Some(vec![actor_url.clone()]),
        links: vec![
            WebFingerLink {
                rel: "self".to_string(),
                link_type: Some("application/activity+json".to_string()),
                href: Some(actor_url.clone()),
                template: None,
            },
            WebFingerLink {
                rel: "http://webfinger.net/rel/profile-page".to_string(),
                link_type: Some("text/html".to_string()),
                href: Some(actor_url),
                template: None,
            },
        ],
    }
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
    Err(AppError::NotImplemented(
        "Actor fetch is not implemented yet".to_string(),
    ))
}

/// Extract relevant data from actor document
///
/// # Arguments
/// * `actor` - Actor JSON
///
/// # Returns
/// Parsed actor data
pub fn parse_actor(_actor: &serde_json::Value) -> Result<ParsedActor, AppError> {
    Err(AppError::NotImplemented(
        "Actor parsing is not implemented yet".to_string(),
    ))
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
