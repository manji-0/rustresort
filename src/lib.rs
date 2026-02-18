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
//! │  - Turso in-memory cache                                    │
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
pub mod metrics;
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
        let turso_sync_options = match config.database.sync.mode {
            config::DatabaseSyncMode::Turso => {
                let remote_url = config
                    .database
                    .sync
                    .turso
                    .remote_url
                    .clone()
                    .ok_or_else(|| {
                        error::AppError::Config(
                            "database.sync.turso.remote_url is required when database.sync.mode=turso"
                                .to_string(),
                        )
                    })?;

                Some(data::TursoSyncOptions {
                    remote_url,
                    auth_token: config.database.sync.turso.auth_token.clone(),
                })
            }
            config::DatabaseSyncMode::D1 => {
                config
                    .database
                    .sync
                    .d1
                    .database
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        error::AppError::Config(
                            "database.sync.d1.database is required and must not be empty when database.sync.mode=d1"
                                .to_string(),
                        )
                    })?;

                data::validate_d1_sync_environment(&config.database.sync.d1)?;
                None
            }
            config::DatabaseSyncMode::None => None,
        };

        let db = data::Database::connect_with_turso_sync(db_path, turso_sync_options).await?;
        tracing::info!("Database connected");

        // 2. Initialize caches
        let timeline_cache = data::TimelineCache::new(config.cache.timeline_max_items).await?;
        let profile_cache = data::ProfileCache::new(config.cache.profile_ttl).await?;
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

        // 7. Initialize admin user
        Self::ensure_admin_user(&db, &config).await?;

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

    /// Ensure admin user exists with current configuration
    ///
    /// Creates or updates the admin user account based on configuration.
    /// Generates RSA keypair if creating new account.
    async fn ensure_admin_user(
        db: &data::Database,
        config: &config::AppConfig,
    ) -> Result<(), error::AppError> {
        use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
        use rsa::{RsaPrivateKey, RsaPublicKey};

        // Check if admin account exists
        if let Some(mut account) = db.get_account().await? {
            // Update admin account if configuration changed
            let mut updated = false;

            if account.username != config.admin.username {
                account.username = config.admin.username.clone();
                updated = true;
            }

            if account.display_name.as_deref() != Some(&config.admin.display_name) {
                account.display_name = Some(config.admin.display_name.clone());
                updated = true;
            }

            let _admin_email = config
                .admin
                .email
                .as_ref()
                .unwrap_or(&config.instance.contact_email);
            // Note: email is not stored in account table currently

            if let Some(ref note) = config.admin.note {
                if account.note.as_deref() != Some(note) {
                    account.note = Some(note.clone());
                    updated = true;
                }
            }

            if updated {
                db.upsert_account(&account).await?;
                tracing::info!(
                    username = %account.username,
                    "Admin account updated"
                );
            } else {
                tracing::info!(
                    username = %account.username,
                    "Admin account exists"
                );
            }

            return Ok(());
        }

        // Create new admin account
        tracing::info!("Creating admin account...");

        // Generate RSA keypair for ActivityPub
        let mut rng = rand::thread_rng();
        let bits = 4096;
        let private_key =
            RsaPrivateKey::new(&mut rng, bits).map_err(|e| error::AppError::Internal(e.into()))?;
        let public_key = RsaPublicKey::from(&private_key);

        // Encode keys to PEM
        let private_key_pem = private_key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|e| error::AppError::Internal(e.into()))?
            .to_string();
        let public_key_pem = public_key
            .to_public_key_pem(LineEnding::LF)
            .map_err(|e| error::AppError::Internal(e.into()))?;

        // Create account
        let account = data::Account {
            id: data::EntityId::new().0,
            username: config.admin.username.clone(),
            display_name: Some(config.admin.display_name.clone()),
            note: config.admin.note.clone(),
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem,
            public_key_pem,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        db.upsert_account(&account).await?;

        tracing::info!(
            username = %account.username,
            display_name = ?account.display_name,
            "Admin account created"
        );

        Ok(())
    }
}

/// Build the Axum router with all routes.
///
/// This is shared by the binary and integration tests to keep route
/// composition consistent across environments.
pub fn build_router(state: AppState) -> axum::Router {
    use axum::Router;
    use tower_http::{compression::CompressionLayer, trace::TraceLayer};

    let cors_layer = build_cors_layer(&state.config.server);

    Router::new()
        .route("/health", axum::routing::get(health_check))
        .merge(auth::auth_router())
        .merge(api::wellknown_router())
        .nest("/api", api::mastodon_api_router(state.clone()))
        .nest("/oauth", api::oauth_router(state.clone()))
        .merge(api::activitypub_router())
        .nest("/admin", api::admin_router())
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer)
        .with_state(state)
        .merge(api::metrics_router())
}

fn build_cors_layer(server: &config::ServerConfig) -> tower_http::cors::CorsLayer {
    use axum::http::HeaderValue;
    use tower_http::cors::{Any, CorsLayer};

    if !server.protocol.eq_ignore_ascii_case("https") {
        return CorsLayer::permissive();
    }

    let allowed_origin = server.base_url();
    match HeaderValue::from_str(&allowed_origin) {
        Ok(origin) => CorsLayer::new()
            .allow_origin([origin])
            .allow_methods(Any)
            .allow_headers(Any),
        Err(error) => {
            tracing::error!(
                %error,
                origin = %allowed_origin,
                "Failed to parse CORS origin from server base URL; denying cross-origin requests"
            );
            CorsLayer::new().allow_methods(Any).allow_headers(Any)
        }
    }
}

async fn health_check() -> &'static str {
    "OK"
}
