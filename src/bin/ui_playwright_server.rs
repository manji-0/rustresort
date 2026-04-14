use rustresort::{AppState, build_router, config};
use std::{env, net::SocketAddr, path::PathBuf};
use tempfile::TempDir;
use tokio::net::TcpListener;

const TEST_PRIVATE_KEY_PEM: &str = include_str!("../../tests/fixtures/test_private_key.pem");
const TEST_PUBLIC_KEY_PEM: &str = include_str!("../../tests/fixtures/test_public_key.pem");
const REMOTE_ACTOR_ID: &str = "https://remote.example/users/alice";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = env::var("RUSTRESORT_UI_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("RUSTRESORT_UI_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(3011);
    let username = env::var("RUSTRESORT_UI_USERNAME").unwrap_or_else(|_| "admin".to_string());
    let password =
        env::var("RUSTRESORT_UI_PASSWORD").unwrap_or_else(|_| "admin-password".to_string());
    let db_path_env = env::var("RUSTRESORT_UI_DB_PATH").ok().map(PathBuf::from);
    let _temp_dir = db_path_env.is_none().then(TempDir::new).transpose()?;
    let db_path = db_path_env.unwrap_or_else(|| {
        _temp_dir
            .as_ref()
            .expect("temp dir should exist when db path is not provided")
            .path()
            .join("ui-playwright.db")
    });

    seed_account_if_needed(&db_path, &username).await?;

    let app_config = config::AppConfig {
        server: config::ServerConfig {
            host: host.clone(),
            port,
            domain: format!("localhost:{port}"),
            protocol: "http".to_string(),
            trusted_proxy_ips: Vec::new(),
        },
        database: config::DatabaseConfig {
            path: db_path,
            sync: config::DatabaseSyncConfig::default(),
        },
        storage: config::StorageConfig {
            media: config::MediaStorageConfig {
                bucket: "ui-playwright-media".to_string(),
                public_url: format!("http://localhost:{port}/media"),
            },
            backup: config::BackupStorageConfig {
                enabled: false,
                bucket: "ui-playwright-backups".to_string(),
                interval_seconds: 86400,
                retention_count: 7,
                encryption: config::BackupEncryptionConfig::default(),
            },
        },
        cloudflare: config::CloudflareConfig {
            account_id: "ui-playwright".to_string(),
            r2_access_key_id: "ui-playwright".to_string(),
            r2_secret_access_key: "ui-playwright".to_string(),
        },
        auth: config::AuthConfig {
            username: username.clone(),
            password: Some(password),
            session_secret: "0123456789abcdef0123456789abcdef".to_string(),
            session_max_age: 604800,
        },
        instance: config::InstanceConfig {
            title: "RustResort Playwright UI".to_string(),
            description: "Playwright UI harness".to_string(),
            contact_email: "admin@example.com".to_string(),
        },
        admin: config::AdminConfig {
            display_name: "Admin".to_string(),
            email: Some("admin@example.com".to_string()),
            note: Some("Playwright admin account".to_string()),
        },
        cache: config::CacheConfig {
            timeline_max_items: 2000,
            profile_ttl: 86400,
        },
        ui: config::UiConfig {
            enabled: true,
            dev_dir: None,
        },
        metrics: config::MetricsConfig { auth_token: None },
        logging: config::LoggingConfig {
            level: "info".to_string(),
            format: "pretty".to_string(),
        },
    };

    let state = AppState::new(app_config).await?;
    state.inbound_public_key_overrides.write().unwrap().insert(
        format!("{REMOTE_ACTOR_ID}#main-key"),
        TEST_PUBLIC_KEY_PEM.to_string(),
    );

    let listener = TcpListener::bind((host.as_str(), port)).await?;
    let local_addr = listener.local_addr()?;
    let app = build_router(state.clone());
    rustresort::federation::spawn_delivery_worker(state.clone());
    rustresort::service::spawn_scheduled_status_runner(state);

    println!(
        "ui-playwright-server listening on http://{}",
        socket_display(local_addr)
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn seed_account_if_needed(
    db_path: &std::path::Path,
    username: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use chrono::Utc;
    use rustresort::data::{Account, Database, EntityId};

    let db = Database::connect(db_path).await?;
    if db.get_account().await?.is_some() {
        return Ok(());
    }

    let now = Utc::now();
    let account = Account {
        id: EntityId::new_string(),
        username: username.to_string(),
        display_name: Some("Admin".to_string()),
        note: Some("Playwright admin account".to_string()),
        also_known_as: None,
        moved_to_uri: None,
        avatar_s3_key: None,
        header_s3_key: None,
        private_key_pem: TEST_PRIVATE_KEY_PEM.to_string(),
        public_key_pem: TEST_PUBLIC_KEY_PEM.to_string(),
        created_at: now,
        updated_at: now,
    };
    db.upsert_account(&account).await?;
    Ok(())
}

fn socket_display(addr: SocketAddr) -> String {
    format!("{}:{}", addr.ip(), addr.port())
}
