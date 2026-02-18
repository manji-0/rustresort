//! Configuration management
//!
//! Loads configuration from:
//! 1. Default values
//! 2. Configuration file (config/local.toml)
//! 3. Environment variables (override)

use serde::Deserialize;
use std::path::PathBuf;

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

        config
            .try_deserialize()
            .map_err(|e| crate::error::AppError::Config(e.to_string()))
    }
}
