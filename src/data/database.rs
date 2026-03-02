//! SQLite database operations
//!
//! All database access goes through this module.
//! Uses SQLx for compile-time checked queries.

use chrono::{DateTime, Utc};
use sqlx::{Pool, QueryBuilder, Row, Sqlite, sqlite::SqlitePoolOptions};
use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use super::models::*;
use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct TursoSyncOptions {
    pub remote_url: String,
    pub auth_token: Option<String>,
}

#[cfg(feature = "turso-sync")]
type TursoSyncDatabase = turso::sync::Database;
#[cfg(not(feature = "turso-sync"))]
type TursoSyncDatabase = ();

#[cfg(feature = "turso-sync")]
fn map_turso_error(context: &str, error: turso::Error) -> AppError {
    AppError::internal(format!("{context}: {error}"))
}

/// Database connection pool wrapper.
///
/// # Turso synchronization
///
/// Dropping `Database` does not automatically perform a final Turso `push`/`pull`.
/// If callers need a final sync before shutdown, they should invoke
/// [`Database::sync_turso`] explicitly and handle any resulting errors.
pub struct Database {
    pool: Pool<Sqlite>,
    turso_sync_db: Option<TursoSyncDatabase>,
}
mod bootstrap;
mod helpers;
mod repository;

use bootstrap::{backfill_missing_status_hashtags, migrate_legacy_oauth_tokens};
use helpers::*;

impl Database {
    // =========================================================================
    // Connection
    // =========================================================================

    /// Connect to SQLite database
    ///
    /// Creates the database file if it doesn't exist.
    /// Runs pending migrations automatically.
    ///
    /// # Arguments
    /// * `path` - Path to SQLite database file
    ///
    /// # Errors
    /// Returns error if connection or migration fails
    pub async fn connect(path: &Path) -> Result<Self, AppError> {
        Self::connect_with_turso_sync(path, None).await
    }

    /// Connect to local Turso file database and optional Turso sync backend.
    pub async fn connect_with_turso_sync(
        path: &Path,
        sync: Option<TursoSyncOptions>,
    ) -> Result<Self, AppError> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::Database(sqlx::Error::Io(e)))?;
        }

        let turso_sync_db = Self::initialize_turso_sync_backend(path, sync).await?;

        // Create connection string
        let connection_string = format!("sqlite:{}?mode=rwc", path.display());

        // Create connection pool with WAL mode and explicit pool sizing
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query("PRAGMA journal_mode=WAL")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA busy_timeout=5000")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA foreign_keys=ON")
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(&connection_string)
            .await?;

        // Run migrations
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| {
                tracing::error!("Migration failed: {}", e);
                AppError::internal(format!("Migration failed: {}", e))
            })?;
        migrate_legacy_oauth_tokens(&pool).await?;
        backfill_missing_status_hashtags(&pool).await?;

        Self::push_migrations_to_turso_if_configured(&pool, &turso_sync_db).await?;

        tracing::info!("Database connected and migrated successfully");

        Ok(Self {
            pool,
            turso_sync_db,
        })
    }

    /// Return whether Turso sync backend is configured.
    pub fn has_turso_sync(&self) -> bool {
        self.turso_sync_db.is_some()
    }

    /// Sync local file DB with Turso remote.
    ///
    /// This performs `push` first (local writes), then `pull` (remote writes).
    #[cfg(feature = "turso-sync")]
    pub async fn sync_turso(&self) -> Result<(), AppError> {
        let started = Instant::now();
        let observe =
            |status: &str| crate::metrics::observe_db_sync("turso", status, started.elapsed());

        let Some(sync_db) = &self.turso_sync_db else {
            observe("skipped");
            return Ok(());
        };

        if let Err(error) = sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
            .execute(&self.pool)
            .await
        {
            observe("error");
            return Err(error.into());
        }

        if let Err(error) = sync_db.push().await {
            observe("error");
            return Err(map_turso_error("failed to push local DB to Turso", error));
        }
        if let Err(error) = sync_db.pull().await {
            observe("error");
            return Err(map_turso_error(
                "failed to pull remote DB from Turso",
                error,
            ));
        }

        observe("success");
        Ok(())
    }

    /// Sync local file DB with Turso remote.
    ///
    /// When built without `turso-sync`, this always returns a configuration error.
    #[cfg(not(feature = "turso-sync"))]
    pub async fn sync_turso(&self) -> Result<(), AppError> {
        let started = Instant::now();
        crate::metrics::observe_db_sync("turso", "error", started.elapsed());
        Err(AppError::Config(
            "database.sync.mode=turso requires building with the `turso-sync` feature".to_string(),
        ))
    }

    #[cfg(feature = "turso-sync")]
    async fn initialize_turso_sync_backend(
        path: &Path,
        sync: Option<TursoSyncOptions>,
    ) -> Result<Option<TursoSyncDatabase>, AppError> {
        let db_path = path.to_str().ok_or_else(|| {
            AppError::Config(format!(
                "database path must be valid UTF-8: {}",
                path.display()
            ))
        })?;

        // Initialize local file through Turso to ensure a Turso-compatible file DB.
        let local_turso_db = turso::Builder::new_local(db_path)
            .build()
            .await
            .map_err(|e| map_turso_error("failed to initialize local Turso file DB", e))?;
        drop(local_turso_db);

        if let Some(sync_options) = sync {
            let mut builder = turso::sync::Builder::new_remote(db_path)
                .with_remote_url(sync_options.remote_url)
                .bootstrap_if_empty(true);

            if let Some(token) = sync_options.auth_token {
                builder = builder.with_auth_token(token);
            }

            let sync_db = builder
                .build()
                .await
                .map_err(|e| map_turso_error("failed to initialize Turso sync database", e))?;

            sync_db
                .pull()
                .await
                .map_err(|e| map_turso_error("failed to pull from Turso sync database", e))?;

            Ok(Some(sync_db))
        } else {
            Ok(None)
        }
    }

    #[cfg(not(feature = "turso-sync"))]
    async fn initialize_turso_sync_backend(
        _path: &Path,
        sync: Option<TursoSyncOptions>,
    ) -> Result<Option<TursoSyncDatabase>, AppError> {
        if sync.is_some() {
            return Err(AppError::Config(
                "database.sync.mode=turso requires building with the `turso-sync` feature"
                    .to_string(),
            ));
        }
        Ok(None)
    }

    #[cfg(feature = "turso-sync")]
    async fn push_migrations_to_turso_if_configured(
        pool: &Pool<Sqlite>,
        turso_sync_db: &Option<TursoSyncDatabase>,
    ) -> Result<(), AppError> {
        if let Some(sync_db) = turso_sync_db {
            sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
                .execute(pool)
                .await?;
            sync_db
                .push()
                .await
                .map_err(|e| map_turso_error("failed to push migrations to Turso", e))?;
        }
        Ok(())
    }

    #[cfg(not(feature = "turso-sync"))]
    async fn push_migrations_to_turso_if_configured(
        _pool: &Pool<Sqlite>,
        _turso_sync_db: &Option<TursoSyncDatabase>,
    ) -> Result<(), AppError> {
        Ok(())
    }
}
