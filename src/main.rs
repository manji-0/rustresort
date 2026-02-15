//! RustResort binary entry point

use rustresort::{AppState, config};
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
    rustresort::api::init_metrics();

    // 3. Load configuration
    let config = config::AppConfig::load()?;
    tracing::info!(
        domain = %config.server.domain,
        protocol = %config.server.protocol,
        "Configuration loaded"
    );

    // 4. Initialize application state
    let state = AppState::new(config.clone()).await?;

    // 5. Build Axum router
    let app = build_router(state.clone());

    // 6. Start HTTP server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("Server listening on {}", addr);
    tracing::info!("Public URL: {}", config.server.base_url());

    // 7. Start background tasks
    if config.storage.backup.enabled {
        spawn_backup_task(state.clone());
    }

    // Start server
    axum::serve(listener, app).await?;

    Ok(())
}

/// Build the Axum router with all routes
fn build_router(state: AppState) -> axum::Router {
    use axum::Router;
    use tower_http::{compression::CompressionLayer, trace::TraceLayer};

    let cors_layer = build_cors_layer(&state.config.server);

    Router::new()
        // Health check endpoint
        .route("/health", axum::routing::get(health_check))
        // Auth endpoints
        .merge(rustresort::auth::auth_router())
        // Well-known endpoints
        .nest("/.well-known", rustresort::api::wellknown_router())
        // Mastodon API
        .nest("/api", rustresort::api::mastodon_api_router(state.clone()))
        // OAuth
        .nest("/oauth", rustresort::api::oauth_router())
        // ActivityPub
        .nest("/users", rustresort::api::activitypub_router())
        // Admin API
        .nest("/admin", rustresort::api::admin_router())
        // Middleware
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer)
        // State
        .with_state(state)
        // Metrics endpoint (Prometheus format) - stateless, added after state
        .merge(rustresort::api::metrics_router())
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

/// Health check endpoint
async fn health_check() -> &'static str {
    "OK"
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
