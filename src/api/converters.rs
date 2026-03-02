//! Conversion functions from database models to API DTOs

use crate::api::dto::*;
use crate::config::AppConfig;
use crate::data::{Account, Database, MediaAttachment, Status};
use crate::error::AppError;
use std::collections::HashMap;

/// Local account counters used in API responses.
#[derive(Debug, Clone, Copy, Default)]
pub struct AccountStats {
    pub followers_count: i32,
    pub following_count: i32,
    pub statuses_count: i32,
}

/// Remote account counters used for status response placeholders.
#[derive(Debug, Clone, Copy, Default)]
pub struct RemoteAccountStats {
    pub followers_count: i32,
    pub following_count: i32,
    pub statuses_count: i32,
}

fn saturating_i32(value: i64) -> i32 {
    if value > i64::from(i32::MAX) {
        i32::MAX
    } else if value < i64::from(i32::MIN) {
        i32::MIN
    } else {
        value as i32
    }
}

fn saturating_u64_i32(value: u64) -> i32 {
    if value > i32::MAX as u64 {
        i32::MAX
    } else {
        value as i32
    }
}

fn default_port_for_protocol(protocol: &str) -> Option<u16> {
    match protocol {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

/// Load local account counters from the database.
pub async fn load_local_account_stats(db: &Database) -> Result<AccountStats, AppError> {
    Ok(AccountStats {
        followers_count: saturating_i32(db.count_follower_addresses().await?),
        following_count: saturating_i32(db.count_follow_addresses().await?),
        statuses_count: saturating_i32(db.count_local_statuses().await?),
    })
}

/// Load remote account status counters for a status list.
///
/// Returns a map keyed by the raw status account address.
pub async fn load_remote_statuses_count_map(
    db: &Database,
    local_protocol: &str,
    statuses: &[Status],
) -> Result<HashMap<String, i32>, AppError> {
    let mut counts = HashMap::new();
    let default_port = default_port_for_protocol(local_protocol);

    for status in statuses {
        if status.is_local {
            continue;
        }
        let account_address = status.account_address.trim();
        if account_address.is_empty() || counts.contains_key(account_address) {
            continue;
        }

        let count = db
            .count_statuses_by_account_address_with_default_port(account_address, default_port)
            .await?;
        counts.insert(account_address.to_string(), saturating_i32(count));
    }

    Ok(counts)
}

/// Load remote account counters for a status list.
///
/// Returns a map keyed by the raw status account address.
pub async fn load_remote_account_stats_map(
    db: &Database,
    profile_cache: &crate::data::ProfileCache,
    local_protocol: &str,
    statuses: &[Status],
) -> Result<HashMap<String, RemoteAccountStats>, AppError> {
    let mut account_stats_map = HashMap::new();
    let default_port = default_port_for_protocol(local_protocol);

    for status in statuses {
        if status.is_local {
            continue;
        }
        let account_address = status.account_address.trim();
        if account_address.is_empty() || account_stats_map.contains_key(account_address) {
            continue;
        }

        let statuses_count = db
            .count_statuses_by_account_address_with_default_port(account_address, default_port)
            .await?;
        let mut remote_stats = RemoteAccountStats {
            statuses_count: saturating_i32(statuses_count),
            ..RemoteAccountStats::default()
        };

        let mut profile = profile_cache.get(account_address).await;
        if profile.is_none() {
            let normalized_address = account_address.to_ascii_lowercase();
            if normalized_address != account_address {
                profile = profile_cache.get(&normalized_address).await;
            }
        }

        if let Some(profile) = profile {
            remote_stats.followers_count =
                profile.followers_count.map(saturating_u64_i32).unwrap_or(0);
            remote_stats.following_count =
                profile.following_count.map(saturating_u64_i32).unwrap_or(0);
        }

        account_stats_map.insert(account_address.to_string(), remote_stats);
    }

    Ok(account_stats_map)
}

/// Convert Account to AccountResponse
#[cfg(test)]
pub fn account_to_response(account: &Account, config: &AppConfig) -> AccountResponse {
    account_to_response_with_stats(account, config, AccountStats::default())
}

/// Convert Account to AccountResponse with explicit counters.
pub fn account_to_response_with_stats(
    account: &Account,
    config: &AppConfig,
    stats: AccountStats,
) -> AccountResponse {
    let base_url = config.server.base_url();
    let media_url = &config.storage.media.public_url;

    AccountResponse {
        id: account.id.to_string(),
        username: account.username.clone(),
        acct: account.username.clone(), // Local account, no @domain
        display_name: account
            .display_name
            .clone()
            .unwrap_or_else(|| account.username.clone()),
        locked: false, // Single user instance, not locked
        bot: false,
        discoverable: true,
        group: false,
        created_at: account.created_at,
        note: account.note.clone().unwrap_or_default(),
        url: format!("{}/users/{}", base_url, account.username),
        avatar: account
            .avatar_s3_key
            .as_ref()
            .map(|key| format!("{}/{}", media_url, key))
            .unwrap_or_else(|| format!("{}/default-avatar.png", media_url)),
        avatar_static: account
            .avatar_s3_key
            .as_ref()
            .map(|key| format!("{}/{}", media_url, key))
            .unwrap_or_else(|| format!("{}/default-avatar.png", media_url)),
        header: account
            .header_s3_key
            .as_ref()
            .map(|key| format!("{}/{}", media_url, key))
            .unwrap_or_else(|| format!("{}/default-header.png", media_url)),
        header_static: account
            .header_s3_key
            .as_ref()
            .map(|key| format!("{}/{}", media_url, key))
            .unwrap_or_else(|| format!("{}/default-header.png", media_url)),
        followers_count: stats.followers_count,
        following_count: stats.following_count,
        statuses_count: stats.statuses_count,
        last_status_at: None,
        emojis: vec![],
        fields: vec![],
    }
}

fn remote_account_to_response(
    status: &Status,
    config: &AppConfig,
    remote_stats: Option<RemoteAccountStats>,
) -> AccountResponse {
    let placeholder_created_at = chrono::DateTime::from_timestamp(0, 0)
        .expect("unix epoch timestamp should always be valid");
    let media_url = &config.storage.media.public_url;
    let address = status.account_address.trim();
    let (username, domain) = address
        .split_once('@')
        .unwrap_or(("unknown", "unknown.invalid"));
    let normalized_username = username.to_ascii_lowercase();
    let normalized_domain = domain.to_ascii_lowercase();
    let acct = format!("{}@{}", normalized_username, normalized_domain);

    AccountResponse {
        id: acct.clone(),
        username: normalized_username.clone(),
        acct,
        display_name: normalized_username.clone(),
        locked: false,
        bot: false,
        discoverable: true,
        group: false,
        // Remote account creation timestamp is unavailable; use a deterministic placeholder.
        created_at: placeholder_created_at,
        note: String::new(),
        url: format!("https://{}/@{}", normalized_domain, normalized_username),
        avatar: format!("{}/default-avatar.png", media_url),
        avatar_static: format!("{}/default-avatar.png", media_url),
        header: format!("{}/default-header.png", media_url),
        header_static: format!("{}/default-header.png", media_url),
        followers_count: remote_stats.map(|stats| stats.followers_count).unwrap_or(0),
        following_count: remote_stats.map(|stats| stats.following_count).unwrap_or(0),
        statuses_count: remote_stats.map(|stats| stats.statuses_count).unwrap_or(0),
        last_status_at: None,
        emojis: vec![],
        fields: vec![],
    }
}

fn local_status_id_from_uri(uri: &str, config: &AppConfig) -> Option<String> {
    let parsed = url::Url::parse(uri).ok()?;
    let local_base = url::Url::parse(&config.server.base_url()).ok()?;

    let parsed_host = parsed
        .host_str()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let local_host = local_base
        .host_str()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if !parsed.scheme().eq_ignore_ascii_case(local_base.scheme())
        || parsed_host != local_host
        || parsed.port_or_known_default() != local_base.port_or_known_default()
    {
        return None;
    }

    let segments = parsed.path_segments()?.collect::<Vec<_>>();
    if segments.len() == 4 && segments[0] == "users" && segments[2] == "statuses" {
        let status_id = segments[3];
        if !status_id.is_empty() {
            return Some(status_id.to_string());
        }
    }
    None
}

fn media_type_from_content_type(content_type: &str) -> &'static str {
    if content_type.starts_with("image/") {
        "image"
    } else if content_type.starts_with("video/") {
        "video"
    } else if content_type.starts_with("audio/") {
        "audio"
    } else {
        "unknown"
    }
}

fn media_url(media_base_url: &str, s3_key: &str) -> String {
    format!(
        "{}/{}",
        media_base_url.trim_end_matches('/'),
        s3_key.trim_start_matches('/')
    )
}

fn media_attachment_to_response(
    attachment: &MediaAttachment,
    config: &AppConfig,
) -> MediaAttachmentResponse {
    let media_base_url = &config.storage.media.public_url;
    let url = media_url(media_base_url, &attachment.s3_key);
    let preview_url = attachment
        .thumbnail_s3_key
        .as_deref()
        .map(|key| media_url(media_base_url, key))
        .unwrap_or_else(|| url.clone());

    let meta = if attachment.width.is_some()
        || attachment.height.is_some()
        || attachment.focus_x.is_some()
        || attachment.focus_y.is_some()
    {
        Some(serde_json::json!({
            "original": {
                "width": attachment.width,
                "height": attachment.height,
                "size": attachment.width.zip(attachment.height).map(|(w, h)| format!("{w}x{h}")),
                "aspect": attachment.width.zip(attachment.height).and_then(|(w, h)| (h != 0).then_some(w as f64 / h as f64)),
                "focus": attachment.focus_x.zip(attachment.focus_y).map(|(x, y)| format!("{x:.3},{y:.3}")),
            }
        }))
    } else {
        None
    };

    MediaAttachmentResponse {
        id: attachment.id.clone(),
        media_type: media_type_from_content_type(&attachment.content_type).to_string(),
        url,
        preview_url,
        remote_url: None,
        text_url: None,
        meta,
        description: attachment.description.clone(),
        blurhash: attachment.blurhash.clone(),
    }
}

fn boost_stub_status(
    boost_of_uri: &str,
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    remote_account_stats: Option<RemoteAccountStats>,
) -> StatusResponse {
    let placeholder_created_at = chrono::DateTime::from_timestamp(0, 0)
        .expect("unix epoch timestamp should always be valid");
    let media_url = &config.storage.media.public_url;
    let boost_account = if local_status_id_from_uri(boost_of_uri, config).is_some() {
        account_to_response_with_stats(account, config, account_stats)
    } else if let Ok(parsed) = url::Url::parse(boost_of_uri) {
        let normalized_host = parsed
            .host_str()
            .map(|host| host.trim_start_matches('[').trim_end_matches(']'))
            .map(str::to_ascii_lowercase)
            .filter(|host| !host.is_empty())
            .unwrap_or_else(|| "unknown.invalid".to_string());
        let authority_host = if normalized_host.contains(':') {
            format!("[{}]", normalized_host)
        } else {
            normalized_host.clone()
        };
        let authority = match parsed.port() {
            Some(port) => format!("{}:{port}", authority_host),
            None => authority_host,
        };
        let normalized_username = parsed
            .path_segments()
            .and_then(|segments| {
                let collected = segments.collect::<Vec<_>>();
                collected
                    .windows(2)
                    .find_map(|window| (window[0] == "users").then_some(window[1]))
                    .or_else(|| {
                        collected
                            .iter()
                            .find_map(|segment| segment.strip_prefix('@'))
                    })
            })
            .map(str::to_ascii_lowercase)
            .filter(|username| !username.is_empty())
            .unwrap_or_else(|| "unknown".to_string());
        let acct = format!("{}@{}", normalized_username, authority);
        let profile_url = format!(
            "{}://{}/@{}",
            parsed.scheme(),
            authority,
            normalized_username
        );
        AccountResponse {
            id: acct.clone(),
            username: normalized_username.clone(),
            acct,
            display_name: normalized_username,
            locked: false,
            bot: false,
            discoverable: true,
            group: false,
            created_at: placeholder_created_at,
            note: String::new(),
            url: profile_url,
            avatar: format!("{}/default-avatar.png", media_url),
            avatar_static: format!("{}/default-avatar.png", media_url),
            header: format!("{}/default-header.png", media_url),
            header_static: format!("{}/default-header.png", media_url),
            followers_count: remote_account_stats
                .map(|stats| stats.followers_count)
                .unwrap_or(0),
            following_count: remote_account_stats
                .map(|stats| stats.following_count)
                .unwrap_or(0),
            statuses_count: remote_account_stats
                .map(|stats| stats.statuses_count)
                .unwrap_or(0),
            last_status_at: None,
            emojis: vec![],
            fields: vec![],
        }
    } else {
        AccountResponse {
            id: "unknown@unknown.invalid".to_string(),
            username: "unknown".to_string(),
            acct: "unknown@unknown.invalid".to_string(),
            display_name: "unknown".to_string(),
            locked: false,
            bot: false,
            discoverable: true,
            group: false,
            created_at: placeholder_created_at,
            note: String::new(),
            url: boost_of_uri.to_string(),
            avatar: format!("{}/default-avatar.png", media_url),
            avatar_static: format!("{}/default-avatar.png", media_url),
            header: format!("{}/default-header.png", media_url),
            header_static: format!("{}/default-header.png", media_url),
            followers_count: remote_account_stats
                .map(|stats| stats.followers_count)
                .unwrap_or(0),
            following_count: remote_account_stats
                .map(|stats| stats.following_count)
                .unwrap_or(0),
            statuses_count: remote_account_stats
                .map(|stats| stats.statuses_count)
                .unwrap_or(0),
            last_status_at: None,
            emojis: vec![],
            fields: vec![],
        }
    };

    StatusResponse {
        id: local_status_id_from_uri(boost_of_uri, config)
            .unwrap_or_else(|| boost_of_uri.to_string()),
        created_at: placeholder_created_at,
        in_reply_to_id: None,
        in_reply_to_account_id: None,
        sensitive: false,
        spoiler_text: String::new(),
        visibility: status.visibility.to_string(),
        language: status.language.clone(),
        uri: boost_of_uri.to_string(),
        url: boost_of_uri.to_string(),
        replies_count: 0,
        reblogs_count: 0,
        favourites_count: 0,
        edited_at: None,
        content: String::new(),
        reblog: None,
        account: boost_account,
        media_attachments: vec![],
        mentions: vec![],
        tags: vec![],
        emojis: vec![],
        card: None,
        poll: None,
        favourited: None,
        reblogged: None,
        muted: None,
        bookmarked: None,
        pinned: None,
    }
}

/// Convert Status to StatusResponse
#[derive(Debug, Clone, Copy, Default)]
pub struct StatusInteractions {
    pub favourited: Option<bool>,
    pub reblogged: Option<bool>,
    pub muted: Option<bool>,
    pub bookmarked: Option<bool>,
    pub pinned: Option<bool>,
}

impl StatusInteractions {
    pub const fn new(
        favourited: Option<bool>,
        reblogged: Option<bool>,
        muted: Option<bool>,
        bookmarked: Option<bool>,
        pinned: Option<bool>,
    ) -> Self {
        Self {
            favourited,
            reblogged,
            muted,
            bookmarked,
            pinned,
        }
    }
}

/// Convert Status to StatusResponse
#[cfg(test)]
pub fn status_to_response(
    status: &Status,
    account: &Account,
    config: &AppConfig,
    interactions: StatusInteractions,
) -> StatusResponse {
    status_to_response_with_account_stats(
        status,
        account,
        config,
        AccountStats::default(),
        interactions,
    )
}

/// Convert Status to StatusResponse with explicit local account stats.
pub fn status_to_response_with_account_stats(
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    interactions: StatusInteractions,
) -> StatusResponse {
    status_to_response_with_account_stats_and_remote_count(
        status,
        account,
        config,
        account_stats,
        None,
        interactions,
    )
}

/// Convert Status to StatusResponse with explicit local account stats and optional remote count.
pub fn status_to_response_with_account_stats_and_remote_count(
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    remote_statuses_count: Option<i32>,
    interactions: StatusInteractions,
) -> StatusResponse {
    let remote_account_stats = remote_statuses_count.map(|statuses_count| RemoteAccountStats {
        statuses_count,
        ..RemoteAccountStats::default()
    });
    status_to_response_with_account_stats_and_remote_stats(
        status,
        account,
        config,
        account_stats,
        remote_account_stats,
        interactions,
    )
}

/// Convert Status to StatusResponse with explicit local account stats and optional remote account stats.
pub fn status_to_response_with_account_stats_and_remote_stats(
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    remote_account_stats: Option<RemoteAccountStats>,
    interactions: StatusInteractions,
) -> StatusResponse {
    status_to_response_with_media(
        status,
        account,
        config,
        account_stats,
        remote_account_stats,
        interactions,
        &[],
    )
}

/// Convert Status to StatusResponse with media attachments
pub fn status_to_response_with_media(
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    remote_account_stats: Option<RemoteAccountStats>,
    interactions: StatusInteractions,
    media_attachments: &[MediaAttachment],
) -> StatusResponse {
    let base_url = config.server.base_url();
    let account_response = if status.is_local || status.account_address.trim().is_empty() {
        account_to_response_with_stats(account, config, account_stats)
    } else {
        remote_account_to_response(status, config, remote_account_stats)
    };

    StatusResponse {
        id: status.id.clone(),
        created_at: status.created_at,
        in_reply_to_id: status
            .in_reply_to_uri
            .as_ref()
            .map(|uri| local_status_id_from_uri(uri, config).unwrap_or_else(|| uri.clone())),
        in_reply_to_account_id: None,
        sensitive: status.content_warning.is_some(),
        spoiler_text: status.content_warning.clone().unwrap_or_default(),
        visibility: status.visibility.to_string(),
        language: status.language.clone(),
        uri: status.uri.clone(),
        url: if status.is_local {
            format!(
                "{}/users/{}/statuses/{}",
                base_url, account.username, status.id
            )
        } else {
            status.uri.clone()
        },
        replies_count: 0,
        reblogs_count: 0,
        favourites_count: 0,
        edited_at: None,
        content: status.content.clone(),
        reblog: status.boost_of_uri.as_deref().map(|uri| {
            Box::new(boost_stub_status(
                uri,
                status,
                account,
                config,
                account_stats,
                remote_account_stats,
            ))
        }),
        account: account_response,
        media_attachments: media_attachments
            .iter()
            .map(|attachment| media_attachment_to_response(attachment, config))
            .collect(),
        mentions: vec![],
        tags: vec![],
        emojis: vec![],
        card: None,
        poll: None,
        favourited: interactions.favourited,
        reblogged: interactions.reblogged,
        muted: interactions.muted,
        bookmarked: interactions.bookmarked,
        pinned: interactions.pinned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use crate::data::{Account, PersistedReason, Status};
    use chrono::Utc;

    fn create_test_config() -> AppConfig {
        AppConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
                domain: "test.example.com".to_string(),
                protocol: "https".to_string(),
            },
            database: DatabaseConfig {
                path: "test.db".into(),
                sync: DatabaseSyncConfig::default(),
            },
            storage: StorageConfig {
                media: MediaStorageConfig {
                    bucket: "test-media".to_string(),
                    public_url: "https://media.test.example.com".to_string(),
                },
                backup: BackupStorageConfig {
                    enabled: false,
                    bucket: "test-backup".to_string(),
                    interval_seconds: 86400,
                    retention_count: 7,
                    encryption: BackupEncryptionConfig::default(),
                },
            },
            cloudflare: CloudflareConfig {
                account_id: "test".to_string(),
                r2_access_key_id: "test".to_string(),
                r2_secret_access_key: "test".to_string(),
            },
            auth: AuthConfig {
                github_username: "testuser".to_string(),
                session_secret: "secret".to_string(),
                session_max_age: 604800,
                github: GitHubOAuthConfig {
                    client_id: "test".to_string(),
                    client_secret: "test".to_string(),
                },
            },
            instance: InstanceConfig {
                title: "Test".to_string(),
                description: "Test instance".to_string(),
                contact_email: "test@example.com".to_string(),
            },
            admin: AdminConfig {
                username: "admin".to_string(),
                display_name: "Admin".to_string(),
                email: Some("admin@test.example.com".to_string()),
                note: Some("Test administrator".to_string()),
            },
            cache: CacheConfig {
                timeline_max_items: 2000,
                profile_ttl: 86400,
            },
            metrics: MetricsConfig::default(),
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "pretty".to_string(),
            },
        }
    }

    #[test]
    fn test_account_to_response() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: Some("Test bio".to_string()),
            avatar_s3_key: Some("avatar.webp".to_string()),
            header_s3_key: Some("header.webp".to_string()),
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let response = account_to_response(&account, &config);

        assert_eq!(response.id, "123");
        assert_eq!(response.username, "testuser");
        assert_eq!(response.acct, "testuser");
        assert_eq!(response.display_name, "Test User");
        assert_eq!(response.note, "Test bio");
        assert_eq!(response.url, "https://test.example.com/users/testuser");
        assert!(response.avatar.contains("media.test.example.com"));
        assert!(response.avatar.contains("avatar.webp"));
        assert!(!response.locked);
        assert!(!response.bot);
    }

    #[test]
    fn test_status_to_response() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Hello, world!</p>".to_string(),
            content_warning: Some("CW".to_string()),
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response = status_to_response(
            &status,
            &account,
            &config,
            StatusInteractions::new(
                Some(true),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            ),
        );

        assert_eq!(response.id, "456");
        assert_eq!(response.content, "<p>Hello, world!</p>");
        assert_eq!(response.spoiler_text, "CW");
        assert_eq!(response.visibility, "public");
        assert_eq!(response.language, Some("en".to_string()));
        assert!(response.sensitive);
        assert_eq!(response.favourited, Some(true));
        assert_eq!(response.reblogged, Some(false));
        assert_eq!(response.muted, Some(false));
        assert_eq!(response.bookmarked, Some(false));
        assert_eq!(response.pinned, Some(false));
        assert_eq!(response.account.username, "testuser");
    }

    #[test]
    fn test_status_to_response_remote_account_uses_stable_placeholder_created_at() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let status = Status {
            id: "remote-1".to_string(),
            uri: "https://remote.example/@alice/123".to_string(),
            content: "<p>Remote</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "alice@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: PersistedReason::Favourited,
            created_at: chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());

        assert_eq!(response.account.acct, "alice@remote.example");
        assert_eq!(
            response.account.created_at,
            chrono::DateTime::from_timestamp(0, 0).unwrap()
        );
        assert_eq!(response.favourited, None);
        assert_eq!(response.reblogged, None);
        assert_eq!(response.muted, None);
        assert_eq!(response.bookmarked, None);
        assert_eq!(response.pinned, None);
    }

    #[test]
    fn test_status_to_response_preserves_in_reply_to_uri_as_id() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Reply</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: None,
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: Some("https://remote.example/users/alice/statuses/123".to_string()),
            boost_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());

        assert_eq!(
            response.in_reply_to_id.as_deref(),
            Some("https://remote.example/users/alice/statuses/123")
        );
    }

    #[test]
    fn test_status_to_response_uses_local_parent_row_id_for_in_reply_to_id() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Reply</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: None,
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: Some(
                "https://test.example.com/users/testuser/statuses/local-123".to_string(),
            ),
            boost_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        assert_eq!(response.in_reply_to_id.as_deref(), Some("local-123"));
    }

    #[test]
    fn test_status_to_response_keeps_local_activity_uri_when_not_canonical_status_path() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let parent_uri = "https://test.example.com/users/testuser/statuses/local-123/activity";
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Reply</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: Some(parent_uri.to_string()),
            boost_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        assert_eq!(response.in_reply_to_id.as_deref(), Some(parent_uri));
    }

    #[test]
    fn test_status_to_response_does_not_map_mismatched_scheme_local_uri_to_row_id() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let parent_uri = "http://test.example.com:443/users/testuser/statuses/local-123";
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Reply</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: Some(parent_uri.to_string()),
            boost_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        assert_eq!(response.in_reply_to_id.as_deref(), Some(parent_uri));
    }

    #[test]
    fn test_status_to_response_populates_reblog_from_boost_uri() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Boost</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: Some("https://remote.example/users/alice/statuses/999".to_string()),
            persisted_reason: PersistedReason::Reposted,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        let reblog = response
            .reblog
            .expect("boosted status should include reblog payload");

        assert_eq!(reblog.id, "https://remote.example/users/alice/statuses/999");
        assert_eq!(
            reblog.uri,
            "https://remote.example/users/alice/statuses/999"
        );
        assert_eq!(reblog.account.acct, "alice@remote.example");
        assert_eq!(reblog.account.username, "alice");
        assert_ne!(reblog.account.username, "testuser");
    }

    #[test]
    fn test_status_to_response_uses_local_boost_row_id_for_reblog_id() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Boost</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: Some(
                "https://test.example.com/users/testuser/statuses/local-123".to_string(),
            ),
            persisted_reason: PersistedReason::Reposted,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        let reblog = response
            .reblog
            .expect("boosted status should include reblog payload");

        assert_eq!(reblog.id, "local-123");
        assert_eq!(
            reblog.uri,
            "https://test.example.com/users/testuser/statuses/local-123"
        );
        assert_eq!(reblog.account.id, account.id.to_string());
        assert_eq!(reblog.account.acct, account.username);
    }

    #[test]
    fn test_status_to_response_reblog_uses_neutral_placeholder_created_at() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let wrapper_created_at = Utc::now();
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Boost</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: Some("https://remote.example/users/alice/statuses/999".to_string()),
            persisted_reason: PersistedReason::Reposted,
            created_at: wrapper_created_at,
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        let reblog = response
            .reblog
            .expect("boosted status should include reblog payload");
        let expected = chrono::DateTime::from_timestamp(0, 0)
            .expect("unix epoch timestamp should always be valid");

        assert_eq!(reblog.created_at, expected);
        assert_ne!(reblog.created_at, wrapper_created_at);
    }

    #[test]
    fn test_status_to_response_reblog_preserves_boost_authority_and_scheme() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Boost</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: Some("http://remote.example:8080/users/alice/statuses/999".to_string()),
            persisted_reason: PersistedReason::Reposted,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        let reblog = response
            .reblog
            .expect("boosted status should include reblog payload");

        assert_eq!(reblog.account.acct, "alice@remote.example:8080");
        assert_eq!(reblog.account.url, "http://remote.example:8080/@alice");
    }

    #[test]
    fn test_status_to_response_with_media_populates_media_attachments() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Media</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };
        let media = MediaAttachment {
            id: "media-1".to_string(),
            status_id: Some(status.id.clone()),
            s3_key: "uploads/media-1.webp".to_string(),
            thumbnail_s3_key: Some("uploads/thumb-media-1.webp".to_string()),
            content_type: "image/webp".to_string(),
            file_size: 1024,
            description: Some("alt".to_string()),
            blurhash: Some("LKO2?U%2Tw=w]~RBVZRi};RPxuwH".to_string()),
            width: Some(1200),
            height: Some(800),
            focus_x: Some(0.1),
            focus_y: Some(-0.2),
            created_at: Utc::now(),
        };

        let response = status_to_response_with_media(
            &status,
            &account,
            &config,
            AccountStats::default(),
            None,
            StatusInteractions::default(),
            &[media],
        );

        assert_eq!(response.media_attachments.len(), 1);
        assert_eq!(response.media_attachments[0].id, "media-1");
        assert_eq!(response.media_attachments[0].media_type, "image");
        assert_eq!(
            response.media_attachments[0].url,
            "https://media.test.example.com/uploads/media-1.webp"
        );
    }
}
