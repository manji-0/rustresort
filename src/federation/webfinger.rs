//! WebFinger protocol implementation
//!
//! Used to discover ActivityPub actor URIs from addresses.

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};

use crate::error::AppError;

const MAX_FETCH_REDIRECTS: usize = 5;

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

fn parse_actor_uri_address(address: &str) -> Option<String> {
    let parsed = url::Url::parse(address.trim()).ok()?;
    match parsed.scheme() {
        "http" | "https" => Some(parsed.to_string()),
        _ => None,
    }
}

fn parse_account_address(address: &str) -> Result<(String, String), AppError> {
    let trimmed = address.trim();
    let without_prefix = trimmed.strip_prefix("acct:").unwrap_or(trimmed);
    let mut segments = without_prefix.split('@');
    let username = segments.next().unwrap_or_default();
    let domain = segments.next().unwrap_or_default();
    if username.is_empty() || domain.is_empty() || segments.next().is_some() {
        return Err(AppError::Validation(
            "address must be in user@domain format".to_string(),
        ));
    }
    validate_account_domain(domain)?;

    Ok((username.to_string(), domain.to_string()))
}

fn validate_account_domain(domain: &str) -> Result<(), AppError> {
    let parsed = url::Url::parse(&format!("https://{}", domain))
        .map_err(|_| AppError::Validation("address must be in user@domain format".to_string()))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::Validation("address must be in user@domain format".to_string()))?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(AppError::Validation(
            "address must be in user@domain format".to_string(),
        ));
    }

    if host == "localhost" || host.ends_with(".localhost") {
        return Err(AppError::Validation(
            "address host is not allowed".to_string(),
        ));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip_address(ip) {
            return Err(AppError::Validation(
                "address host is not allowed".to_string(),
            ));
        }
    }

    Ok(())
}

fn is_blocked_ip_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
        }
        IpAddr::V6(v6) => {
            if let Some(mapped_v4) = v6.to_ipv4_mapped() {
                return is_blocked_ip_address(IpAddr::V4(mapped_v4));
            }
            v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_unspecified()
                || v6.is_multicast()
        }
    }
}

fn redirects_disabled(http_client: &reqwest::Client) -> bool {
    let debug = format!("{:?}", http_client);
    debug.contains("redirect_policy") && debug.contains("Policy(None)")
}

async fn validate_remote_actor_fetch_url(url: &url::Url) -> Result<(), AppError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Validation(
            "actor URI must not include user info".to_string(),
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| AppError::Validation("actor URI must include a host".to_string()))?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return Err(AppError::Validation(
            "actor URI host is not allowed".to_string(),
        ));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip_address(ip) {
            return Err(AppError::Validation(
                "actor URI host is not allowed".to_string(),
            ));
        }
    }

    let port = url.port_or_known_default().ok_or_else(|| {
        AppError::Validation("actor URI must include a known default port".to_string())
    })?;
    let mut resolved_addrs: Vec<SocketAddr> = Vec::new();
    let resolved = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|error| {
            AppError::Federation(format!("Failed to resolve actor host {}: {}", host, error))
        })?;
    for address in resolved {
        if is_blocked_ip_address(address.ip()) {
            return Err(AppError::Validation(
                "actor URI host is not allowed".to_string(),
            ));
        }
        resolved_addrs.push(address);
    }
    if resolved_addrs.is_empty() {
        return Err(AppError::Federation(format!(
            "actor host did not resolve to any IP addresses: {}",
            host
        )));
    }

    Ok(())
}

async fn fetch_with_validated_redirects(
    http_client: &reqwest::Client,
    start_url: &url::Url,
    accept_header: &str,
) -> Result<reqwest::Response, AppError> {
    if !redirects_disabled(http_client) {
        return Err(AppError::Federation(
            "validated fetch requires an HTTP client with redirects disabled".to_string(),
        ));
    }

    let mut current = start_url.clone();
    for _ in 0..=MAX_FETCH_REDIRECTS {
        validate_remote_actor_fetch_url(&current).await?;

        let response = http_client
            .get(current.clone())
            .header("Accept", accept_header)
            .send()
            .await
            .map_err(|error| AppError::Federation(format!("Actor fetch failed: {}", error)))?;

        if response.status().is_redirection() {
            let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
                return Err(AppError::Federation(
                    "Actor fetch redirect missing Location header".to_string(),
                ));
            };
            let location = location.to_str().map_err(|_| {
                AppError::Federation(
                    "Actor fetch redirect Location was not valid UTF-8".to_string(),
                )
            })?;
            current = current.join(location).map_err(|error| {
                AppError::Federation(format!(
                    "Actor fetch redirect target is invalid URL {} ({})",
                    location, error
                ))
            })?;
            continue;
        }

        return Ok(response);
    }

    Err(AppError::Federation(format!(
        "Actor fetch exceeded redirect limit ({})",
        MAX_FETCH_REDIRECTS
    )))
}

fn extract_explicit_port_from_domain(domain: &str) -> Option<u16> {
    let domain = domain.trim();

    if let Some(rest) = domain.strip_prefix('[') {
        let (_, tail) = rest.split_once(']')?;
        let port_str = tail.strip_prefix(':')?;
        if port_str.is_empty() || !port_str.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        return port_str.parse::<u16>().ok();
    }

    let (host_part, port_str) = domain.rsplit_once(':')?;
    if host_part.is_empty()
        || host_part.contains(':')
        || port_str.is_empty()
        || !port_str.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }

    port_str.parse::<u16>().ok()
}

fn webfinger_urls_for_domain(domain: &str, resource: &str) -> Result<Vec<url::Url>, AppError> {
    url::Url::parse(&format!("http://{}", domain)).map_err(|error| {
        AppError::Federation(format!(
            "Failed to parse remote account domain {}: {}",
            domain, error
        ))
    })?;

    let schemes: &[&str] = match extract_explicit_port_from_domain(domain) {
        Some(80) => &["http"],
        Some(443) | None => &["https"],
        Some(_) => &["https", "http"],
    };

    schemes
        .iter()
        .map(|scheme| {
            let mut url =
                url::Url::parse(&format!("{}://{}/.well-known/webfinger", scheme, domain))
                    .map_err(|error| {
                        AppError::Federation(format!(
                            "Failed to build WebFinger URL for {}: {}",
                            domain, error
                        ))
                    })?;
            url.query_pairs_mut().append_pair("resource", resource);
            Ok(url)
        })
        .collect()
}

fn is_supported_webfinger_link_type(link_type: &str) -> bool {
    let normalized = link_type.trim().to_ascii_lowercase();
    normalized.contains("activity+json")
        || (normalized.contains("ld+json") && normalized.contains("activitystreams"))
}

fn extract_actor_uri_from_webfinger(webfinger: &serde_json::Value) -> Option<String> {
    webfinger
        .get("links")
        .and_then(|value| value.as_array())
        .and_then(|links| {
            links.iter().find_map(|link| {
                let rel = link.get("rel").and_then(|value| value.as_str())?;
                if rel != "self" {
                    return None;
                }
                let link_type = link.get("type").and_then(|value| value.as_str())?;
                if !is_supported_webfinger_link_type(link_type) {
                    return None;
                }
                link.get("href")
                    .and_then(|value| value.as_str())
                    .map(|href| href.to_string())
            })
        })
}

fn extract_profile_url_from_webfinger(webfinger: &serde_json::Value) -> Option<String> {
    webfinger
        .get("links")
        .and_then(|value| value.as_array())
        .and_then(|links| {
            links.iter().find_map(|link| {
                let rel = link.get("rel").and_then(|value| value.as_str())?;
                if rel != "http://webfinger.net/rel/profile-page" && rel != "profile-page" {
                    return None;
                }
                link.get("href")
                    .and_then(|value| value.as_str())
                    .map(|href| href.to_string())
            })
        })
}

fn extract_url(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(url) => Some(url.to_string()),
        serde_json::Value::Array(values) => values.iter().find_map(extract_url),
        serde_json::Value::Object(_) => value
            .get("url")
            .and_then(extract_url)
            .or_else(|| value.get("href").and_then(extract_url)),
        _ => None,
    }
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
    address: &str,
    http_client: &reqwest::Client,
) -> Result<WebFingerResult, AppError> {
    if let Some(actor_uri) = parse_actor_uri_address(address) {
        return Ok(WebFingerResult {
            subject: address.trim().to_string(),
            actor_uri: actor_uri.clone(),
            profile_url: Some(actor_uri),
        });
    }

    let (username, domain) = parse_account_address(address)?;
    let resource = format!("acct:{}@{}", username, domain);
    let webfinger_urls = webfinger_urls_for_domain(&domain, &resource)?;
    let mut last_error = None;

    for webfinger_url in webfinger_urls {
        let response = match fetch_with_validated_redirects(
            http_client,
            &webfinger_url,
            "application/jrd+json, application/json",
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                last_error = Some(AppError::Federation(format!(
                    "WebFinger request failed for {} via {}: {}",
                    resource, webfinger_url, error
                )));
                continue;
            }
        };

        if !response.status().is_success() {
            last_error = Some(AppError::Federation(format!(
                "WebFinger request failed for {} via {}: HTTP {}",
                resource,
                webfinger_url,
                response.status()
            )));
            continue;
        }

        let webfinger: serde_json::Value = match response.json().await {
            Ok(webfinger) => webfinger,
            Err(error) => {
                last_error = Some(AppError::Federation(format!(
                    "Failed to decode WebFinger response for {} via {}: {}",
                    resource, webfinger_url, error
                )));
                continue;
            }
        };

        if let Some(actor_uri) = extract_actor_uri_from_webfinger(&webfinger) {
            let subject = webfinger
                .get("subject")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| resource.clone());
            let profile_url = extract_profile_url_from_webfinger(&webfinger);
            return Ok(WebFingerResult {
                subject,
                actor_uri,
                profile_url,
            });
        }

        last_error = Some(AppError::Federation(format!(
            "WebFinger response for {} via {} did not include an ActivityPub actor URL",
            resource, webfinger_url
        )));
    }

    Err(last_error.unwrap_or_else(|| {
        AppError::Federation(format!(
            "Failed to discover actor URI from WebFinger for {}",
            resource
        ))
    }))
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
pub fn generate_webfinger_response(
    username: &str,
    domain: &str,
    base_url: &str,
) -> WebFingerResponse {
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
    actor_uri: &str,
    http_client: &reqwest::Client,
) -> Result<serde_json::Value, AppError> {
    let parsed = url::Url::parse(actor_uri)
        .map_err(|_| AppError::Validation("actor URI must be a valid URL".to_string()))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(AppError::Validation(
            "actor URI must use http or https".to_string(),
        ));
    }
    validate_remote_actor_fetch_url(&parsed).await?;

    let response = fetch_with_validated_redirects(
        http_client,
        &parsed,
        "application/activity+json, application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"",
    )
    .await?;

    if !response.status().is_success() {
        return Err(AppError::Federation(format!(
            "Actor fetch failed with HTTP {}",
            response.status()
        )));
    }

    response.json().await.map_err(|error| {
        AppError::Federation(format!("Failed to decode actor document: {}", error))
    })
}

/// Extract relevant data from actor document
///
/// # Arguments
/// * `actor` - Actor JSON
///
/// # Returns
/// Parsed actor data
pub fn parse_actor(actor: &serde_json::Value) -> Result<ParsedActor, AppError> {
    let id = actor
        .get("id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| AppError::Federation("Actor document is missing id".to_string()))?;

    let username = actor
        .get("preferredUsername")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::Federation("Actor document is missing preferredUsername".to_string())
        })?;

    let inbox = actor
        .get("inbox")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| AppError::Federation("Actor document is missing inbox".to_string()))?;

    let public_key_id = actor
        .get("publicKey")
        .and_then(|pk| pk.get("id"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}#main-key", id));

    let public_key_pem = actor
        .get("publicKeyPem")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .or_else(|| {
            actor
                .get("publicKey")
                .and_then(|pk| pk.get("publicKeyPem"))
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .ok_or_else(|| {
            AppError::Federation("Actor document is missing publicKeyPem".to_string())
        })?;

    Ok(ParsedActor {
        id,
        username,
        display_name: actor
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        summary: actor
            .get("summary")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        avatar_url: actor.get("icon").and_then(extract_url),
        header_url: actor.get("image").and_then(extract_url),
        inbox,
        outbox: actor
            .get("outbox")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        followers: actor
            .get("followers")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        following: actor
            .get("following")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        public_key_id,
        public_key_pem,
    })
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

#[cfg(test)]
mod tests {
    use super::{
        fetch_actor, generate_webfinger_response, parse_account_address, parse_actor,
        redirects_disabled,
    };

    #[test]
    fn generate_webfinger_response_contains_activitypub_self_link() {
        let response = generate_webfinger_response("alice", "example.com", "https://example.com");

        assert_eq!(response.subject, "acct:alice@example.com");
        assert!(response.links.iter().any(|link| {
            link.rel == "self"
                && link.link_type.as_deref() == Some("application/activity+json")
                && link.href.as_deref() == Some("https://example.com/users/alice")
        }));
    }

    #[test]
    fn parse_actor_extracts_key_fields() {
        let actor = serde_json::json!({
            "id": "https://remote.example/users/alice",
            "preferredUsername": "alice",
            "name": "Alice",
            "summary": "<p>Hello</p>",
            "icon": {"url": "https://remote.example/media/alice.png"},
            "image": {"url": "https://remote.example/media/alice-header.png"},
            "inbox": "https://remote.example/users/alice/inbox",
            "outbox": "https://remote.example/users/alice/outbox",
            "followers": "https://remote.example/users/alice/followers",
            "following": "https://remote.example/users/alice/following",
            "publicKey": {
                "id": "https://remote.example/users/alice#main-key",
                "publicKeyPem": "-----BEGIN PUBLIC KEY-----\\nabc\\n-----END PUBLIC KEY-----"
            }
        });

        let parsed = parse_actor(&actor).expect("actor should parse");
        assert_eq!(parsed.id, "https://remote.example/users/alice");
        assert_eq!(parsed.username, "alice");
        assert_eq!(
            parsed.avatar_url.as_deref(),
            Some("https://remote.example/media/alice.png")
        );
        assert_eq!(
            parsed.header_url.as_deref(),
            Some("https://remote.example/media/alice-header.png")
        );
        assert_eq!(
            parsed.public_key_id,
            "https://remote.example/users/alice#main-key"
        );
    }

    #[test]
    fn parse_actor_rejects_missing_public_key() {
        let actor = serde_json::json!({
            "id": "https://remote.example/users/alice",
            "preferredUsername": "alice",
            "inbox": "https://remote.example/users/alice/inbox"
        });

        let error = parse_actor(&actor).expect_err("missing key should fail");
        assert!(matches!(
            error,
            crate::error::AppError::Federation(message)
                if message.contains("publicKeyPem")
        ));
    }

    #[test]
    fn parse_account_address_rejects_multiple_at_signs() {
        let error =
            parse_account_address("alice@trusted.example@attacker.tld").expect_err("invalid");
        assert!(matches!(
            error,
            crate::error::AppError::Validation(message)
                if message.contains("user@domain format")
        ));
    }

    #[test]
    fn parse_account_address_accepts_acct_prefix() {
        let (username, domain) =
            parse_account_address("acct:alice@trusted.example").expect("valid address");
        assert_eq!(username, "alice");
        assert_eq!(domain, "trusted.example");
    }

    #[test]
    fn parse_account_address_rejects_domain_with_path_or_query() {
        let path_error =
            parse_account_address("alice@example.com/path").expect_err("path must be rejected");
        assert!(matches!(path_error, crate::error::AppError::Validation(_)));

        let query_error =
            parse_account_address("alice@example.com?x=1").expect_err("query must be rejected");
        assert!(matches!(query_error, crate::error::AppError::Validation(_)));
    }

    #[test]
    fn parse_account_address_rejects_private_or_localhost_domains() {
        let localhost_error =
            parse_account_address("alice@localhost").expect_err("localhost must be rejected");
        assert!(matches!(
            localhost_error,
            crate::error::AppError::Validation(_)
        ));

        let loopback_error =
            parse_account_address("alice@127.0.0.1").expect_err("loopback must be rejected");
        assert!(matches!(
            loopback_error,
            crate::error::AppError::Validation(_)
        ));
    }

    #[test]
    fn redirects_disabled_checks_client_policy() {
        let default_client = reqwest::Client::new();
        assert!(!redirects_disabled(&default_client));

        let no_redirect_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client should build");
        assert!(redirects_disabled(&no_redirect_client));
    }

    #[tokio::test]
    async fn fetch_actor_rejects_private_and_localhost_targets() {
        let http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client should build");

        let loopback_error = fetch_actor("http://127.0.0.1:8080/actor", &http_client)
            .await
            .expect_err("loopback targets must be rejected");
        assert!(matches!(
            loopback_error,
            crate::error::AppError::Validation(message)
                if message.contains("host is not allowed")
        ));

        let localhost_error = fetch_actor("http://localhost:8080/actor", &http_client)
            .await
            .expect_err("localhost targets must be rejected");
        assert!(matches!(
            localhost_error,
            crate::error::AppError::Validation(message)
                if message.contains("host is not allowed")
        ));
    }
}
