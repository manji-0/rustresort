// Suppress dead_code warnings for WIP modules (will be removed as features are completed)
#![allow(dead_code)]

//! RustResort - A lightweight, single-user ActivityPub server
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      API Layer (Axum)                        │
//! │  - Mastodon API compatible endpoints                        │
//! │  - ActivityPub endpoints                                    │
//! │  - Admin/Auth endpoints                                     │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//! ┌─────────────────────────────────────────────────────────────┐
//! │                     Service Layer                            │
//! │  - Business logic                                           │
//! │  - Activity processing                                      │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      Data Layer                              │
//! │  - SQLite (sqlx)                                            │
//! │  - Memory cache (moka)                                      │
//! │  - R2 storage                                               │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Modules
//!
//! - `api`: HTTP handlers for Mastodon API and ActivityPub
//! - `service`: Business logic layer
//! - `federation`: ActivityPub federation handling
//! - `data`: Database and cache layer
//! - `storage`: Cloudflare R2 media storage
//! - `auth`: GitHub OAuth authentication
//! - `config`: Configuration management
//! - `error`: Error types

pub mod api;
pub mod auth;
pub mod config;
pub mod data;
pub mod error;
pub mod federation;
pub mod service;
pub mod storage;

use std::sync::Arc;

/// Application state shared across all handlers
///
/// This struct is cloned for each request and contains
/// shared resources like database pool, caches, and HTTP client.
#[derive(Clone)]
pub struct AppState {
    /// Application configuration
    pub config: Arc<config::AppConfig>,

    /// Database connection pool
    pub db: Arc<data::Database>,

    /// Timeline cache (volatile, max 2000 items)
    pub timeline_cache: Arc<data::TimelineCache>,

    /// Profile cache (volatile, fetched on startup)
    pub profile_cache: Arc<data::ProfileCache>,

    /// Media storage (Cloudflare R2)
    pub storage: Arc<storage::MediaStorage>,

    /// Backup service (Cloudflare R2)
    pub backup: Arc<storage::BackupService>,

    /// HTTP client for federation
    pub http_client: Arc<reqwest::Client>,
}

impl AppState {
    /// Initialize application state
    ///
    /// # Steps
    /// 1. Load configuration
    /// 2. Connect to SQLite database
    /// 3. Initialize caches
    /// 4. Connect to R2 storage
    /// 5. Fetch followee/follower profiles
    ///
    /// # Errors
    /// Returns error if any initialization step fails
    pub async fn new(config: config::AppConfig) -> Result<Self, error::AppError> {
        use std::path::Path;

        tracing::info!("Initializing application state...");

        // 1. Connect to SQLite database
        let db_path = Path::new(&config.database.path);
        let db = data::Database::connect(db_path).await?;
        tracing::info!("Database connected");

        // 2. Initialize caches
        let timeline_cache = data::TimelineCache::new(config.cache.timeline_max_items);
        let profile_cache = data::ProfileCache::new();
        tracing::info!("Caches initialized");

        // 3. Initialize HTTP client
        let http_client = reqwest::Client::builder()
            .user_agent("RustResort/0.1.0")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| error::AppError::Internal(e.into()))?;

        // 4. Connect to R2 storage
        let storage = storage::MediaStorage::new(&config.storage.media, &config.cloudflare).await?;
        tracing::info!("Media storage initialized");

        // 5. Initialize backup service
        let backup = storage::BackupService::new(
            &config.storage.backup,
            &config.cloudflare,
            db_path.to_path_buf(),
        )
        .await?;
        tracing::info!("Backup service initialized");

        // 6. Fetch followee/follower profiles
        let follow_addresses = db.get_all_follow_addresses().await?;
        let follower_addresses = db.get_all_follower_addresses().await?;

        tracing::info!(
            follows = follow_addresses.len(),
            followers = follower_addresses.len(),
            "Fetching profiles..."
        );

        // Fetch profiles in parallel
        tokio::join!(
            profile_cache.initialize_from_addresses(&follow_addresses, &http_client),
            profile_cache.initialize_from_addresses(&follower_addresses, &http_client),
        );

        tracing::info!("Application state initialized successfully");

        Ok(Self {
            config: Arc::new(config),
            db: Arc::new(db),
            timeline_cache: Arc::new(timeline_cache),
            profile_cache: Arc::new(profile_cache),
            storage: Arc::new(storage),
            backup: Arc::new(backup),
            http_client: Arc::new(http_client),
        })
    }
}
