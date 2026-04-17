//! Instance endpoints

use axum::{
    extract::{Query, State},
    response::Json,
};
use chrono::{Datelike, Timelike};
use serde::Deserialize;
use std::collections::BTreeSet;

use crate::InstanceApiState;

const DEFAULT_INSTANCE_RULES: [&str; 3] = [
    "Be respectful and civil in all interactions.",
    "No spam, harassment, or illegal content.",
    "Content warnings are required for sensitive material.",
];

const MASTODON_COMPAT_VERSION: &str = "4.2.0";

fn instance_version_string() -> String {
    format!(
        "{MASTODON_COMPAT_VERSION} (compatible; RustResort/{})",
        env!("CARGO_PKG_VERSION")
    )
}

fn streaming_base_url(base_url: &str) -> Option<String> {
    let url = url::Url::parse(base_url).ok()?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return None,
    }
    Some(url.to_string().trim_end_matches('/').to_string())
}

fn streaming_endpoint_url(state: &InstanceApiState) -> String {
    let base_url = state.config.server.base_url();
    let streaming_base = streaming_base_url(&base_url)
        .unwrap_or_else(|| format!("https://{}", state.config.server.domain));
    format!("{streaming_base}/api/v1/streaming")
}

fn domain_from_account_address(address: &str) -> Option<String> {
    let trimmed = address.trim();
    if let Ok(parsed) = url::Url::parse(trimmed)
        && let Some(host) = parsed.host_str().map(|value| {
            value
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim_end_matches('.')
                .to_ascii_lowercase()
        })
    {
        if host.is_empty() {
            return None;
        }
        let authority = match parsed.port() {
            Some(port) => format!("{}:{port}", format_authority_host(&host)),
            None => format_authority_host(&host),
        };
        return normalize_domain_authority(&authority, None);
    }

    let acct_like = trimmed.strip_prefix("acct:").unwrap_or(trimmed);
    if let Some((_, domain)) = acct_like.split_once('@') {
        return normalize_domain_authority(domain, None);
    }

    None
}

fn default_port_for_protocol(protocol: &str) -> Option<u16> {
    match protocol.to_ascii_lowercase().as_str() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

fn format_authority_host(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn normalize_domain_authority(raw: &str, default_port: Option<u16>) -> Option<String> {
    let parsed = url::Url::parse(&format!("https://{}", raw.trim())).ok()?;
    let host = parsed.host_str().map(|value| {
        value
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim_end_matches('.')
            .to_ascii_lowercase()
    })?;
    if host.is_empty() {
        return None;
    }
    let host = format_authority_host(&host);
    let port = match (parsed.port(), default_port) {
        (Some(port), Some(default_port)) if port == default_port => None,
        (port, _) => port,
    };
    Some(match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

fn compute_peer_domains(
    follow_addresses: &[String],
    follower_addresses: &[String],
    local_domain: &str,
    local_protocol: &str,
) -> Vec<String> {
    let default_port = default_port_for_protocol(local_protocol);
    let local_domain = normalize_domain_authority(local_domain, default_port)
        .unwrap_or_else(|| local_domain.trim().to_ascii_lowercase());
    let mut peers = BTreeSet::new();

    for address in follow_addresses {
        if let Some(domain) = domain_from_account_address(address)
            .and_then(|domain| normalize_domain_authority(&domain, default_port))
            && domain != local_domain
        {
            peers.insert(domain);
        }
    }
    for address in follower_addresses {
        if let Some(domain) = domain_from_account_address(address)
            .and_then(|domain| normalize_domain_authority(&domain, default_port))
            && domain != local_domain
        {
            peers.insert(domain);
        }
    }

    peers.into_iter().collect()
}

fn rule_texts_from_setting(raw: &str) -> Option<Vec<String>> {
    let parsed: serde_json::Value = serde_json::from_str(raw).ok()?;
    let items = parsed.as_array()?;
    let mut rules = Vec::with_capacity(items.len());

    for item in items {
        if let Some(text) = item.as_str().map(str::trim).filter(|text| !text.is_empty()) {
            rules.push(text.to_string());
            continue;
        }

        if let Some(text) = item
            .get("text")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            rules.push(text.to_string());
        }
    }

    (!rules.is_empty()).then_some(rules)
}

async fn load_instance_rule_texts(state: &InstanceApiState) -> Vec<String> {
    if let Ok(Some(raw)) = state.db.get_setting("instance.rules").await {
        if let Some(rules) = rule_texts_from_setting(&raw) {
            return rules;
        }
        tracing::warn!("Invalid JSON in settings key instance.rules; falling back to defaults");
    }

    DEFAULT_INSTANCE_RULES
        .iter()
        .map(|rule| rule.to_string())
        .collect()
}

async fn load_instance_status_count(state: &InstanceApiState) -> i64 {
    state.db.count_local_statuses().await.unwrap_or(0)
}

async fn load_instance_peer_domains(state: &InstanceApiState) -> Vec<String> {
    let follow_addresses = state
        .db
        .get_all_follow_addresses()
        .await
        .unwrap_or_default();
    let follower_addresses = state
        .db
        .get_all_follower_addresses()
        .await
        .unwrap_or_default();
    compute_peer_domains(
        &follow_addresses,
        &follower_addresses,
        &state.config.server.domain,
        &state.config.server.protocol,
    )
}

fn rules_to_json(rule_texts: &[String]) -> serde_json::Value {
    serde_json::Value::Array(
        rule_texts
            .iter()
            .enumerate()
            .map(|(idx, text)| {
                serde_json::json!({
                    "id": (idx + 1).to_string(),
                    "text": text
                })
            })
            .collect(),
    )
}

/// GET /api/v1/custom_emojis
pub async fn custom_emojis() -> Json<serde_json::Value> {
    Json(serde_json::json!([]))
}

/// GET /api/v1/announcements
pub async fn announcements() -> Json<serde_json::Value> {
    Json(serde_json::json!([]))
}

/// GET /api/v1/trends and /api/v1/trends/statuses
pub async fn trending_statuses() -> Json<serde_json::Value> {
    Json(serde_json::json!([]))
}

/// GET /api/v1/trends/links
pub async fn trending_links() -> Json<serde_json::Value> {
    Json(serde_json::json!([]))
}

/// GET /api/v1/trends/tags
pub async fn trending_tags(State(state): State<InstanceApiState>) -> Json<serde_json::Value> {
    let tags = state.db.get_trending_hashtags(10).await.unwrap_or_default();
    Json(serde_json::Value::Array(
        tags.into_iter()
            .map(|(name, usage_count, _last_used)| {
                serde_json::json!({
                    "name": name,
                    "url": format!("{}/tags/{}", state.config.server.base_url(), name),
                    "history": [],
                    "uses": usage_count,
                })
            })
            .collect(),
    ))
}

#[derive(Debug, Default, Deserialize)]
pub struct DirectoryParams {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    pub order: Option<String>,
    pub local: Option<bool>,
}

/// GET /api/v1/directory
pub async fn directory(
    State(state): State<InstanceApiState>,
    Query(params): Query<DirectoryParams>,
) -> Json<serde_json::Value> {
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(40).min(80);
    let _local_only = params.local.unwrap_or(false);
    let _order = params.order.as_deref().unwrap_or("active");

    let results = if offset > 0 || limit == 0 {
        Vec::new()
    } else if let Ok(Some(account)) = state.db.get_account().await {
        let account_stats = crate::api::load_local_account_stats(state.db.as_ref())
            .await
            .unwrap_or_default();
        vec![
            serde_json::to_value(crate::api::account_to_response_with_stats(
                &account,
                &state.config,
                account_stats,
            ))
            .unwrap_or_default(),
        ]
    } else {
        Vec::new()
    };
    Json(serde_json::Value::Array(results))
}

/// GET /api/v1/instance/privacy_policy
pub async fn instance_privacy_policy(
    State(state): State<InstanceApiState>,
) -> Json<serde_json::Value> {
    let content = state
        .db
        .get_setting("instance.privacy_policy")
        .await
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| state.config.instance.description.clone());
    Json(serde_json::json!({
        "content": content
    }))
}

/// GET /api/v1/instance/translation_languages
pub async fn instance_translation_languages() -> Json<serde_json::Value> {
    Json(serde_json::json!({}))
}

/// GET /api/v1/instance
pub async fn instance(State(state): State<InstanceApiState>) -> Json<serde_json::Value> {
    use crate::api::dto::*;

    let _base_url = state.config.server.base_url();

    // Get account for contact
    let contact_account = if let Ok(Some(account)) = state.db.get_account().await {
        let account_stats = crate::api::load_local_account_stats(state.db.as_ref())
            .await
            .unwrap_or_default();
        Some(crate::api::account_to_response_with_stats(
            &account,
            &state.config,
            account_stats,
        ))
    } else {
        None
    };

    // Get stats
    let user_count = 1; // Single-user instance
    let status_count = load_instance_status_count(&state).await;
    let peer_domains = load_instance_peer_domains(&state).await;
    let domain_count = peer_domains.len() as i64;
    let rules = rules_to_json(&load_instance_rule_texts(&state).await)
        .as_array()
        .cloned()
        .unwrap_or_default();

    let response = InstanceResponse {
        uri: state.config.server.domain.clone(),
        title: state.config.instance.title.clone(),
        short_description: state.config.instance.description.clone(),
        description: state.config.instance.description.clone(),
        email: state.config.instance.contact_email.clone(),
        version: instance_version_string(),
        languages: vec!["en".to_string()],
        registrations: false, // Single-user instance
        approval_required: false,
        invites_enabled: false,
        configuration: InstanceConfiguration {
            statuses: StatusesConfiguration {
                max_characters: 500,
                max_media_attachments: 4,
                characters_reserved_per_url: 23,
            },
            media_attachments: MediaConfiguration {
                supported_mime_types: vec![
                    "image/jpeg".to_string(),
                    "image/png".to_string(),
                    "image/gif".to_string(),
                    "image/webp".to_string(),
                    "image/avif".to_string(),
                    "image/heif".to_string(),
                    "image/heic".to_string(),
                    "video/mp4".to_string(),
                    "video/webm".to_string(),
                    "video/quicktime".to_string(),
                ],
                image_size_limit: 10485760,   // 10MB
                image_matrix_limit: 16777216, // 4096x4096
                video_size_limit: 41943040,   // 40MB
                video_frame_rate_limit: 60,
                video_matrix_limit: 2304000, // 1920x1200
            },
            polls: PollsConfiguration {
                max_options: 4,
                max_characters_per_option: 50,
                min_expiration: 300,     // 5 minutes
                max_expiration: 2629746, // 1 month
            },
        },
        urls: InstanceUrls {
            streaming_api: streaming_endpoint_url(&state),
        },
        stats: InstanceStats {
            user_count,
            status_count,
            domain_count,
        },
        thumbnail: None,
        contact_account,
        rules,
    };

    Json(serde_json::to_value(response).unwrap())
}

/// GET /api/v1/instance/peers - Get instance peers
///
/// List of federated instances this instance knows about.
pub async fn instance_peers(State(_state): State<InstanceApiState>) -> Json<serde_json::Value> {
    let peer_domains = load_instance_peer_domains(&_state).await;
    Json(serde_json::json!(peer_domains))
}

/// GET /api/v1/instance/activity - Get instance activity
///
/// Instance activity over the last 3 months, binned weekly.
pub async fn instance_activity(State(_state): State<InstanceApiState>) -> Json<serde_json::Value> {
    let mut activity = Vec::new();
    let now = chrono::Utc::now();
    let week_floor = now
        - chrono::Duration::days(i64::from(now.weekday().num_days_from_monday()))
        - chrono::Duration::seconds(i64::from(now.num_seconds_from_midnight()))
        - chrono::Duration::nanoseconds(i64::from(now.nanosecond()));

    for i in 0..12 {
        let week_start = week_floor - chrono::Duration::weeks(11 - i);
        let week_end = week_start + chrono::Duration::weeks(1);
        let statuses = _state
            .db
            .count_local_statuses_between(week_start, week_end)
            .await
            .unwrap_or(0);
        let logins = _state
            .db
            .count_user_oauth_tokens_created_between(week_start, week_end)
            .await
            .unwrap_or(0);
        let registrations = _state
            .db
            .count_accounts_created_between(week_start, week_end)
            .await
            .unwrap_or(0);
        activity.push(serde_json::json!({
            "week": week_start.timestamp().to_string(),
            "statuses": statuses.to_string(),
            "logins": logins.to_string(),
            "registrations": registrations.to_string()
        }));
    }

    Json(serde_json::json!(activity))
}

/// GET /api/v1/instance/rules - Get instance rules
///
/// List of rules for this instance.
pub async fn instance_rules(State(state): State<InstanceApiState>) -> Json<serde_json::Value> {
    let rules = load_instance_rule_texts(&state).await;
    Json(rules_to_json(&rules))
}

/// GET /api/v2/instance - Get instance information (v2)
///
/// Extended instance information with additional fields.
pub async fn instance_v2(State(state): State<InstanceApiState>) -> Json<serde_json::Value> {
    // Get account for contact
    let contact_account = if let Ok(Some(account)) = state.db.get_account().await {
        let account_stats = crate::api::load_local_account_stats(state.db.as_ref())
            .await
            .unwrap_or_default();
        Some(crate::api::account_to_response_with_stats(
            &account,
            &state.config,
            account_stats,
        ))
    } else {
        None
    };

    // Get stats
    let user_count = 1; // Single-user instance
    let status_count = load_instance_status_count(&state).await;
    let peer_domains = load_instance_peer_domains(&state).await;
    let rules = load_instance_rule_texts(&state).await;

    Json(serde_json::json!({
        "domain": state.config.server.domain,
        "title": state.config.instance.title,
        "version": instance_version_string(),
        "source_url": env!("CARGO_PKG_REPOSITORY"),
        "description": state.config.instance.description,
        "usage": {
            "users": {
                "active_month": 1,
                "total": user_count
            },
            "local_posts": status_count
        },
        "thumbnail": {
            "url": null,
            "blurhash": null,
            "versions": {}
        },
        "languages": ["en"],
        "configuration": {
            "urls": {
                "streaming": streaming_endpoint_url(&state)
            },
            "accounts": {
                "max_featured_tags": 10
            },
            "statuses": {
                "max_characters": 500,
                "max_media_attachments": 4,
                "characters_reserved_per_url": 23
            },
            "media_attachments": {
                "supported_mime_types": [
                    "image/jpeg",
                    "image/png",
                    "image/gif",
                    "image/webp",
                    "image/avif",
                    "image/heif",
                    "image/heic",
                    "video/mp4",
                    "video/webm",
                    "video/quicktime"
                ],
                "image_size_limit": 10485760,
                "image_matrix_limit": 16777216,
                "video_size_limit": 41943040,
                "video_frame_rate_limit": 60,
                "video_matrix_limit": 2304000
            },
            "polls": {
                "max_options": 4,
                "max_characters_per_option": 50,
                "min_expiration": 300,
                "max_expiration": 2629746
            },
            "translation": {
                "enabled": false
            }
        },
        "registrations": {
            "enabled": false,
            "approval_required": false,
            "message": null
        },
        "contact": {
            "email": state.config.instance.contact_email,
            "account": contact_account
        },
        "rules": rules_to_json(&rules),
        "stats": {
            "user_count": user_count,
            "status_count": status_count,
            "domain_count": peer_domains.len()
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::{compute_peer_domains, domain_from_account_address, rule_texts_from_setting};

    #[test]
    fn compute_peer_domains_merges_follows_and_followers_without_duplicates() {
        let follows = vec![
            "alice@remote.example".to_string(),
            "bob@social.example".to_string(),
        ];
        let followers = vec![
            "carol@social.example".to_string(),
            "dave@another.example".to_string(),
        ];

        let peers = compute_peer_domains(&follows, &followers, "local.example", "https");
        assert_eq!(
            peers,
            vec![
                "another.example".to_string(),
                "remote.example".to_string(),
                "social.example".to_string()
            ]
        );
    }

    #[test]
    fn compute_peer_domains_normalizes_default_https_ports() {
        let follows = vec![
            "alice@remote.example".to_string(),
            "bob@remote.example:443".to_string(),
            "carol@remote.example:8443".to_string(),
        ];
        let followers = vec!["dave@local.example:443".to_string()];

        let peers = compute_peer_domains(&follows, &followers, "local.example", "https");
        assert_eq!(
            peers,
            vec![
                "remote.example".to_string(),
                "remote.example:8443".to_string()
            ]
        );
    }

    #[test]
    fn compute_peer_domains_preserves_bracketed_ipv6_with_port() {
        assert_eq!(
            domain_from_account_address("alice@[2001:db8::1]:8443"),
            Some("[2001:db8::1]:8443".to_string())
        );
        let follows = vec!["alice@[2001:db8::1]:8443".to_string()];
        let followers = vec!["bob@[2001:db8::1]:8443".to_string()];

        let peers = compute_peer_domains(&follows, &followers, "local.example", "https");
        assert_eq!(peers, vec!["[2001:db8::1]:8443".to_string()]);
    }

    #[test]
    fn compute_peer_domains_accepts_uri_form_addresses() {
        let follows = vec![
            "https://remote.example/users/alice".to_string(),
            "https://remote.example:443/users/bob".to_string(),
        ];
        let followers = vec![
            "https://remote.example:8443/users/carol".to_string(),
            "https://local.example/users/testuser".to_string(),
        ];

        let peers = compute_peer_domains(&follows, &followers, "local.example", "https");
        assert_eq!(
            peers,
            vec![
                "remote.example".to_string(),
                "remote.example:8443".to_string()
            ]
        );
    }

    #[test]
    fn compute_peer_domains_accepts_uri_form_addresses_with_at_in_path() {
        let follows = vec!["https://remote.example/actors/@alice".to_string()];
        let followers = vec!["https://local.example/users/testuser".to_string()];

        let peers = compute_peer_domains(&follows, &followers, "local.example", "https");
        assert_eq!(peers, vec!["remote.example".to_string()]);
    }

    #[test]
    fn compute_peer_domains_accepts_acct_uri_addresses() {
        let follows = vec!["acct:alice@remote.example".to_string()];
        let followers = vec!["acct:testuser@local.example".to_string()];

        let peers = compute_peer_domains(&follows, &followers, "local.example", "https");
        assert_eq!(peers, vec!["remote.example".to_string()]);
    }

    #[test]
    fn rule_texts_from_setting_accepts_string_and_object_arrays() {
        let from_strings = rule_texts_from_setting(r#"["One","Two"]"#).unwrap();
        assert_eq!(from_strings, vec!["One".to_string(), "Two".to_string()]);

        let from_objects =
            rule_texts_from_setting(r#"[{"id":"1","text":"Alpha"},{"text":"Beta"}]"#).unwrap();
        assert_eq!(from_objects, vec!["Alpha".to_string(), "Beta".to_string()]);
    }
}
