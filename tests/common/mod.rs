//! Common test utilities for E2E tests

pub mod schema_validator;

use rustresort::{AppState, config};
use tempfile::TempDir;
use tokio::net::TcpListener;

/// Test server instance
pub struct TestServer {
    pub addr: String,
    pub state: AppState,
    pub _temp_dir: TempDir,
    pub client: reqwest::Client,
}

impl TestServer {
    /// Create a new test server instance
    pub async fn new() -> Self {
        // Create temporary directory for test database
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        // Create test configuration
        let config = config::AppConfig {
            server: config::ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 0, // Let OS assign port
                domain: "test.example.com".to_string(),
                protocol: "https".to_string(),
            },
            database: config::DatabaseConfig {
                path: db_path.clone(),
            },
            storage: config::StorageConfig {
                media: config::MediaStorageConfig {
                    bucket: "test-media".to_string(),
                    public_url: "https://media.test.example.com".to_string(),
                },
                backup: config::BackupStorageConfig {
                    enabled: false,
                    bucket: "test-backup".to_string(),
                    interval_seconds: 86400,
                    retention_count: 7,
                },
            },
            cloudflare: config::CloudflareConfig {
                account_id: "test-account".to_string(),
                r2_access_key_id: "test-key".to_string(),
                r2_secret_access_key: "test-secret".to_string(),
            },
            auth: config::AuthConfig {
                github_username: "testuser".to_string(),
                session_secret: "test-secret-key-32-bytes-long!!".to_string(),
                session_max_age: 604800,
                github: config::GitHubOAuthConfig {
                    client_id: "test-client-id".to_string(),
                    client_secret: "test-client-secret".to_string(),
                },
            },
            instance: config::InstanceConfig {
                title: "Test Instance".to_string(),
                description: "Test RustResort Instance".to_string(),
                contact_email: "test@example.com".to_string(),
            },
            admin: config::AdminConfig {
                username: "testuser".to_string(),
                display_name: "Test User".to_string(),
                email: Some("testuser@test.example.com".to_string()),
                note: Some("Test account".to_string()),
            },
            cache: config::CacheConfig {
                timeline_max_items: 2000,
                profile_ttl: 86400,
            },
            logging: config::LoggingConfig {
                level: "info".to_string(),
                format: "pretty".to_string(),
            },
        };

        // Initialize app state
        let state = AppState::new(config.clone()).await.unwrap();

        // Create HTTP client
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap();

        // Bind to random port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let addr_str = format!("http://{}", addr);

        // Build router
        let app = build_test_router(state.clone());

        // Spawn server in background
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Wait a bit for server to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        Self {
            addr: addr_str,
            state,
            _temp_dir: temp_dir,
            client,
        }
    }

    /// Get base URL for API requests
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.addr, path)
    }

    /// Create a test account in the database
    pub async fn create_test_account(&self) -> rustresort::data::Account {
        use chrono::Utc;
        use rustresort::data::{Account, EntityId};

        let now = Utc::now();
        let account = if let Some(mut account) = self.state.db.get_account().await.unwrap() {
            account.username = "testuser".to_string();
            account.display_name = Some("Test User".to_string());
            account.note = Some("Test bio".to_string());
            account.avatar_s3_key = None;
            account.header_s3_key = None;
            account.private_key_pem = "test_private_key".to_string();
            account.public_key_pem = "test_public_key".to_string();
            account.updated_at = now;
            account
        } else {
            Account {
                id: EntityId::new().0,
                username: "testuser".to_string(),
                display_name: Some("Test User".to_string()),
                note: Some("Test bio".to_string()),
                avatar_s3_key: None,
                header_s3_key: None,
                private_key_pem: "test_private_key".to_string(),
                public_key_pem: "test_public_key".to_string(),
                created_at: now,
                updated_at: now,
            }
        };

        self.state.db.upsert_account(&account).await.unwrap();
        account
    }

    /// Create a test OAuth token
    pub async fn create_test_token(&self) -> String {
        use chrono::{Duration, Utc};
        use rustresort::auth::session::{Session, create_session_token};

        // Create a test session
        let session = Session {
            github_username: "testuser".to_string(),
            github_id: 12345,
            avatar_url: "https://example.com/avatar.png".to_string(),
            name: Some("Test User".to_string()),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::days(7),
        };

        // Generate token using the session secret from config
        create_session_token(&session, &self.state.config.auth.session_secret)
            .expect("Failed to create test token")
    }
}

/// Build router for testing
fn build_test_router(state: AppState) -> axum::Router {
    use axum::Router;
    use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};

    Router::new()
        .route("/health", axum::routing::get(health_check))
        .nest("/.well-known", rustresort::api::wellknown_router())
        .nest("/api", rustresort::api::mastodon_api_router(state.clone()))
        .merge(rustresort::api::activitypub_router())
        .nest("/admin", rustresort::api::admin_router())
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn health_check() -> &'static str {
    "OK"
}
