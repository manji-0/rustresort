//! Conversion functions from database models to API DTOs

use crate::api::dto::*;
use crate::config::AppConfig;
use crate::data::{Account, Status};

/// Convert Account to AccountResponse
pub fn account_to_response(account: &Account, config: &AppConfig) -> AccountResponse {
    let base_url = config.server.base_url();
    let media_url = &config.storage.media.public_url;

    AccountResponse {
        id: account.id.clone(),
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
        followers_count: 0, // Will be populated from database
        following_count: 0, // Will be populated from database
        statuses_count: 0,  // Will be populated from database
        last_status_at: None,
        emojis: vec![],
        fields: vec![],
    }
}

fn remote_account_to_response(status: &Status, config: &AppConfig) -> AccountResponse {
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
        followers_count: 0,
        following_count: 0,
        statuses_count: 0,
        last_status_at: None,
        emojis: vec![],
        fields: vec![],
    }
}

/// Convert Status to StatusResponse
pub fn status_to_response(
    status: &Status,
    account: &Account,
    config: &AppConfig,
    favourited: Option<bool>,
    reblogged: Option<bool>,
    bookmarked: Option<bool>,
) -> StatusResponse {
    let base_url = config.server.base_url();
    let account_response = if status.is_local || status.account_address.trim().is_empty() {
        account_to_response(account, config)
    } else {
        remote_account_to_response(status, config)
    };

    StatusResponse {
        id: status.id.clone(),
        created_at: status.created_at,
        in_reply_to_id: None, // TODO: Extract from in_reply_to_uri
        in_reply_to_account_id: None,
        sensitive: status.content_warning.is_some(),
        spoiler_text: status.content_warning.clone().unwrap_or_default(),
        visibility: status.visibility.clone(),
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
        reblog: None, // TODO: Handle boosts
        account: account_response,
        media_attachments: vec![], // TODO: Load from database
        mentions: vec![],
        tags: vec![],
        emojis: vec![],
        card: None,
        poll: None,
        favourited,
        reblogged,
        bookmarked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use crate::data::{Account, Status};
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
            id: "123".to_string(),
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
            id: "123".to_string(),
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
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: "own".to_string(),
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response = status_to_response(
            &status,
            &account,
            &config,
            Some(true),
            Some(false),
            Some(false),
        );

        assert_eq!(response.id, "456");
        assert_eq!(response.content, "<p>Hello, world!</p>");
        assert_eq!(response.spoiler_text, "CW");
        assert_eq!(response.visibility, "public");
        assert_eq!(response.language, Some("en".to_string()));
        assert!(response.sensitive);
        assert_eq!(response.favourited, Some(true));
        assert_eq!(response.reblogged, Some(false));
        assert_eq!(response.bookmarked, Some(false));
        assert_eq!(response.account.username, "testuser");
    }

    #[test]
    fn test_status_to_response_remote_account_uses_stable_placeholder_created_at() {
        let config = create_test_config();
        let account = Account {
            id: "123".to_string(),
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
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: "alice@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: "favourited".to_string(),
            created_at: chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            fetched_at: None,
        };

        let response = status_to_response(&status, &account, &config, None, None, None);

        assert_eq!(response.account.acct, "alice@remote.example");
        assert_eq!(
            response.account.created_at,
            chrono::DateTime::from_timestamp(0, 0).unwrap()
        );
    }
}
