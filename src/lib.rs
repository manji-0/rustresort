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
//! │  - In-process memory cache                                  │
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

use axum::extract::FromRef;
use std::sync::Arc;

pub const APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

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
    pub storage: Arc<dyn storage::MediaStorageRepository>,

    /// Backup service (Cloudflare R2)
    pub backup: Arc<dyn storage::BackupRepository>,

    /// HTTP client for federation
    pub http_client: Arc<reqwest::Client>,

    /// HTTP client for federation fetches with redirect handling disabled.
    pub federation_fetch_client: Arc<reqwest::Client>,

    /// Federation inbound rate limiter
    pub federation_rate_limiter: Arc<federation::RateLimiter>,

    /// Auth endpoint rate limiter (GitHub callback and OAuth token exchange)
    pub auth_rate_limiter: Arc<federation::RateLimiter>,
}

/// Minimal state required for authentication middleware.
#[derive(Clone)]
pub struct AuthState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
}

/// Minimal state required for GitHub OAuth web login routes.
#[derive(Clone)]
pub struct OAuthWebState {
    pub config: Arc<config::AppConfig>,
    pub http_client: Arc<reqwest::Client>,
    pub auth_rate_limiter: Arc<federation::RateLimiter>,
}

/// Minimal state required for Mastodon timeline endpoints.
#[derive(Clone)]
pub struct TimelineApiState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
    pub timeline_cache: Arc<data::TimelineCache>,
    pub profile_cache: Arc<data::ProfileCache>,
}

/// Minimal state required for Mastodon status endpoints.
#[derive(Clone)]
pub struct StatusApiState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
    pub timeline_cache: Arc<data::TimelineCache>,
    pub profile_cache: Arc<data::ProfileCache>,
    pub storage: Arc<dyn storage::MediaStorageRepository>,
    pub http_client: Arc<reqwest::Client>,
    pub federation_fetch_client: Arc<reqwest::Client>,
}

/// Minimal state required for Mastodon search endpoints.
#[derive(Clone)]
pub struct SearchApiState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
    pub profile_cache: Arc<data::ProfileCache>,
    pub federation_fetch_client: Arc<reqwest::Client>,
}

/// Minimal state required for Mastodon instance endpoints.
#[derive(Clone)]
pub struct InstanceApiState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
}

/// Minimal state required for Mastodon admin API endpoints.
#[derive(Clone)]
pub struct AdminApiState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
}

/// Minimal state required for Mastodon account endpoints.
#[derive(Clone)]
pub struct AccountApiState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
    pub profile_cache: Arc<data::ProfileCache>,
    pub storage: Arc<dyn storage::MediaStorageRepository>,
    pub http_client: Arc<reqwest::Client>,
    pub federation_fetch_client: Arc<reqwest::Client>,
}

/// Minimal state required for Mastodon app/OAuth endpoints.
#[derive(Clone)]
pub struct AppsApiState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
    pub auth_rate_limiter: Arc<federation::RateLimiter>,
}

/// Minimal state required for Mastodon media endpoints.
#[derive(Clone)]
pub struct MediaApiState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
    pub timeline_cache: Arc<data::TimelineCache>,
    pub storage: Arc<dyn storage::MediaStorageRepository>,
}

/// Minimal state required for Mastodon list endpoints.
#[derive(Clone)]
pub struct ListsApiState {
    pub db: Arc<data::Database>,
}

/// Minimal state required for Mastodon filter endpoints.
#[derive(Clone)]
pub struct FiltersApiState {
    pub db: Arc<data::Database>,
}

/// Minimal state required for Mastodon conversation endpoints.
#[derive(Clone)]
pub struct ConversationsApiState {
    pub db: Arc<data::Database>,
}

/// Minimal state required for Mastodon poll endpoints.
#[derive(Clone)]
pub struct PollsApiState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
}

/// Minimal state required for Mastodon scheduled status endpoints.
#[derive(Clone)]
pub struct ScheduledStatusesApiState {
    pub db: Arc<data::Database>,
}

/// Minimal state required for ActivityPub endpoints.
#[derive(Clone)]
pub struct ActivityPubState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
    pub timeline_cache: Arc<data::TimelineCache>,
    pub profile_cache: Arc<data::ProfileCache>,
    pub storage: Arc<dyn storage::MediaStorageRepository>,
    pub http_client: Arc<reqwest::Client>,
    pub federation_rate_limiter: Arc<federation::RateLimiter>,
}

/// Minimal state required for admin API endpoints.
#[derive(Clone)]
pub struct SystemAdminState {
    pub db: Arc<data::Database>,
    pub backup: Arc<dyn storage::BackupRepository>,
}

/// Minimal state required for well-known endpoints.
#[derive(Clone)]
pub struct WellKnownState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
}

macro_rules! impl_from_ref_field {
    ($target:ty, $field:ident) => {
        impl FromRef<AppState> for $target {
            fn from_ref(state: &AppState) -> Self {
                state.$field.clone()
            }
        }
    };
}

macro_rules! impl_from_ref_state {
    ($target:ty { $($field:ident),+ $(,)? }) => {
        impl FromRef<AppState> for $target {
            fn from_ref(state: &AppState) -> Self {
                Self {
                    $($field: state.$field.clone(),)+
                }
            }
        }
    };
}

impl_from_ref_field!(Arc<config::AppConfig>, config);
impl_from_ref_field!(Arc<data::Database>, db);

impl_from_ref_state!(AuthState { config, db });
impl_from_ref_state!(OAuthWebState {
    config,
    http_client,
    auth_rate_limiter,
});
impl_from_ref_state!(TimelineApiState {
    config,
    db,
    timeline_cache,
    profile_cache,
});
impl_from_ref_state!(StatusApiState {
    config,
    db,
    timeline_cache,
    profile_cache,
    storage,
    http_client,
    federation_fetch_client,
});
impl_from_ref_state!(SearchApiState {
    config,
    db,
    profile_cache,
    federation_fetch_client,
});
impl_from_ref_state!(InstanceApiState { config, db });
impl_from_ref_state!(AdminApiState { config, db });
impl_from_ref_state!(AccountApiState {
    config,
    db,
    profile_cache,
    storage,
    http_client,
    federation_fetch_client,
});
impl_from_ref_state!(AppsApiState {
    config,
    db,
    auth_rate_limiter,
});
impl_from_ref_state!(MediaApiState {
    config,
    db,
    timeline_cache,
    storage,
});
impl_from_ref_state!(ListsApiState { db });
impl_from_ref_state!(FiltersApiState { db });
impl_from_ref_state!(ConversationsApiState { db });
impl_from_ref_state!(PollsApiState { config, db });
impl_from_ref_state!(ScheduledStatusesApiState { db });
impl_from_ref_state!(ActivityPubState {
    config,
    db,
    timeline_cache,
    profile_cache,
    storage,
    http_client,
    federation_rate_limiter,
});
impl_from_ref_state!(SystemAdminState { db, backup });
impl_from_ref_state!(WellKnownState { config, db });

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
                #[cfg(not(feature = "turso-sync"))]
                {
                    return Err(error::AppError::Config(
                        "database.sync.mode=turso requires building with the `turso-sync` feature"
                            .to_string(),
                    ));
                }

                #[cfg(feature = "turso-sync")]
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

                #[cfg(feature = "turso-sync")]
                {
                    Some(data::TursoSyncOptions {
                        remote_url,
                        auth_token: config.database.sync.turso.auth_token.clone(),
                    })
                }
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
            .user_agent(APP_USER_AGENT)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(error::AppError::internal)?;
        let federation_fetch_client = reqwest::Client::builder()
            .user_agent(APP_USER_AGENT)
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(error::AppError::internal)?;

        // 4. Initialize federation inbound rate limiter
        let federation_rate_limiter = federation::RateLimiter::new(None, None);
        let auth_rate_limiter =
            federation::RateLimiter::new(Some(30), Some(std::time::Duration::from_secs(60)));

        // 5. Connect to R2 storage
        let storage = storage::MediaStorage::new(&config.storage.media, &config.cloudflare).await?;
        tracing::info!("Media storage initialized");

        // 6. Initialize backup service
        let backup = storage::BackupService::new(
            &config.storage.backup,
            &config.cloudflare,
            db_path.to_path_buf(),
        )
        .await?;
        tracing::info!("Backup service initialized");

        // 7. Fetch followee/follower profiles
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

        // 8. Initialize admin user
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
            federation_fetch_client: Arc::new(federation_fetch_client),
            federation_rate_limiter: Arc::new(federation_rate_limiter),
            auth_rate_limiter: Arc::new(auth_rate_limiter),
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

            if let Some(ref note) = config.admin.note
                && account.note.as_deref() != Some(note)
            {
                account.note = Some(note.clone());
                updated = true;
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
        let private_key = RsaPrivateKey::new(&mut rng, bits).map_err(error::AppError::internal)?;
        let public_key = RsaPublicKey::from(&private_key);

        // Encode keys to PEM
        let private_key_pem = private_key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(error::AppError::internal)?
            .to_string();
        let public_key_pem = public_key
            .to_public_key_pem(LineEnding::LF)
            .map_err(error::AppError::internal)?;

        // Create account
        let account = data::Account {
            id: data::EntityId::new_string(),
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
    use tower_http::{
        compression::CompressionLayer, limit::RequestBodyLimitLayer, trace::TraceLayer,
    };

    let cors_layer = build_cors_layer(&state.config.server);
    let auth_state = AuthState::from_ref(&state);
    let config_state: Arc<config::AppConfig> = Arc::from_ref(&state);
    let security_headers_config = config_state.clone();

    Router::new()
        .route("/health", axum::routing::get(health_check))
        .merge(auth::auth_router())
        .merge(api::wellknown_router())
        .nest("/api", api::mastodon_api_router(auth_state.clone()))
        .nest("/oauth", api::oauth_router(config_state.clone()))
        .merge(api::activitypub_router())
        .nest(
            "/admin",
            api::admin_router().route_layer(axum::middleware::from_fn_with_state(
                config_state.clone(),
                auth::require_session_auth,
            )),
        )
        .merge(
            api::metrics_router().route_layer(axum::middleware::from_fn_with_state(
                config_state,
                auth::require_metrics_auth,
            )),
        )
        .layer(RequestBodyLimitLayer::new(50 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer)
        .layer(axum::middleware::from_fn_with_state(
            security_headers_config,
            append_security_headers,
        ))
        .with_state(state)
}

fn build_cors_layer(server: &config::ServerConfig) -> tower_http::cors::CorsLayer {
    use axum::http::{HeaderName, HeaderValue, Method, header};
    use tower_http::cors::CorsLayer;

    let allowed_methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
    ];
    let allowed_headers = [
        header::AUTHORIZATION,
        header::CONTENT_TYPE,
        header::ACCEPT,
        HeaderName::from_static("idempotency-key"),
    ];

    let allowed_origin = server.base_url();
    match HeaderValue::from_str(&allowed_origin) {
        Ok(origin) => CorsLayer::new()
            .allow_origin([origin])
            .allow_methods(allowed_methods)
            .allow_headers(allowed_headers),
        Err(error) => {
            tracing::error!(
                %error,
                origin = %allowed_origin,
                "Failed to parse CORS origin from server base URL; denying cross-origin requests"
            );
            CorsLayer::new()
                .allow_methods(allowed_methods)
                .allow_headers(allowed_headers)
        }
    }
}

async fn append_security_headers(
    axum::extract::State(config): axum::extract::State<Arc<config::AppConfig>>,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{HeaderValue, header};

    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers
        .entry(header::X_CONTENT_TYPE_OPTIONS)
        .or_insert(HeaderValue::from_static("nosniff"));
    headers
        .entry(header::X_FRAME_OPTIONS)
        .or_insert(HeaderValue::from_static("DENY"));

    if config.server.protocol.eq_ignore_ascii_case("https") {
        headers
            .entry(header::STRICT_TRANSPORT_SECURITY)
            .or_insert(HeaderValue::from_static(
                "max-age=31536000; includeSubDomains",
            ));
    }

    response
}

async fn health_check() -> &'static str {
    "OK"
}
