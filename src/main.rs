//! RustResort binary entry point

use rustresort::{AppState, config};
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Application entry point
///
/// # Setup
/// 1. Initialize tracing/logging
/// 2. Load configuration from file and environment
/// 3. Initialize AppState
/// 4. Build Axum router
/// 5. Start HTTP server
/// 6. Start background tasks (backup scheduler)
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialize tracing/logging
    let log_format =
        std::env::var("RUSTRESORT__LOGGING__FORMAT").unwrap_or_else(|_| "pretty".to_string());

    if log_format == "json" {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "rustresort=info,tower_http=debug".into()),
            )
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "rustresort=info,tower_http=debug".into()),
            )
            .with(tracing_subscriber::fmt::layer().pretty())
            .init();
    }

    tracing::info!("Starting RustResort...");

    // 2. Initialize metrics
    rustresort::metrics::init_metrics();

    // 3. Load configuration
    let mut config = config::AppConfig::load()?;
    apply_startup_flags(&mut config)?;
    tracing::info!(
        domain = %config.server.domain,
        protocol = %config.server.protocol,
        ui_enabled = config.ui.enabled,
        "Configuration loaded"
    );

    // 4. Initialize application state
    let state = AppState::new(config.clone()).await?;

    // 5. Build Axum router
    let app = rustresort::build_router(state.clone());

    // 6. Start HTTP server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("Server listening on {}", addr);
    tracing::info!("Public URL: {}", config.server.base_url());

    // 7. Start background tasks
    rustresort::federation::spawn_delivery_worker(state.clone());
    rustresort::service::spawn_scheduled_status_runner(state.clone());
    if config.storage.backup.enabled {
        spawn_backup_task(state.clone());
    }
    if config.database.sync.mode != config::DatabaseSyncMode::None {
        spawn_database_sync_task(state.clone());
    }

    // Start server
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

fn apply_startup_flags(config: &mut config::AppConfig) -> Result<(), Box<dyn std::error::Error>> {
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--enable-ui" => config.ui.enabled = true,
            "--disable-ui" => config.ui.enabled = false,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                return Err(format!("unknown startup flag: {other}").into());
            }
        }
    }

    Ok(())
}

fn print_help() {
    println!("rustresort");
    println!();
    println!("Startup flags:");
    println!("  --enable-ui   Enable the integrated Rust/WASM UI at /ui");
    println!("  --disable-ui  Disable the integrated Rust/WASM UI at /ui");
}

/// Spawn background backup task
fn spawn_backup_task(state: AppState) {
    tokio::spawn(async move {
        let interval_secs = state.config.storage.backup.interval_seconds;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));

        loop {
            interval.tick().await;

            tracing::info!("Running scheduled backup...");
            match state.backup.backup().await {
                Ok(key) => {
                    tracing::info!(backup_key = %key, "Backup completed successfully");
                }
                Err(e) => {
                    tracing::error!(error = %e, "Backup failed");
                }
            }
        }
    });

    tracing::info!("Backup task spawned");
}

/// Spawn background database sync task
fn spawn_database_sync_task(state: AppState) {
    let sync_mode = state.config.database.sync.mode.clone();
    if sync_mode == config::DatabaseSyncMode::None {
        tracing::debug!("Database sync mode is none; skipping background sync task spawn");
        return;
    }

    tokio::spawn(async move {
        let configured_interval_secs = state.config.database.sync.interval_seconds;
        let interval_secs = configured_interval_secs.max(1);
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));

        if configured_interval_secs == 0 {
            tracing::warn!("database.sync.interval_seconds=0 is invalid; clamped to 1 second");
        }

        // Consume the immediate first tick to delay initial sync until one interval passes.
        interval.tick().await;

        match sync_mode {
            config::DatabaseSyncMode::Turso => loop {
                interval.tick().await;

                tracing::info!("Running scheduled Turso sync...");
                match state.db.sync_turso().await {
                    Ok(()) => tracing::info!("Turso sync completed successfully"),
                    Err(error) => tracing::error!(%error, "Turso sync failed"),
                }
            },
            config::DatabaseSyncMode::D1 => loop {
                interval.tick().await;

                tracing::info!("Running scheduled Cloudflare D1 sync...");
                match rustresort::data::sync_to_d1(
                    &state.config.database.path,
                    &state.config.database.sync.d1,
                )
                .await
                {
                    Ok(()) => tracing::info!("Cloudflare D1 sync completed successfully"),
                    Err(error) => tracing::error!(%error, "Cloudflare D1 sync failed"),
                }
            },
            config::DatabaseSyncMode::None => {
                unreachable!("database sync task should not be spawned when mode is none")
            }
        }
    });

    tracing::info!("Database sync task spawned");
}
