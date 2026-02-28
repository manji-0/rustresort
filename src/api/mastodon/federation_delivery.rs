use std::future::Future;
use std::net::IpAddr;
use std::time::Duration;

use crate::AppState;
use crate::data::{Account, CachedProfile};
use crate::error::AppError;
use crate::federation::{ActivityDelivery, DeliveryResult};
use chrono::Utc;

const OUTBOUND_DELIVERY_TIMEOUT_SECS: u64 = 5;

struct DiscoveredRemoteActor {
    actor_uri: String,
    inbox_uri: String,
    actor_document: serde_json::Value,
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
            v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_unspecified()
                || v6.is_multicast()
        }
    }
}

async fn validate_remote_fetch_url(url: &url::Url) -> Result<(), AppError> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(AppError::Validation(
            "Remote URL must use http or https".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Validation(
            "Remote URL must not include user info".to_string(),
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| AppError::Validation("Remote URL must include a host".to_string()))?
        .trim_end_matches('.')
        .to_ascii_lowercase();

    if host == "localhost" || host.ends_with(".localhost") {
        return Err(AppError::Validation(
            "Remote URL host is not allowed".to_string(),
        ));
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip_address(ip) {
            return Err(AppError::Validation(
                "Remote URL host is not allowed".to_string(),
            ));
        }
    }

    let port = url.port_or_known_default().ok_or_else(|| {
        AppError::Validation("Remote URL must include a known default port".to_string())
    })?;
    let mut resolved_any = false;
    let resolved = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|error| {
            AppError::Federation(format!("Failed to resolve remote host {}: {}", host, error))
        })?;
    for address in resolved {
        resolved_any = true;
        if is_blocked_ip_address(address.ip()) {
            return Err(AppError::Validation(
                "Remote URL host is not allowed".to_string(),
            ));
        }
    }
    if !resolved_any {
        return Err(AppError::Federation(format!(
            "Remote host did not resolve to any IP addresses: {}",
            host
        )));
    }

    Ok(())
}

async fn validate_actor_and_inbox_urls(actor_uri: &str, inbox_uri: &str) -> Result<(), AppError> {
    let actor_url = url::Url::parse(actor_uri).map_err(|error| {
        AppError::Federation(format!("Invalid actor URI {} ({})", actor_uri, error))
    })?;
    validate_remote_fetch_url(&actor_url).await?;

    let inbox_url = url::Url::parse(inbox_uri).map_err(|error| {
        AppError::Federation(format!("Invalid inbox URI {} ({})", inbox_uri, error))
    })?;
    validate_remote_fetch_url(&inbox_url).await?;
    Ok(())
}

pub fn local_actor_uri(state: &AppState, username: &str) -> String {
    crate::federation::local_actor_uri(&state.config.server.base_url(), username)
}

pub fn local_key_id(actor_uri: &str) -> String {
    crate::federation::local_key_id(actor_uri)
}

pub fn build_delivery(state: &AppState, account: &Account) -> ActivityDelivery {
    crate::federation::build_local_delivery(
        state.http_client.clone(),
        &state.config.server.base_url(),
        account,
    )
}

pub async fn resolve_remote_actor_and_inbox(
    state: &AppState,
    address: &str,
) -> Result<(String, String), AppError> {
    let address = address.trim();

    if let Some(profile) = state.profile_cache.get(address).await {
        validate_actor_and_inbox_urls(&profile.uri, &profile.inbox_uri).await?;
        return Ok((profile.uri.clone(), profile.inbox_uri.clone()));
    }

    if let Some(actor_uri_address) = parse_actor_uri_address(address) {
        if let Some(profile) = state.profile_cache.get_by_uri(&actor_uri_address).await {
            validate_actor_and_inbox_urls(&profile.uri, &profile.inbox_uri).await?;
            return Ok((profile.uri.clone(), profile.inbox_uri.clone()));
        }
    }

    let discovered = discover_remote_actor_and_inbox(&state.http_client, address).await?;

    if let Some(profile) = build_cached_profile(
        address,
        &discovered.actor_uri,
        &discovered.inbox_uri,
        &discovered.actor_document,
    ) {
        state.profile_cache.insert(profile.clone()).await;

        if profile.address != discovered.actor_uri {
            let mut actor_uri_alias = profile;
            actor_uri_alias.address = discovered.actor_uri.clone();
            state.profile_cache.insert(actor_uri_alias).await;
        }
    } else {
        tracing::warn!(
            address,
            actor_uri = %discovered.actor_uri,
            "Skipping profile cache insert because actor document is missing public key"
        );
    }

    Ok((discovered.actor_uri, discovered.inbox_uri))
}

pub fn spawn_best_effort_delivery<F>(action: &'static str, future: F)
where
    F: Future<Output = Result<(), AppError>> + Send + 'static,
{
    tokio::spawn(async move {
        match tokio::time::timeout(Duration::from_secs(OUTBOUND_DELIVERY_TIMEOUT_SECS), future)
            .await
        {
            Ok(Ok(())) => {
                tracing::info!(action, "Outbound federation delivery completed");
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    action,
                    %error,
                    "Outbound federation delivery failed (no retry policy configured)"
                );
            }
            Err(_) => {
                tracing::warn!(
                    action,
                    timeout_seconds = OUTBOUND_DELIVERY_TIMEOUT_SECS,
                    "Outbound federation delivery timed out (no retry policy configured)"
                );
            }
        }
    });
}

pub fn spawn_best_effort_batch_delivery<F>(action: &'static str, future: F)
where
    F: Future<Output = Vec<DeliveryResult>> + Send + 'static,
{
    tokio::spawn(async move {
        match tokio::time::timeout(Duration::from_secs(OUTBOUND_DELIVERY_TIMEOUT_SECS), future)
            .await
        {
            Ok(results) => {
                let delivered = results.iter().filter(|result| result.success).count();
                let failed = results.len().saturating_sub(delivered);

                if failed == 0 {
                    tracing::info!(
                        action,
                        delivered,
                        "Outbound federation batch delivery completed"
                    );
                } else {
                    tracing::warn!(
                        action,
                        delivered,
                        failed,
                        "Outbound federation batch delivery completed with failures (no retry policy configured)"
                    );
                }
            }
            Err(_) => {
                tracing::warn!(
                    action,
                    timeout_seconds = OUTBOUND_DELIVERY_TIMEOUT_SECS,
                    "Outbound federation batch delivery timed out (no retry policy configured)"
                );
            }
        }
    });
}

async fn discover_remote_actor_and_inbox(
    http_client: &reqwest::Client,
    address: &str,
) -> Result<DiscoveredRemoteActor, AppError> {
    let actor_uri = if let Some(actor_uri) = parse_actor_uri_address(address) {
        actor_uri
    } else {
        let (username, domain) = address.split_once('@').ok_or_else(|| {
            AppError::Validation(
                "Invalid account address format for federation delivery".to_string(),
            )
        })?;

        if username.is_empty() || domain.is_empty() {
            return Err(AppError::Validation(
                "Invalid account address format for federation delivery".to_string(),
            ));
        }

        discover_actor_uri(http_client, username, domain).await?
    };
    let actor = fetch_actor_document(http_client, &actor_uri).await?;
    let canonical_actor_uri = actor
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or(&actor_uri)
        .to_string();
    let inbox_uri = actor
        .get("inbox")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            AppError::Federation(format!(
                "Actor document for {} is missing required inbox URI",
                actor_uri
            ))
        })?
        .to_string();

    validate_actor_and_inbox_urls(&canonical_actor_uri, &inbox_uri).await?;

    Ok(DiscoveredRemoteActor {
        actor_uri: canonical_actor_uri,
        inbox_uri,
        actor_document: actor,
    })
}

fn parse_actor_uri_address(address: &str) -> Option<String> {
    let parsed = url::Url::parse(address.trim()).ok()?;
    match parsed.scheme() {
        "http" | "https" => Some(parsed.to_string()),
        _ => None,
    }
}

fn build_cached_profile(
    address: &str,
    actor_uri: &str,
    inbox_uri: &str,
    actor_document: &serde_json::Value,
) -> Option<CachedProfile> {
    let public_key_pem = extract_public_key_pem(actor_document)?;

    Some(CachedProfile {
        address: address.to_string(),
        uri: actor_uri.to_string(),
        display_name: actor_document
            .get("name")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        note: actor_document
            .get("summary")
            .or_else(|| actor_document.get("note"))
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        avatar_url: actor_document.get("icon").and_then(extract_url),
        header_url: actor_document.get("image").and_then(extract_url),
        public_key_pem,
        inbox_uri: inbox_uri.to_string(),
        outbox_uri: actor_document
            .get("outbox")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        followers_count: actor_document
            .get("followersCount")
            .and_then(|value| value.as_u64()),
        following_count: actor_document
            .get("followingCount")
            .and_then(|value| value.as_u64()),
        fetched_at: Utc::now(),
    })
}

fn extract_public_key_pem(actor_document: &serde_json::Value) -> Option<String> {
    actor_document
        .get("publicKeyPem")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            actor_document
                .get("publicKey")
                .and_then(|value| value.get("publicKeyPem"))
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
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

async fn discover_actor_uri(
    http_client: &reqwest::Client,
    username: &str,
    domain: &str,
) -> Result<String, AppError> {
    let resource = format!("acct:{}@{}", username, domain);
    let webfinger_urls = webfinger_urls_for_domain(domain, &resource)?;
    let mut last_error = None;

    for webfinger_url in webfinger_urls {
        if let Err(error) = validate_remote_fetch_url(&webfinger_url).await {
            last_error = Some(error);
            continue;
        }
        let response = match http_client
            .get(webfinger_url.clone())
            .header("Accept", "application/jrd+json, application/json")
            .send()
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
            return Ok(actor_uri);
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

fn is_supported_webfinger_link_type(link_type: &str) -> bool {
    let normalized = link_type.trim().to_ascii_lowercase();
    normalized.contains("activity+json")
        || (normalized.contains("ld+json") && normalized.contains("activitystreams"))
}

async fn fetch_actor_document(
    http_client: &reqwest::Client,
    actor_uri: &str,
) -> Result<serde_json::Value, AppError> {
    let actor_url = url::Url::parse(actor_uri).map_err(|error| {
        AppError::Federation(format!("Invalid actor URI {} ({})", actor_uri, error))
    })?;
    validate_remote_fetch_url(&actor_url).await?;

    let response = http_client
        .get(actor_url)
        .header(
            "Accept",
            "application/activity+json, application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"",
        )
        .send()
        .await
        .map_err(|error| {
            AppError::Federation(format!(
                "Actor fetch failed for {}: {}",
                actor_uri, error
            ))
        })?;

    if !response.status().is_success() {
        return Err(AppError::Federation(format!(
            "Actor fetch failed for {}: HTTP {}",
            actor_uri,
            response.status()
        )));
    }

    response.json().await.map_err(|error| {
        AppError::Federation(format!(
            "Failed to decode actor document {}: {}",
            actor_uri, error
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_cached_profile, extract_actor_uri_from_webfinger, parse_actor_uri_address,
        validate_remote_fetch_url, webfinger_urls_for_domain,
    };
    use crate::error::AppError;

    #[test]
    fn extract_actor_uri_accepts_activity_json_type() {
        let webfinger = serde_json::json!({
            "links": [
                {
                    "rel": "self",
                    "type": "application/activity+json",
                    "href": "https://remote.example/users/alice"
                }
            ]
        });

        assert_eq!(
            extract_actor_uri_from_webfinger(&webfinger),
            Some("https://remote.example/users/alice".to_string())
        );
    }

    #[test]
    fn extract_actor_uri_accepts_ld_json_activitystreams_profile() {
        let webfinger = serde_json::json!({
            "links": [
                {
                    "rel": "self",
                    "type": "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"",
                    "href": "https://remote.example/@alice"
                }
            ]
        });

        assert_eq!(
            extract_actor_uri_from_webfinger(&webfinger),
            Some("https://remote.example/@alice".to_string())
        );
    }

    #[test]
    fn extract_actor_uri_rejects_missing_type() {
        let webfinger = serde_json::json!({
            "links": [
                {
                    "rel": "self",
                    "href": "https://remote.example/users/alice"
                }
            ]
        });

        assert_eq!(extract_actor_uri_from_webfinger(&webfinger), None);
    }

    #[test]
    fn extract_actor_uri_rejects_non_activitypub_type() {
        let webfinger = serde_json::json!({
            "links": [
                {
                    "rel": "self",
                    "type": "text/html",
                    "href": "https://remote.example/users/alice"
                }
            ]
        });

        assert_eq!(extract_actor_uri_from_webfinger(&webfinger), None);
    }

    #[test]
    fn parse_actor_uri_address_accepts_http_and_https() {
        assert_eq!(
            parse_actor_uri_address("https://remote.example/users/alice"),
            Some("https://remote.example/users/alice".to_string())
        );
        assert_eq!(
            parse_actor_uri_address("http://remote.example/users/alice"),
            Some("http://remote.example/users/alice".to_string())
        );
    }

    #[test]
    fn parse_actor_uri_address_rejects_non_uri_account_addresses() {
        assert_eq!(parse_actor_uri_address("alice@remote.example"), None);
        assert_eq!(parse_actor_uri_address("acct:alice@remote.example"), None);
    }

    #[test]
    fn webfinger_urls_use_http_for_explicit_port_80() {
        let urls = webfinger_urls_for_domain("remote.example:80", "acct:alice@remote.example:80")
            .expect("failed to build webfinger urls");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].scheme(), "http");
    }

    #[test]
    fn webfinger_urls_use_https_for_default_domain() {
        let urls = webfinger_urls_for_domain("remote.example", "acct:alice@remote.example")
            .expect("failed to build webfinger urls");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].scheme(), "https");
    }

    #[test]
    fn webfinger_urls_use_https_for_explicit_port_443() {
        let urls = webfinger_urls_for_domain("remote.example:443", "acct:alice@remote.example:443")
            .expect("failed to build webfinger urls");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].scheme(), "https");
    }

    #[test]
    fn webfinger_urls_try_https_then_http_for_custom_port() {
        let urls =
            webfinger_urls_for_domain("remote.example:8080", "acct:alice@remote.example:8080")
                .expect("failed to build webfinger urls");
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].scheme(), "https");
        assert_eq!(urls[1].scheme(), "http");
    }

    #[test]
    fn build_cached_profile_extracts_actor_and_inbox_fields() {
        let actor = serde_json::json!({
            "id": "https://remote.example/users/alice",
            "name": "Alice",
            "summary": "hello",
            "icon": { "url": "https://remote.example/media/avatar.png" },
            "image": { "url": "https://remote.example/media/header.png" },
            "outbox": "https://remote.example/users/alice/outbox",
            "followersCount": 10,
            "followingCount": 20,
            "publicKey": {
                "publicKeyPem": "-----BEGIN PUBLIC KEY-----\\nabc\\n-----END PUBLIC KEY-----"
            }
        });

        let profile = build_cached_profile(
            "alice@remote.example",
            "https://remote.example/users/alice",
            "https://remote.example/users/alice/inbox",
            &actor,
        )
        .expect("profile should be built");

        assert_eq!(profile.address, "alice@remote.example");
        assert_eq!(profile.uri, "https://remote.example/users/alice");
        assert_eq!(
            profile.inbox_uri,
            "https://remote.example/users/alice/inbox"
        );
        assert_eq!(profile.display_name.as_deref(), Some("Alice"));
        assert_eq!(profile.note.as_deref(), Some("hello"));
        assert_eq!(
            profile.avatar_url.as_deref(),
            Some("https://remote.example/media/avatar.png")
        );
        assert_eq!(
            profile.header_url.as_deref(),
            Some("https://remote.example/media/header.png")
        );
        assert_eq!(profile.followers_count, Some(10));
        assert_eq!(profile.following_count, Some(20));
        assert!(profile.public_key_pem.contains("BEGIN PUBLIC KEY"));
    }

    #[test]
    fn build_cached_profile_requires_public_key() {
        let actor = serde_json::json!({
            "name": "Alice",
            "inbox": "https://remote.example/users/alice/inbox"
        });

        let profile = build_cached_profile(
            "alice@remote.example",
            "https://remote.example/users/alice",
            "https://remote.example/users/alice/inbox",
            &actor,
        );
        assert!(profile.is_none());
    }

    #[tokio::test]
    async fn validate_remote_fetch_url_rejects_loopback_ip() {
        let url =
            url::Url::parse("http://127.0.0.1/users/alice").expect("loopback URL should parse");
        let error = validate_remote_fetch_url(&url)
            .await
            .expect_err("loopback URL must be rejected");
        assert!(matches!(error, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn validate_remote_fetch_url_rejects_localhost_domain() {
        let url =
            url::Url::parse("https://localhost/users/alice").expect("localhost URL should parse");
        let error = validate_remote_fetch_url(&url)
            .await
            .expect_err("localhost URL must be rejected");
        assert!(matches!(error, AppError::Validation(_)));
    }
}
