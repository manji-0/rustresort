//! Instance endpoints

use axum::{extract::State, response::Json};
use std::collections::BTreeSet;

use crate::AppState;

const DEFAULT_INSTANCE_RULES: [&str; 3] = [
    "Be respectful and civil in all interactions.",
    "No spam, harassment, or illegal content.",
    "Content warnings are required for sensitive material.",
];

fn domain_from_account_address(address: &str) -> Option<String> {
    let (_, domain) = address.trim().split_once('@')?;
    let domain = domain
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    (!domain.is_empty()).then_some(domain)
}

fn compute_peer_domains(
    follow_addresses: &[String],
    follower_addresses: &[String],
    local_domain: &str,
) -> Vec<String> {
    let local_domain = local_domain.trim().to_ascii_lowercase();
    let mut peers = BTreeSet::new();

    for address in follow_addresses {
        if let Some(domain) = domain_from_account_address(address) {
            if domain != local_domain {
                peers.insert(domain);
            }
        }
    }
    for address in follower_addresses {
        if let Some(domain) = domain_from_account_address(address) {
            if domain != local_domain {
                peers.insert(domain);
            }
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

async fn load_instance_rule_texts(state: &AppState) -> Vec<String> {
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

/// GET /api/v1/instance
pub async fn instance(State(state): State<AppState>) -> Json<serde_json::Value> {
    use crate::api::dto::*;

    let _base_url = state.config.server.base_url();

    // Get account for contact
    let contact_account = if let Ok(Some(account)) = state.db.get_account().await {
        Some(crate::api::account_to_response(&account, &state.config))
    } else {
        None
    };

    // Get stats
    let user_count = 1; // Single-user instance
    let status_count = state
        .db
        .get_local_statuses(1000, None)
        .await
        .map(|s| s.len() as i64)
        .unwrap_or(0);
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
    let peer_domains = compute_peer_domains(
        &follow_addresses,
        &follower_addresses,
        &state.config.server.domain,
    );
    let domain_count = peer_domains.len() as i64;

    let response = InstanceResponse {
        uri: state.config.server.domain.clone(),
        title: state.config.instance.title.clone(),
        short_description: state.config.instance.description.clone(),
        description: state.config.instance.description.clone(),
        email: state.config.instance.contact_email.clone(),
        version: format!("RustResort {}", env!("CARGO_PKG_VERSION")),
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
                    "video/mp4".to_string(),
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
            streaming_api: format!("wss://{}", state.config.server.domain),
        },
        stats: InstanceStats {
            user_count,
            status_count,
            domain_count,
        },
        thumbnail: None,
        contact_account,
    };

    Json(serde_json::to_value(response).unwrap())
}

/// GET /api/v1/instance/peers - Get instance peers
///
/// List of federated instances this instance knows about.
pub async fn instance_peers(State(_state): State<AppState>) -> Json<serde_json::Value> {
    let follow_addresses = _state
        .db
        .get_all_follow_addresses()
        .await
        .unwrap_or_default();
    let follower_addresses = _state
        .db
        .get_all_follower_addresses()
        .await
        .unwrap_or_default();
    let peer_domains = compute_peer_domains(
        &follow_addresses,
        &follower_addresses,
        &_state.config.server.domain,
    );
    Json(serde_json::json!(peer_domains))
}

/// GET /api/v1/instance/activity - Get instance activity
///
/// Instance activity over the last 3 months, binned weekly.
pub async fn instance_activity(State(_state): State<AppState>) -> Json<serde_json::Value> {
    // Return activity statistics for the last 12 weeks
    // For single-user instance, return minimal activity data

    let mut activity = Vec::new();
    let now = chrono::Utc::now();

    for i in 0..12 {
        let week_start = now - chrono::Duration::weeks(11 - i);
        activity.push(serde_json::json!({
            "week": week_start.timestamp().to_string(),
            "statuses": "0",
            "logins": "0",
            "registrations": "0"
        }));
    }

    Json(serde_json::json!(activity))
}

/// GET /api/v1/instance/rules - Get instance rules
///
/// List of rules for this instance.
pub async fn instance_rules(State(state): State<AppState>) -> Json<serde_json::Value> {
    let rules = load_instance_rule_texts(&state).await;
    Json(rules_to_json(&rules))
}

/// GET /api/v2/instance - Get instance information (v2)
///
/// Extended instance information with additional fields.
pub async fn instance_v2(State(state): State<AppState>) -> Json<serde_json::Value> {
    // Get account for contact
    let contact_account = if let Ok(Some(account)) = state.db.get_account().await {
        Some(crate::api::account_to_response(&account, &state.config))
    } else {
        None
    };

    // Get stats
    let _user_count = 1; // Single-user instance
    let _status_count = state
        .db
        .get_local_statuses(1000, None)
        .await
        .map(|s| s.len() as i64)
        .unwrap_or(0);
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
    let peer_domains = compute_peer_domains(
        &follow_addresses,
        &follower_addresses,
        &state.config.server.domain,
    );
    let rules = load_instance_rule_texts(&state).await;

    Json(serde_json::json!({
        "domain": state.config.server.domain,
        "title": state.config.instance.title,
        "version": format!("RustResort {}", env!("CARGO_PKG_VERSION")),
        "source_url": "https://github.com/yourusername/rustresort",
        "description": state.config.instance.description,
        "usage": {
            "users": {
                "active_month": 1
            }
        },
        "thumbnail": {
            "url": null,
            "blurhash": null,
            "versions": {}
        },
        "languages": ["en"],
        "configuration": {
            "urls": {
                "streaming": format!("wss://{}", state.config.server.domain)
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
                    "video/mp4"
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
            "domain_count": peer_domains.len()
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::{compute_peer_domains, rule_texts_from_setting};

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

        let peers = compute_peer_domains(&follows, &followers, "local.example");
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
    fn rule_texts_from_setting_accepts_string_and_object_arrays() {
        let from_strings = rule_texts_from_setting(r#"["One","Two"]"#).unwrap();
        assert_eq!(from_strings, vec!["One".to_string(), "Two".to_string()]);

        let from_objects =
            rule_texts_from_setting(r#"[{"id":"1","text":"Alpha"},{"text":"Beta"}]"#).unwrap();
        assert_eq!(from_objects, vec!["Alpha".to_string(), "Beta".to_string()]);
    }
}
