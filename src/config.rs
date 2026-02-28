//! Configuration management
//!
//! Loads configuration from:
//! 1. Default values
//! 2. Configuration file (config/local.toml)
//! 3. Environment variables (override)

use serde::Deserialize;
use std::{net::IpAddr, path::PathBuf};

/// Main application configuration
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub storage: StorageConfig,
    pub cloudflare: CloudflareConfig,
    pub auth: AuthConfig,
    pub instance: InstanceConfig,
    pub admin: AdminConfig,
    pub cache: CacheConfig,
    pub logging: LoggingConfig,
}

/// Server configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Bind address (e.g., "0.0.0.0")
    pub host: String,
    /// Port number (e.g., 8080)
    pub port: u16,
    /// Public domain (e.g., "social.example.com")
    pub domain: String,
    /// Protocol ("http" or "https")
    pub protocol: String,
}

impl ServerConfig {
    /// Get the base URL for the instance
    ///
    /// # Returns
    /// Full URL like "https://social.example.com"
    pub fn base_url(&self) -> String {
        format!("{}://{}", self.protocol, self.domain)
    }
}

/// Database configuration (SQLite only)
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    /// Path to SQLite database file
    pub path: PathBuf,
    /// Optional database sync configuration
    #[serde(default)]
    pub sync: DatabaseSyncConfig,
}

/// Database sync configuration
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseSyncConfig {
    /// Sync backend to use
    #[serde(default)]
    pub mode: DatabaseSyncMode,
    /// Sync interval in seconds
    pub interval_seconds: u64,
    /// Turso sync configuration
    #[serde(default)]
    pub turso: TursoSyncConfig,
    /// Cloudflare D1 sync configuration
    #[serde(default)]
    pub d1: D1SyncConfig,
}

impl Default for DatabaseSyncConfig {
    fn default() -> Self {
        Self {
            mode: DatabaseSyncMode::None,
            interval_seconds: 300,
            turso: TursoSyncConfig::default(),
            d1: D1SyncConfig::default(),
        }
    }
}

/// Sync backend selector
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseSyncMode {
    #[default]
    None,
    Turso,
    D1,
}

/// Turso sync configuration
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TursoSyncConfig {
    /// Remote Turso/libSQL URL (e.g. libsql://db-name.turso.io)
    pub remote_url: Option<String>,
    /// Turso auth token
    pub auth_token: Option<String>,
}

/// Cloudflare D1 sync configuration
#[derive(Debug, Clone, Deserialize)]
pub struct D1SyncConfig {
    /// D1 database binding/name
    pub database: Option<String>,
    /// Run against remote D1 database
    #[serde(default)]
    pub remote: bool,
    /// Optional wrangler config path
    pub wrangler_config: Option<PathBuf>,
    /// Optional local snapshot DB path for diff generation
    ///
    /// If omitted, `<database.path>.d1-sync-snapshot.db` is used.
    pub snapshot_path: Option<PathBuf>,
    /// Number of sync history rows to retain on D1.
    ///
    /// Set to 0 to disable pruning.
    #[serde(default = "default_d1_sync_history_retention_count")]
    pub history_retention_count: usize,
}

impl Default for D1SyncConfig {
    fn default() -> Self {
        Self {
            database: None,
            remote: false,
            wrangler_config: None,
            snapshot_path: None,
            history_retention_count: default_d1_sync_history_retention_count(),
        }
    }
}

fn default_d1_sync_history_retention_count() -> usize {
    10_000
}

/// Storage configuration (Cloudflare R2)
#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub media: MediaStorageConfig,
    pub backup: BackupStorageConfig,
}

/// Media storage configuration
#[derive(Debug, Clone, Deserialize)]
pub struct MediaStorageConfig {
    /// R2 bucket name for media
    pub bucket: String,
    /// Public URL for media (Custom Domain)
    /// e.g., "https://media.example.com"
    pub public_url: String,
}

/// Backup storage configuration
#[derive(Debug, Clone, Deserialize)]
pub struct BackupStorageConfig {
    /// Enable automatic backups
    pub enabled: bool,
    /// R2 bucket name for backups (separate from media)
    pub bucket: String,
    /// Backup interval in seconds (default: 86400 = 24h)
    pub interval_seconds: u64,
    /// Number of backup generations to keep
    pub retention_count: usize,
    /// Optional client-side backup encryption.
    #[serde(default)]
    pub encryption: BackupEncryptionConfig,
}

/// Backup encryption configuration
#[derive(Debug, Clone, Deserialize, Default)]
pub struct BackupEncryptionConfig {
    /// Enable AES-256-GCM encryption for backup payloads.
    #[serde(default)]
    pub enabled: bool,
    /// Base64-encoded 32-byte encryption key.
    pub key: Option<String>,
}

/// Cloudflare credentials
#[derive(Debug, Clone, Deserialize)]
pub struct CloudflareConfig {
    /// Cloudflare account ID
    pub account_id: String,
    /// R2 access key ID
    pub r2_access_key_id: String,
    /// R2 secret access key
    pub r2_secret_access_key: String,
}

/// Authentication configuration (GitHub OAuth)
#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    /// Allowed GitHub username (single user)
    pub github_username: String,
    /// Session secret key (32+ bytes)
    pub session_secret: String,
    /// Session max age in seconds (default: 604800 = 7 days)
    pub session_max_age: i64,
    pub github: GitHubOAuthConfig,
}

/// GitHub OAuth configuration
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

/// Instance metadata
#[derive(Debug, Clone, Deserialize)]
pub struct InstanceConfig {
    pub title: String,
    pub description: String,
    pub contact_email: String,
}

/// Admin user configuration
#[derive(Debug, Clone, Deserialize)]
pub struct AdminConfig {
    /// Admin username (default: "admin")
    #[serde(default = "default_admin_username")]
    pub username: String,
    /// Admin display name (default: "Admin")
    #[serde(default = "default_admin_display_name")]
    pub display_name: String,
    /// Admin email (falls back to instance.contact_email if not set)
    pub email: Option<String>,
    /// Admin bio/note
    pub note: Option<String>,
}

fn default_admin_username() -> String {
    "admin".to_string()
}

fn default_admin_display_name() -> String {
    "Admin".to_string()
}

/// Cache configuration
#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    /// Maximum items in timeline cache (default: 2000)
    pub timeline_max_items: usize,
    /// Profile cache TTL in seconds (default: 86400)
    pub profile_ttl: u64,
}

/// Logging configuration
#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    /// Log level: trace, debug, info, warn, error
    pub level: String,
    /// Log format: "pretty" or "json"
    pub format: String,
}

impl AppConfig {
    /// Load configuration from file and environment
    ///
    /// # Loading Order
    /// 1. Default values
    /// 2. config/default.toml (if exists)
    /// 3. config/local.toml (if exists)
    /// 4. Environment variables (RUSTRESORT_*)
    ///
    /// # Errors
    /// Returns error if configuration is invalid
    pub fn load() -> Result<Self, crate::error::AppError> {
        use config::{Config, Environment, File};

        let config = Config::builder()
            // Start with default values
            .set_default("server.host", "127.0.0.1")?
            .set_default("server.port", 8080)?
            .set_default("server.protocol", "http")?
            .set_default("database.sync.mode", "none")?
            .set_default("database.sync.interval_seconds", 300)?
            .set_default("database.sync.d1.remote", false)?
            .set_default("database.sync.d1.history_retention_count", 10000)?
            .set_default("cache.timeline_max_items", 2000)?
            .set_default("cache.profile_ttl", 86400)?
            .set_default("storage.backup.enabled", false)?
            .set_default("storage.backup.interval_seconds", 86400)?
            .set_default("storage.backup.retention_count", 7)?
            .set_default("storage.backup.encryption.enabled", false)?
            .set_default("auth.session_max_age", 604800)?
            .set_default("logging.level", "info")?
            .set_default("logging.format", "pretty")?
            // Load from config/default.toml if it exists
            .add_source(File::with_name("config/default").required(false))
            // Load from config/local.toml if it exists (overrides default)
            .add_source(File::with_name("config/local").required(false))
            // Load from environment variables (RUSTRESORT_*)
            .add_source(
                Environment::with_prefix("RUSTRESORT")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()
            .map_err(|e| crate::error::AppError::Config(e.to_string()))?;

        let app_config: Self = config
            .try_deserialize()
            .map_err(|e| crate::error::AppError::Config(e.to_string()))?;
        app_config.validate()?;
        Ok(app_config)
    }

    pub fn should_use_secure_cookies(&self) -> bool {
        self.server.protocol.eq_ignore_ascii_case("https")
            || !is_local_server_domain(&self.server.domain)
    }

    fn validate(&self) -> Result<(), crate::error::AppError> {
        const MIN_SESSION_SECRET_BYTES: usize = 32;

        if self.auth.session_secret.as_bytes().len() < MIN_SESSION_SECRET_BYTES {
            return Err(crate::error::AppError::Config(format!(
                "auth.session_secret must be at least {} bytes",
                MIN_SESSION_SECRET_BYTES
            )));
        }

        if self.auth.session_max_age <= 0 {
            return Err(crate::error::AppError::Config(
                "auth.session_max_age must be greater than 0".to_string(),
            ));
        }

        if !self.should_use_secure_cookies() {
            let host = normalized_server_host(&self.server.domain);
            tracing::warn!(
                host = %host,
                protocol = %self.server.protocol,
                "Using insecure session cookies for local development"
            );
        } else if !self.server.protocol.eq_ignore_ascii_case("https") {
            return Err(crate::error::AppError::Config(
                "server.protocol must be https for non-local server domains".to_string(),
            ));
        }

        Ok(())
    }
}

fn normalized_server_host(domain: &str) -> String {
    let trimmed = domain.trim();
    let parsed_host = url::Url::parse(&format!("http://{trimmed}"))
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_string()));
    let host = parsed_host.unwrap_or_else(|| trimmed.to_string());
    host.trim_end_matches('.').to_ascii_lowercase()
}

fn is_local_server_domain(domain: &str) -> bool {
    let host = normalized_server_host(domain);
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        return ip.is_loopback() || ip.is_unspecified();
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> AppConfig {
        AppConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
                domain: "localhost".to_string(),
                protocol: "http".to_string(),
            },
            database: DatabaseConfig {
                path: PathBuf::from("/tmp/rustresort-test.db"),
                sync: DatabaseSyncConfig::default(),
            },
            storage: StorageConfig {
                media: MediaStorageConfig {
                    bucket: "media".to_string(),
                    public_url: "https://media.example.com".to_string(),
                },
                backup: BackupStorageConfig {
                    enabled: false,
                    bucket: "backup".to_string(),
                    interval_seconds: 86_400,
                    retention_count: 7,
                    encryption: BackupEncryptionConfig::default(),
                },
            },
            cloudflare: CloudflareConfig {
                account_id: "account".to_string(),
                r2_access_key_id: "access-key".to_string(),
                r2_secret_access_key: "secret-key".to_string(),
            },
            auth: AuthConfig {
                github_username: "admin".to_string(),
                session_secret: "x".repeat(32),
                session_max_age: 604_800,
                github: GitHubOAuthConfig {
                    client_id: "github-client-id".to_string(),
                    client_secret: "github-client-secret".to_string(),
                },
            },
            instance: InstanceConfig {
                title: "RustResort".to_string(),
                description: "Test instance".to_string(),
                contact_email: "admin@example.com".to_string(),
            },
            admin: AdminConfig {
                username: "admin".to_string(),
                display_name: "Admin".to_string(),
                email: None,
                note: None,
            },
            cache: CacheConfig {
                timeline_max_items: 2000,
                profile_ttl: 86_400,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "pretty".to_string(),
            },
        }
    }

    #[test]
    fn validate_accepts_http_on_localhost() {
        let config = valid_config();
        assert!(config.validate().is_ok());
        assert!(!config.should_use_secure_cookies());
    }

    #[test]
    fn validate_rejects_short_session_secret() {
        let mut config = valid_config();
        config.auth.session_secret = "short-secret".to_string();

        let error = config
            .validate()
            .expect_err("session secret shorter than 32 bytes must fail");
        assert!(matches!(
            error,
            crate::error::AppError::Config(message)
                if message.contains("auth.session_secret")
        ));
    }

    #[test]
    fn validate_rejects_http_for_non_local_domain() {
        let mut config = valid_config();
        config.server.domain = "social.example.com".to_string();
        config.server.protocol = "http".to_string();

        let error = config
            .validate()
            .expect_err("public domains must require https");
        assert!(matches!(
            error,
            crate::error::AppError::Config(message)
                if message.contains("server.protocol must be https")
        ));
    }
}
