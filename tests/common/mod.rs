//! Common test utilities for E2E tests

use rustresort::{AppState, config};
use std::net::SocketAddr;
use tempfile::TempDir;
use tokio::net::TcpListener;

const TEST_PRIVATE_KEY_PEM: &str = include_str!("../fixtures/test_private_key.pem");
const TEST_PUBLIC_KEY_PEM: &str = include_str!("../fixtures/test_public_key.pem");

/// Test server instance
#[allow(dead_code)]
pub struct TestServer {
    pub addr: String,
    pub state: AppState,
    pub _temp_dir: TempDir,
    pub client: reqwest::Client,
}

#[allow(dead_code)]
impl TestServer {
    /// Create a new test server instance
    pub async fn new() -> Self {
        Self::with_options(None, true, false).await
    }

    /// Create a new test server instance with optional `/metrics` bearer auth token.
    pub async fn with_metrics_auth_token(metrics_auth_token: Option<&str>) -> Self {
        Self::with_options(metrics_auth_token, true, false).await
    }

    /// Create a test server without pre-seeding the local account.
    pub async fn new_unseeded() -> Self {
        Self::with_options(None, false, false).await
    }

    /// Create a test server with the integrated `/ui` route enabled.
    pub async fn with_ui_enabled() -> Self {
        Self::with_options(None, true, true).await
    }

    async fn with_options(
        metrics_auth_token: Option<&str>,
        preseed_admin_account: bool,
        ui_enabled: bool,
    ) -> Self {
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
                trusted_proxy_ips: Vec::new(),
            },
            database: config::DatabaseConfig {
                path: db_path.clone(),
                sync: config::DatabaseSyncConfig::default(),
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
                    encryption: config::BackupEncryptionConfig::default(),
                },
            },
            cloudflare: config::CloudflareConfig {
                account_id: "test-account".to_string(),
                r2_access_key_id: "test-key".to_string(),
                r2_secret_access_key: "test-secret".to_string(),
            },
            auth: config::AuthConfig {
                username: "testuser".to_string(),
                password: Some("test-password".to_string()),
                session_secret: "test-secret-key-32-bytes-long!!".to_string(),
                session_max_age: 604800,
            },
            instance: config::InstanceConfig {
                title: "Test Instance".to_string(),
                description: "Test RustResort Instance".to_string(),
                contact_email: "test@example.com".to_string(),
            },
            admin: config::AdminConfig {
                display_name: "Test User".to_string(),
                email: Some("testuser@test.example.com".to_string()),
                note: Some("Test account".to_string()),
            },
            cache: config::CacheConfig {
                timeline_max_items: 2000,
                profile_ttl: 86400,
            },
            ui: config::UiConfig {
                enabled: ui_enabled,
                dev_dir: None,
            },
            metrics: config::MetricsConfig {
                auth_token: metrics_auth_token.map(|token| token.to_string()),
            },
            logging: config::LoggingConfig {
                level: "info".to_string(),
                format: "pretty".to_string(),
            },
        };

        // Pre-seed the admin account to avoid expensive RSA key generation
        // in AppState::ensure_admin_user for every test server startup.
        if preseed_admin_account {
            use chrono::Utc;
            use rustresort::data::{Account, Database, EntityId};

            let db = Database::connect(&db_path).await.unwrap();
            let now = Utc::now();
            let seeded_account = Account {
                id: EntityId::new_string(),
                username: "testuser".to_string(),
                display_name: Some("Test User".to_string()),
                note: Some("Test account".to_string()),
                also_known_as: None,
                moved_to_uri: None,
                avatar_s3_key: None,
                header_s3_key: None,
                private_key_pem: TEST_PRIVATE_KEY_PEM.to_string(),
                public_key_pem: TEST_PUBLIC_KEY_PEM.to_string(),
                created_at: now,
                updated_at: now,
            };
            db.upsert_account(&seeded_account).await.unwrap();
        }

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

        // Build router (shared with production composition).
        let app = rustresort::build_router(state.clone());
        rustresort::federation::spawn_delivery_worker(state.clone());
        rustresort::service::spawn_scheduled_status_runner(state.clone());

        // Spawn server in background
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        // Poll health endpoint instead of fixed sleep to minimize startup wait.
        let mut healthy = false;
        for _ in 0..200 {
            match client.get(format!("{}/health", addr_str)).send().await {
                Ok(response) if response.status().is_success() => {
                    healthy = true;
                    break;
                }
                _ => tokio::time::sleep(tokio::time::Duration::from_millis(5)).await,
            }
        }
        assert!(
            healthy,
            "Test server failed to become healthy within the startup timeout"
        );

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

    /// Get the public instance URL for a path based on configured domain/protocol.
    pub fn public_url(&self, path: &str) -> String {
        format!("{}{}", self.state.config.server.base_url(), path)
    }

    /// Register a test-only inbound public key override so signed federation
    /// requests can be verified without external network fetches.
    pub fn register_inbound_public_key(&self, key_id: &str, public_key_pem: &str) {
        self.state
            .inbound_public_key_overrides
            .write()
            .unwrap()
            .insert(key_id.to_string(), public_key_pem.to_string());
    }

    /// Send a signed ActivityPub request to the started test server.
    pub async fn post_signed_activity(
        &self,
        path: &str,
        activity: &serde_json::Value,
        key_id: &str,
    ) -> reqwest::Response {
        let public_url = self.public_url(path);
        let body = serde_json::to_vec(activity).unwrap();
        let signed = rustresort::federation::sign_request(
            "POST",
            &public_url,
            Some(&body),
            TEST_PRIVATE_KEY_PEM,
            key_id,
        )
        .unwrap();
        let parsed_public_url = url::Url::parse(&public_url).unwrap();

        let mut request = self
            .client
            .post(self.url(path))
            .header("Content-Type", "application/activity+json")
            .header("Host", parsed_public_url.host_str().unwrap())
            .header("Date", signed.date)
            .header("Signature", signed.signature)
            .body(body);

        if let Some(digest) = signed.digest {
            request = request.header("Digest", digest);
        }

        request.send().await.unwrap()
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
            account.private_key_pem = TEST_PRIVATE_KEY_PEM.to_string();
            account.public_key_pem = TEST_PUBLIC_KEY_PEM.to_string();
            account.updated_at = now;
            account
        } else {
            Account {
                id: EntityId::new_string(),
                username: "testuser".to_string(),
                display_name: Some("Test User".to_string()),
                note: Some("Test bio".to_string()),
                also_known_as: None,
                moved_to_uri: None,
                avatar_s3_key: None,
                header_s3_key: None,
                private_key_pem: TEST_PRIVATE_KEY_PEM.to_string(),
                public_key_pem: TEST_PUBLIC_KEY_PEM.to_string(),
                created_at: now,
                updated_at: now,
            }
        };

        self.state.db.upsert_account(&account).await.unwrap();
        account
    }

    /// Create a test local session token
    pub async fn create_test_session_token(&self) -> String {
        use chrono::{Duration, Utc};
        use rustresort::auth::session::{Session, create_session_token};

        // Create a test session
        let session = Session {
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            auth_method: "test".to_string(),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::days(7),
        };

        // Generate token using the session secret from config
        create_session_token(&session, &self.state.config.auth.session_secret)
            .expect("Failed to create test token")
    }

    /// Create a broad-scope OAuth token for API compatibility tests.
    pub async fn create_test_token(&self) -> String {
        use chrono::{Duration, Utc};
        use rustresort::data::EntityId;
        use rustresort_models::{OAuthApp, OAuthToken};

        self.create_test_account().await;

        let now = Utc::now();
        let app = OAuthApp {
            id: EntityId::new_string(),
            name: "RustResort Test Client".to_string(),
            website: Some("https://client.example".to_string()),
            redirect_uri: "urn:ietf:wg:oauth:2.0:oob".to_string(),
            client_id: EntityId::new_string(),
            client_secret: EntityId::new_string(),
            vapid_key: None,
            scopes: concat!(
                "read ",
                "write ",
                "follow ",
                "push ",
                "admin:read ",
                "admin:write ",
                "read:accounts ",
                "write:accounts ",
                "read:statuses ",
                "write:statuses ",
                "write:favourites ",
                "read:notifications ",
                "write:notifications ",
                "write:media ",
                "read:lists ",
                "write:lists ",
                "read:filters ",
                "write:filters ",
                "read:search"
            )
            .to_string(),
            created_at: now,
        };
        self.state.db.insert_oauth_app(&app).await.unwrap();

        let access_token = EntityId::new_string();
        let token = OAuthToken {
            id: EntityId::new_string(),
            app_id: app.id.clone(),
            access_token: access_token.clone(),
            grant_type: "authorization_code".to_string(),
            scopes: app.scopes.clone(),
            created_at: now,
            expires_at: now + Duration::days(7),
            revoked: false,
        };
        self.state.db.insert_oauth_token(&token).await.unwrap();

        access_token
    }

    /// Log in with the built-in password and return `(access_token, session_cookie)`.
    pub async fn login_password(&self) -> (String, String) {
        let response = self
            .client
            .post(self.url("/auth/login"))
            .json(&serde_json::json!({
                "username": self.state.config.auth.username.clone(),
                "password": self.state.config.auth.password.clone(),
            }))
            .send()
            .await
            .expect("password login request");

        assert!(
            response.status().is_success(),
            "password login failed: {}",
            response.status()
        );

        let set_cookie = response
            .headers()
            .get("set-cookie")
            .and_then(|value| value.to_str().ok())
            .expect("session cookie header")
            .to_string();
        let session_cookie = set_cookie
            .split(';')
            .next()
            .expect("cookie pair")
            .to_string();
        let body = response
            .json::<serde_json::Value>()
            .await
            .expect("password login body");
        let access_token = body["access_token"]
            .as_str()
            .expect("session access token")
            .to_string();
        (access_token, session_cookie)
    }

    /// Create an OAuth app for compatibility tests.
    pub async fn create_oauth_app(&self, redirect_uris: &str, scopes: &str) -> serde_json::Value {
        let response = self
            .client
            .post(self.url("/api/v1/apps"))
            .json(&serde_json::json!({
                "client_name": "RustResort E2E Client",
                "redirect_uris": redirect_uris,
                "scopes": scopes,
                "website": "https://client.example",
            }))
            .send()
            .await
            .expect("create oauth app request");
        assert!(
            response.status().is_success(),
            "create oauth app failed: {}",
            response.status()
        );
        response
            .json::<serde_json::Value>()
            .await
            .expect("create oauth app body")
    }

    /// Create an OAuth authorization-code token for the local account.
    pub async fn create_oauth_authorization_code_token(&self, scopes: &str) -> String {
        let redirect_uri = "https://client.example/callback";
        let app = self.create_oauth_app(redirect_uri, scopes).await;
        let client_id = app["client_id"].as_str().expect("client_id");
        let client_secret = app["client_secret"].as_str().expect("client_secret");
        let (_, session_cookie) = self.login_password().await;

        let no_redirect_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("redirect-free client");
        let authorize = no_redirect_client
            .get(self.url("/oauth/authorize"))
            .query(&[
                ("response_type", "code"),
                ("client_id", client_id),
                ("redirect_uri", redirect_uri),
                ("scope", scopes),
                ("state", "state-123"),
            ])
            .header("Cookie", &session_cookie)
            .send()
            .await
            .expect("authorize request");
        assert!(
            authorize.status() == reqwest::StatusCode::FOUND
                || authorize.status() == reqwest::StatusCode::SEE_OTHER,
            "authorize should redirect back to client, got {}",
            authorize.status()
        );

        let redirect_location = authorize
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .expect("authorize redirect location");
        let redirect_url = url::Url::parse(redirect_location).expect("redirect URL");
        let code = redirect_url
            .query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, value)| value.into_owned())
            .expect("authorization code");

        let token = self
            .client
            .post(self.url("/oauth/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("redirect_uri", redirect_uri),
                ("code", code.as_str()),
                ("scope", scopes),
            ])
            .send()
            .await
            .expect("token exchange request");
        assert!(
            token.status().is_success(),
            "token exchange failed: {}",
            token.status()
        );
        token
            .json::<serde_json::Value>()
            .await
            .expect("token exchange body")["access_token"]
            .as_str()
            .expect("oauth access token")
            .to_string()
    }
}

#[allow(dead_code)]
pub fn test_public_key_pem() -> &'static str {
    TEST_PUBLIC_KEY_PEM
}
