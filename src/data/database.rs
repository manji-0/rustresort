//! SQLite database operations
//!
//! All database access goes through this module.
//! Uses SQLx for compile-time checked queries.

use chrono::{DateTime, Utc};
use sqlx::{Pool, QueryBuilder, Row, Sqlite, SqlitePool};
use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

use super::models::*;
use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct TursoSyncOptions {
    pub remote_url: String,
    pub auth_token: Option<String>,
}

fn map_turso_error(context: &str, error: turso::Error) -> AppError {
    AppError::Internal(anyhow::anyhow!("{context}: {error}"))
}

fn poll_is_expired(expires_at: &str, persisted_expired: i64) -> bool {
    if persisted_expired != 0 {
        return true;
    }

    DateTime::parse_from_rfc3339(expires_at)
        .map(|parsed| parsed.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true)
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
    turso_sync_db: Option<turso::sync::Database>,
}

fn parse_account_address(address: &str) -> Option<(String, String, Option<u16>)> {
    let (username, authority) = address.split_once('@')?;
    let parsed = url::Url::parse(&format!("http://{}", authority)).ok()?;
    let host = parsed.host_str()?;
    Some((
        username.to_ascii_lowercase(),
        host.to_ascii_lowercase(),
        extract_explicit_port(authority),
    ))
}

fn extract_explicit_port(authority: &str) -> Option<u16> {
    let authority = authority.trim();

    if let Some(rest) = authority.strip_prefix('[') {
        let (_, tail) = rest.split_once(']')?;
        let port_str = tail.strip_prefix(':')?;
        if port_str.is_empty() || !port_str.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        return port_str.parse::<u16>().ok();
    }

    let (host_part, port_str) = authority.rsplit_once(':')?;
    if host_part.is_empty()
        || host_part.contains(':')
        || port_str.is_empty()
        || !port_str.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }

    port_str.parse::<u16>().ok()
}

fn format_host_for_authority(host: &str) -> String {
    if host.contains(':') {
        format!("[{}]", host)
    } else {
        host.to_string()
    }
}

fn push_case_insensitive_unique(
    values: &mut Vec<String>,
    seen_casefold: &mut HashSet<String>,
    candidate: String,
) {
    if !seen_casefold.insert(candidate.to_ascii_lowercase()) {
        return;
    }
    values.push(candidate);
}

fn equivalent_account_address_candidates(
    target_address: &str,
    default_port: Option<u16>,
) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut seen_casefold = HashSet::new();
    push_case_insensitive_unique(
        &mut candidates,
        &mut seen_casefold,
        target_address.to_string(),
    );

    let Some((username, host, explicit_port)) = parse_account_address(target_address) else {
        return candidates;
    };
    let authority = format_host_for_authority(&host);
    let without_port = format!("{}@{}", username, authority);

    if let Some(port) = explicit_port {
        push_case_insensitive_unique(
            &mut candidates,
            &mut seen_casefold,
            format!("{}@{}:{}", username, authority, port),
        );

        if default_port == Some(port) {
            push_case_insensitive_unique(&mut candidates, &mut seen_casefold, without_port);
        }
    } else {
        push_case_insensitive_unique(&mut candidates, &mut seen_casefold, without_port);

        if let Some(default_port) = default_port {
            push_case_insensitive_unique(
                &mut candidates,
                &mut seen_casefold,
                format!("{}@{}:{}", username, authority, default_port),
            );
        }
    }

    candidates
}

fn account_addresses_match(left: &str, right: &str, default_port: Option<u16>) -> bool {
    let Some((left_user, left_host, left_port)) = parse_account_address(left) else {
        return left.eq_ignore_ascii_case(right);
    };
    let Some((right_user, right_host, right_port)) = parse_account_address(right) else {
        return left.eq_ignore_ascii_case(right);
    };

    if left_user != right_user || left_host != right_host {
        return false;
    }

    match default_port {
        Some(port) => left_port.unwrap_or(port) == right_port.unwrap_or(port),
        None => left_port == right_port,
    }
}

fn find_matching_addresses(
    candidates: &[String],
    target: &str,
    default_port: Option<u16>,
) -> Vec<String> {
    candidates
        .iter()
        .filter(|candidate| account_addresses_match(candidate, target, default_port))
        .cloned()
        .collect()
}

fn parse_json_value(raw: Option<String>) -> Option<serde_json::Value> {
    raw.and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
}

const OAUTH_ACCESS_TOKEN_HASH_PREFIX: &str = "sha256:";
const OAUTH_ACCESS_TOKEN_HASH_ENCODED_LEN: usize = 43;
const OAUTH_ACCESS_TOKEN_HASH_DECODED_LEN: usize = 32;
const OAUTH_ACCESS_TOKEN_HASH_MIGRATION_SETTING_KEY: &str =
    "oauth_tokens_access_token_hash_migration";
const OAUTH_ACCESS_TOKEN_HASH_MIGRATION_DONE: &str = "done";

fn hash_oauth_access_token(access_token: &str) -> String {
    let digest = Sha256::digest(access_token.as_bytes());
    format!(
        "{}{}",
        OAUTH_ACCESS_TOKEN_HASH_PREFIX,
        URL_SAFE_NO_PAD.encode(digest)
    )
}

fn is_hashed_oauth_access_token(stored_access_token: &str) -> bool {
    let Some(encoded_digest) = stored_access_token.strip_prefix(OAUTH_ACCESS_TOKEN_HASH_PREFIX)
    else {
        return false;
    };

    if encoded_digest.len() != OAUTH_ACCESS_TOKEN_HASH_ENCODED_LEN
        || !encoded_digest
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return false;
    }

    URL_SAFE_NO_PAD
        .decode(encoded_digest)
        .map(|bytes| bytes.len() == OAUTH_ACCESS_TOKEN_HASH_DECODED_LEN)
        .unwrap_or(false)
}

async fn migrate_legacy_oauth_tokens(pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let migration_state =
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
            .bind(OAUTH_ACCESS_TOKEN_HASH_MIGRATION_SETTING_KEY)
            .fetch_optional(pool)
            .await?;

    if migration_state.as_deref() == Some(OAUTH_ACCESS_TOKEN_HASH_MIGRATION_DONE) {
        return Ok(());
    }

    let legacy_rows =
        sqlx::query_as::<_, (String, String)>("SELECT id, access_token FROM oauth_tokens")
            .fetch_all(pool)
            .await?;

    if legacy_rows.is_empty() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    let mut migrated_count = 0usize;

    for (id, stored_access_token) in legacy_rows {
        if is_hashed_oauth_access_token(&stored_access_token) {
            continue;
        }

        let hashed_access_token = hash_oauth_access_token(&stored_access_token);
        sqlx::query("UPDATE oauth_tokens SET access_token = ? WHERE id = ?")
            .bind(&hashed_access_token)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        migrated_count += 1;
    }

    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(OAUTH_ACCESS_TOKEN_HASH_MIGRATION_SETTING_KEY)
    .bind(OAUTH_ACCESS_TOKEN_HASH_MIGRATION_DONE)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    if migrated_count > 0 {
        tracing::info!(
            migrated_count,
            "Migrated legacy OAuth access tokens to hashed storage"
        );
    }

    Ok(())
}

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

        // Optional Turso sync setup.
        let turso_sync_db = if let Some(sync_options) = sync {
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

            Some(sync_db)
        } else {
            None
        };

        // Create connection string
        let connection_string = format!("sqlite:{}?mode=rwc", path.display());

        // Create connection pool
        let pool = SqlitePool::connect(&connection_string).await?;

        // Run migrations
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| {
                tracing::error!("Migration failed: {}", e);
                AppError::Internal(anyhow::anyhow!("Migration failed: {}", e))
            })?;
        migrate_legacy_oauth_tokens(&pool).await?;

        if let Some(sync_db) = &turso_sync_db {
            sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
                .execute(&pool)
                .await?;
            sync_db
                .push()
                .await
                .map_err(|e| map_turso_error("failed to push migrations to Turso", e))?;
        }

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

    // =========================================================================
    // Account (single user)
    // =========================================================================

    /// Get the single admin account
    ///
    /// # Returns
    /// The account or None if not initialized
    pub async fn get_account(&self) -> Result<Option<Account>, AppError> {
        let account = sqlx::query_as::<_, Account>("SELECT * FROM account LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;

        Ok(account)
    }

    /// Create or update the admin account
    ///
    /// # Arguments
    /// * `account` - Account data to upsert
    pub async fn upsert_account(&self, account: &Account) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO account (
                id, username, display_name, note, avatar_s3_key, header_s3_key,
                private_key_pem, public_key_pem, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&account.id)
        .bind(&account.username)
        .bind(&account.display_name)
        .bind(&account.note)
        .bind(&account.avatar_s3_key)
        .bind(&account.header_s3_key)
        .bind(&account.private_key_pem)
        .bind(&account.public_key_pem)
        .bind(&account.created_at)
        .bind(&account.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Insert the admin account only when the table is empty.
    ///
    /// This is atomic at the SQL statement level and prevents races where
    /// multiple initializers try to create the first account concurrently.
    ///
    /// # Returns
    /// `true` if inserted, `false` if an account already existed.
    pub async fn insert_account_if_empty(&self, account: &Account) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            INSERT INTO account (
                id, username, display_name, note, avatar_s3_key, header_s3_key,
                private_key_pem, public_key_pem, created_at, updated_at
            )
            SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
            WHERE NOT EXISTS (SELECT 1 FROM account)
            "#,
        )
        .bind(&account.id)
        .bind(&account.username)
        .bind(&account.display_name)
        .bind(&account.note)
        .bind(&account.avatar_s3_key)
        .bind(&account.header_s3_key)
        .bind(&account.private_key_pem)
        .bind(&account.public_key_pem)
        .bind(&account.created_at)
        .bind(&account.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Update account profile fields by account ID.
    ///
    /// # Returns
    /// `true` if updated, `false` if no matching account row exists.
    pub async fn update_account_profile(
        &self,
        account_id: &str,
        display_name: Option<&str>,
        note: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE account
            SET display_name = ?, note = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(display_name)
        .bind(note)
        .bind(updated_at)
        .bind(account_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Patch account profile fields by account ID.
    ///
    /// Use `None` for omitted fields (no change), and `Some(None)` to clear a field.
    ///
    /// # Returns
    /// `true` if updated, `false` if no matching account row exists.
    pub async fn patch_account_profile(
        &self,
        account_id: &str,
        display_name: Option<Option<&str>>,
        note: Option<Option<&str>>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError> {
        let result = match (display_name, note) {
            (Some(display_name), Some(note)) => {
                sqlx::query(
                    r#"
                    UPDATE account
                    SET display_name = ?, note = ?, updated_at = ?
                    WHERE id = ?
                    "#,
                )
                .bind(display_name)
                .bind(note)
                .bind(updated_at)
                .bind(account_id)
                .execute(&self.pool)
                .await?
            }
            (Some(display_name), None) => {
                sqlx::query(
                    r#"
                    UPDATE account
                    SET display_name = ?, updated_at = ?
                    WHERE id = ?
                    "#,
                )
                .bind(display_name)
                .bind(updated_at)
                .bind(account_id)
                .execute(&self.pool)
                .await?
            }
            (None, Some(note)) => {
                sqlx::query(
                    r#"
                    UPDATE account
                    SET note = ?, updated_at = ?
                    WHERE id = ?
                    "#,
                )
                .bind(note)
                .bind(updated_at)
                .bind(account_id)
                .execute(&self.pool)
                .await?
            }
            // Treat a no-op patch as success. Callers can still decide
            // whether they want to skip calling this API beforehand.
            (None, None) => return Ok(true),
        };

        Ok(result.rows_affected() == 1)
    }

    /// Update account avatar key by account ID.
    ///
    /// # Returns
    /// `true` if updated, `false` if no matching account row exists.
    pub async fn update_account_avatar_key_if_matches(
        &self,
        account_id: &str,
        expected_current_avatar_s3_key: Option<&str>,
        avatar_s3_key: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE account
            SET avatar_s3_key = ?, updated_at = ?
            WHERE id = ? AND avatar_s3_key IS ?
            "#,
        )
        .bind(avatar_s3_key)
        .bind(updated_at)
        .bind(account_id)
        .bind(expected_current_avatar_s3_key)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Update account header key by account ID.
    ///
    /// # Returns
    /// `true` if updated, `false` if no matching account row exists.
    pub async fn update_account_header_key_if_matches(
        &self,
        account_id: &str,
        expected_current_header_s3_key: Option<&str>,
        header_s3_key: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE account
            SET header_s3_key = ?, updated_at = ?
            WHERE id = ? AND header_s3_key IS ?
            "#,
        )
        .bind(header_s3_key)
        .bind(updated_at)
        .bind(account_id)
        .bind(expected_current_header_s3_key)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    // =========================================================================
    // Status
    // =========================================================================

    /// Get status by ID
    pub async fn get_status(&self, id: &str) -> Result<Option<Status>, AppError> {
        let status = sqlx::query_as::<_, Status>("SELECT * FROM statuses WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(status)
    }

    /// Get status by ActivityPub URI
    pub async fn get_status_by_uri(&self, uri: &str) -> Result<Option<Status>, AppError> {
        let status = sqlx::query_as::<_, Status>("SELECT * FROM statuses WHERE uri = ?")
            .bind(uri)
            .fetch_optional(&self.pool)
            .await?;

        Ok(status)
    }

    /// Get multiple statuses by URIs (batch operation to avoid N+1)
    pub async fn get_statuses_by_uris(&self, uris: &[String]) -> Result<Vec<Status>, AppError> {
        if uris.is_empty() {
            return Ok(vec![]);
        }

        // SQLiteのIN句には制限があるため、チャンク化して処理
        let mut all_statuses = Vec::new();

        for chunk in uris.chunks(100) {
            let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");

            let query = format!("SELECT * FROM statuses WHERE uri IN ({})", placeholders);

            let mut query_builder = sqlx::query_as::<_, Status>(&query);
            for uri in chunk {
                query_builder = query_builder.bind(uri);
            }

            let statuses = query_builder.fetch_all(&self.pool).await?;
            all_statuses.extend(statuses);
        }

        Ok(all_statuses)
    }

    /// Insert a new status
    pub async fn insert_status(&self, status: &Status) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO statuses (
                id, uri, content, content_warning, visibility, language,
                account_address, is_local, in_reply_to_uri, boost_of_uri,
                persisted_reason, created_at, fetched_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&status.id)
        .bind(&status.uri)
        .bind(&status.content)
        .bind(&status.content_warning)
        .bind(&status.visibility)
        .bind(&status.language)
        .bind(&status.account_address)
        .bind(status.is_local)
        .bind(&status.in_reply_to_uri)
        .bind(&status.boost_of_uri)
        .bind(&status.persisted_reason)
        .bind(&status.created_at)
        .bind(&status.fetched_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Insert a new status and attach media atomically.
    pub async fn insert_status_with_media(
        &self,
        status: &Status,
        media_ids: &[String],
    ) -> Result<(), AppError> {
        self.insert_status_with_media_and_poll(status, media_ids, None)
            .await
    }

    /// Insert a new status with optional media and poll atomically.
    pub async fn insert_status_with_media_and_poll(
        &self,
        status: &Status,
        media_ids: &[String],
        poll: Option<(&[String], i64, bool)>,
    ) -> Result<(), AppError> {
        if media_ids.is_empty() && poll.is_none() {
            return self.insert_status(status).await;
        }

        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            sqlx::query(
                r#"
                INSERT INTO statuses (
                    id, uri, content, content_warning, visibility, language,
                    account_address, is_local, in_reply_to_uri, boost_of_uri,
                    persisted_reason, created_at, fetched_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&status.id)
            .bind(&status.uri)
            .bind(&status.content)
            .bind(&status.content_warning)
            .bind(&status.visibility)
            .bind(&status.language)
            .bind(&status.account_address)
            .bind(status.is_local)
            .bind(&status.in_reply_to_uri)
            .bind(&status.boost_of_uri)
            .bind(&status.persisted_reason)
            .bind(&status.created_at)
            .bind(&status.fetched_at)
            .execute(&mut *conn)
            .await?;

            for media_id in media_ids {
                let updated = sqlx::query(
                    "UPDATE media_attachments SET status_id = ? WHERE id = ? AND status_id IS NULL",
                )
                .bind(&status.id)
                .bind(media_id)
                .execute(&mut *conn)
                .await?;

                if updated.rows_affected() == 0 {
                    return Err(AppError::Validation(format!(
                        "media attachment is unavailable: {}",
                        media_id
                    )));
                }
            }

            if let Some((poll_options, expires_in, multiple)) = poll {
                let poll_id = EntityId::new().0;
                let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in);
                sqlx::query(
                    r#"
                    INSERT INTO polls (id, status_id, expires_at, expired, multiple, votes_count, voters_count, created_at)
                    VALUES (?, ?, ?, 0, ?, 0, 0, datetime('now'))
                    "#,
                )
                .bind(&poll_id)
                .bind(&status.id)
                .bind(expires_at.to_rfc3339())
                .bind(multiple as i64)
                .execute(&mut *conn)
                .await?;

                for (index, option) in poll_options.iter().enumerate() {
                    let option_id = EntityId::new().0;
                    sqlx::query(
                        r#"
                        INSERT INTO poll_options (id, poll_id, title, votes_count, option_index, created_at)
                        VALUES (?, ?, ?, 0, ?, datetime('now'))
                        "#,
                    )
                    .bind(&option_id)
                    .bind(&poll_id)
                    .bind(option)
                    .bind(index as i64)
                    .execute(&mut *conn)
                    .await?;
                }
            }

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }

    /// Update an existing status
    pub async fn update_status(&self, status: &Status) -> Result<(), AppError> {
        sqlx::query(
            r#"
            UPDATE statuses
            SET content = ?, content_warning = ?, visibility = ?, language = ?,
                in_reply_to_uri = ?, boost_of_uri = ?, persisted_reason = ?, fetched_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&status.content)
        .bind(&status.content_warning)
        .bind(&status.visibility)
        .bind(&status.language)
        .bind(&status.in_reply_to_uri)
        .bind(&status.boost_of_uri)
        .bind(&status.persisted_reason)
        .bind(&status.fetched_at)
        .bind(&status.id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete status by ID
    pub async fn delete_status(&self, id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM statuses WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get a cached idempotency response for an endpoint and key.
    pub async fn get_idempotency_response(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<Option<serde_json::Value>, AppError> {
        let response_json = sqlx::query_scalar::<_, Option<String>>(
            "SELECT response_json FROM idempotency_keys WHERE endpoint = ? AND key = ?",
        )
        .bind(endpoint)
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await?
        .flatten();

        response_json
            .map(|raw| {
                serde_json::from_str::<serde_json::Value>(&raw).map_err(|error| {
                    AppError::Internal(anyhow::anyhow!(
                        "failed to deserialize idempotency response: {error}"
                    ))
                })
            })
            .transpose()
    }

    /// Try to reserve an idempotency key for processing.
    ///
    /// Returns `true` when this request successfully reserved the key and should
    /// proceed, or `false` when another request already owns/owned the key.
    pub async fn reserve_idempotency_key(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            INSERT INTO idempotency_keys (endpoint, key, response_json, created_at)
            VALUES (?, ?, NULL, datetime('now'))
            ON CONFLICT(endpoint, key) DO UPDATE
            SET response_json = NULL, created_at = datetime('now')
            WHERE idempotency_keys.response_json IS NULL
              AND idempotency_keys.created_at < datetime('now', '-5 minutes')
            "#,
        )
        .bind(endpoint)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Store an idempotency response payload for an endpoint and key.
    pub async fn store_idempotency_response(
        &self,
        endpoint: &str,
        idempotency_key: &str,
        response: &serde_json::Value,
    ) -> Result<(), AppError> {
        let response_json = serde_json::to_string(response).map_err(|error| {
            AppError::Internal(anyhow::anyhow!(
                "failed to serialize idempotency response: {error}"
            ))
        })?;

        let result = sqlx::query(
            "UPDATE idempotency_keys SET response_json = ? WHERE endpoint = ? AND key = ?",
        )
        .bind(&response_json)
        .bind(endpoint)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            sqlx::query(
                "INSERT OR IGNORE INTO idempotency_keys (endpoint, key, response_json) VALUES (?, ?, ?)",
            )
            .bind(endpoint)
            .bind(idempotency_key)
            .bind(&response_json)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Delete a pending idempotency reservation with no stored response.
    pub async fn clear_pending_idempotency_key(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<(), AppError> {
        sqlx::query(
            "DELETE FROM idempotency_keys WHERE endpoint = ? AND key = ? AND response_json IS NULL",
        )
        .bind(endpoint)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn backdate_pending_idempotency_key_for_test(
        &self,
        endpoint: &str,
        idempotency_key: &str,
        minutes: i64,
    ) -> Result<(), AppError> {
        let modifier = format!("-{} minutes", minutes);
        sqlx::query(
            "UPDATE idempotency_keys SET created_at = datetime('now', ?) WHERE endpoint = ? AND key = ? AND response_json IS NULL",
        )
        .bind(modifier)
        .bind(endpoint)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get user's own statuses (paginated)
    ///
    /// # Arguments
    /// * `limit` - Maximum number of results
    /// * `max_id` - Return statuses older than this ID (for pagination)
    pub async fn get_local_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let statuses = if let Some(max_id) = max_id {
            sqlx::query_as::<_, Status>(
                r#"
                SELECT * FROM statuses 
                WHERE is_local = 1 AND id < ?
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(max_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, Status>(
                r#"
                SELECT * FROM statuses 
                WHERE is_local = 1
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(statuses)
    }

    /// Get user's own statuses (paginated) with optional min/max ID window
    ///
    /// # Arguments
    /// * `limit` - Maximum number of results
    /// * `max_id` - Return statuses older than this ID (exclusive)
    /// * `min_id` - Return statuses newer than this ID (exclusive)
    pub async fn get_local_statuses_in_window(
        &self,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let statuses = match (max_id, min_id) {
            (Some(max_id), Some(min_id)) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT * FROM statuses 
                    WHERE is_local = 1 AND id < ? AND id > ?
                    ORDER BY created_at DESC
                    LIMIT ?
                    "#,
                )
                .bind(max_id)
                .bind(min_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (Some(max_id), None) => self.get_local_statuses(limit, Some(max_id)).await?,
            (None, Some(min_id)) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT * FROM statuses 
                    WHERE is_local = 1 AND id > ?
                    ORDER BY created_at DESC
                    LIMIT ?
                    "#,
                )
                .bind(min_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, None) => self.get_local_statuses(limit, None).await?,
        };

        Ok(statuses)
    }

    /// Get user's own public statuses (paginated)
    ///
    /// # Arguments
    /// * `limit` - Maximum number of results
    /// * `max_id` - Return statuses older than this ID (for pagination)
    pub async fn get_local_public_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let statuses = if let Some(max_id) = max_id {
            sqlx::query_as::<_, Status>(
                r#"
                SELECT * FROM statuses 
                WHERE is_local = 1 AND visibility = 'public' AND id < ?
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(max_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, Status>(
                r#"
                SELECT * FROM statuses 
                WHERE is_local = 1 AND visibility = 'public'
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(statuses)
    }

    /// Get statuses safe to expose in ActivityPub outbox.
    ///
    /// Outbox must never leak private/direct statuses.
    pub async fn get_local_outbox_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let statuses = if let Some(max_id) = max_id {
            sqlx::query_as::<_, Status>(
                r#"
                SELECT * FROM statuses
                WHERE is_local = 1 AND visibility IN ('public', 'unlisted') AND id < ?
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(max_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, Status>(
                r#"
                SELECT * FROM statuses
                WHERE is_local = 1 AND visibility IN ('public', 'unlisted')
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(statuses)
    }

    // =========================================================================
    // Media Attachments
    // =========================================================================

    /// Insert media attachment
    pub async fn insert_media(&self, media: &MediaAttachment) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO media_attachments (
                id, status_id, s3_key, thumbnail_s3_key, content_type,
                file_size, description, blurhash, width, height, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&media.id)
        .bind(&media.status_id)
        .bind(&media.s3_key)
        .bind(&media.thumbnail_s3_key)
        .bind(&media.content_type)
        .bind(media.file_size)
        .bind(&media.description)
        .bind(&media.blurhash)
        .bind(media.width)
        .bind(media.height)
        .bind(&media.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get media by status ID
    pub async fn get_media_by_status(
        &self,
        status_id: &str,
    ) -> Result<Vec<MediaAttachment>, AppError> {
        let media = sqlx::query_as::<_, MediaAttachment>(
            "SELECT * FROM media_attachments WHERE status_id = ?",
        )
        .bind(status_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(media)
    }

    /// Attach media to status
    pub async fn attach_media_to_status(
        &self,
        media_id: &str,
        status_id: &str,
    ) -> Result<(), AppError> {
        let result = sqlx::query(
            "UPDATE media_attachments SET status_id = ? WHERE id = ? AND (status_id IS NULL OR status_id = ?)",
        )
        .bind(status_id)
        .bind(media_id)
        .bind(status_id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(AppError::Validation(format!(
                "media attachment is already attached to another status: {}",
                media_id
            )));
        }

        Ok(())
    }

    /// Get media by ID
    pub async fn get_media(&self, id: &str) -> Result<Option<MediaAttachment>, AppError> {
        let media =
            sqlx::query_as::<_, MediaAttachment>("SELECT * FROM media_attachments WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;

        Ok(media)
    }

    /// Update media attachment
    pub async fn update_media(&self, media: &MediaAttachment) -> Result<(), AppError> {
        sqlx::query(
            r#"
            UPDATE media_attachments 
            SET description = ?, blurhash = ?, width = ?, height = ?
            WHERE id = ?
            "#,
        )
        .bind(&media.description)
        .bind(&media.blurhash)
        .bind(media.width)
        .bind(media.height)
        .bind(&media.id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // =========================================================================
    // Follow relationships
    // =========================================================================

    /// Get all follow addresses
    ///
    /// # Returns
    /// List of addresses (user@domain) that the user follows
    pub async fn get_all_follow_addresses(&self) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT target_address FROM follows ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    /// Count follows.
    pub async fn count_follow_addresses(&self) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM follows")
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    /// Get all follower addresses
    ///
    /// # Returns
    /// List of addresses (user@domain) that follow the user
    pub async fn get_all_follower_addresses(&self) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT follower_address FROM followers ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    /// Count followers.
    pub async fn count_follower_addresses(&self) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM followers")
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    /// Get follower inbox URIs for activity delivery
    pub async fn get_follower_inboxes(&self) -> Result<Vec<String>, AppError> {
        let inboxes = sqlx::query_scalar::<_, String>("SELECT DISTINCT inbox_uri FROM followers")
            .fetch_all(&self.pool)
            .await?;

        Ok(inboxes)
    }

    /// Insert new follow relationship
    pub async fn insert_follow(&self, follow: &Follow) -> Result<(), AppError> {
        sqlx::query(
            "INSERT OR IGNORE INTO follows (id, target_address, uri, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&follow.id)
        .bind(&follow.target_address)
        .bind(&follow.uri)
        .bind(&follow.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Insert follow relationship when no equivalent address exists.
    ///
    /// Uses an IMMEDIATE transaction so the equivalence check and insert are atomic.
    pub async fn insert_follow_if_absent(
        &self,
        follow: &Follow,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<bool, AppError> = async {
            let existing_addresses =
                sqlx::query_scalar::<_, String>("SELECT target_address FROM follows")
                    .fetch_all(&mut *conn)
                    .await?;
            if existing_addresses
                .iter()
                .any(|existing| account_addresses_match(existing, &follow.target_address, default_port))
            {
                return Ok(false);
            }

            let inserted = sqlx::query(
                "INSERT OR IGNORE INTO follows (id, target_address, uri, created_at) VALUES (?, ?, ?, ?)",
            )
            .bind(&follow.id)
            .bind(&follow.target_address)
            .bind(&follow.uri)
            .bind(&follow.created_at)
            .execute(&mut *conn)
            .await?;

            Ok(inserted.rows_affected() > 0)
        }
        .await;

        match result {
            Ok(inserted) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(inserted)
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }

    /// Get Follow activity URI for a target address.
    pub async fn get_follow_uri(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<Option<String>, AppError> {
        let candidates = equivalent_account_address_candidates(target_address, default_port);
        if candidates.is_empty() {
            return Ok(None);
        }

        let mut query_builder = QueryBuilder::<Sqlite>::new(
            "SELECT uri FROM follows WHERE target_address COLLATE NOCASE IN (",
        );
        {
            let mut separated = query_builder.separated(", ");
            for candidate in &candidates {
                separated.push_bind(candidate);
            }
        }
        query_builder.push(") ORDER BY created_at DESC LIMIT 1");

        let uri = query_builder
            .build_query_scalar::<String>()
            .fetch_optional(&self.pool)
            .await?;

        Ok(uri)
    }

    /// Delete follow relationship
    pub async fn delete_follow(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<(), AppError> {
        let existing_addresses = self.get_all_follow_addresses().await?;
        let matches = find_matching_addresses(&existing_addresses, target_address, default_port);
        for existing in matches {
            sqlx::query("DELETE FROM follows WHERE target_address COLLATE NOCASE = ?")
                .bind(existing)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    /// Insert new follower
    pub async fn insert_follower(&self, follower: &Follower) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO followers (id, follower_address, inbox_uri, uri, created_at) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&follower.id)
        .bind(&follower.follower_address)
        .bind(&follower.inbox_uri)
        .bind(&follower.uri)
        .bind(&follower.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete follower
    pub async fn delete_follower(
        &self,
        follower_address: &str,
        default_port: Option<u16>,
    ) -> Result<(), AppError> {
        let existing_addresses = self.get_all_follower_addresses().await?;
        let matches = find_matching_addresses(&existing_addresses, follower_address, default_port);
        for existing in matches {
            sqlx::query("DELETE FROM followers WHERE follower_address COLLATE NOCASE = ?")
                .bind(existing)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    /// Delete follower by follower address and Follow activity URI
    pub async fn delete_follower_by_address_and_uri(
        &self,
        follower_address: &str,
        follow_uri: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let existing_addresses = self.get_all_follower_addresses().await?;
        let matches = find_matching_addresses(&existing_addresses, follower_address, default_port);
        let mut removed = false;
        for existing in matches {
            let result = sqlx::query(
                "DELETE FROM followers WHERE follower_address COLLATE NOCASE = ? AND uri = ?",
            )
            .bind(existing)
            .bind(follow_uri)
            .execute(&self.pool)
            .await?;
            removed |= result.rows_affected() > 0;
        }

        Ok(removed)
    }

    // =========================================================================
    // Notifications
    // =========================================================================

    /// Insert notification
    pub async fn insert_notification(&self, notification: &Notification) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO notifications (
                id, notification_type, origin_account_address, status_uri, read, created_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&notification.id)
        .bind(&notification.notification_type)
        .bind(&notification.origin_account_address)
        .bind(&notification.status_uri)
        .bind(notification.read)
        .bind(&notification.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get notifications (paginated)
    pub async fn get_notifications(
        &self,
        limit: usize,
        max_id: Option<&str>,
        unread_only: bool,
    ) -> Result<Vec<Notification>, AppError> {
        let notifications = match (max_id, unread_only) {
            (Some(max_id), true) => {
                sqlx::query_as::<_, Notification>(
                    "SELECT * FROM notifications WHERE id < ? AND read = 0 ORDER BY created_at DESC LIMIT ?"
                )
                .bind(max_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (Some(max_id), false) => {
                sqlx::query_as::<_, Notification>(
                    "SELECT * FROM notifications WHERE id < ? ORDER BY created_at DESC LIMIT ?"
                )
                .bind(max_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, true) => {
                sqlx::query_as::<_, Notification>(
                    "SELECT * FROM notifications WHERE read = 0 ORDER BY created_at DESC LIMIT ?"
                )
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, false) => {
                sqlx::query_as::<_, Notification>(
                    "SELECT * FROM notifications ORDER BY created_at DESC LIMIT ?"
                )
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(notifications)
    }

    /// Mark notification as read
    pub async fn mark_notification_read(&self, id: &str) -> Result<(), AppError> {
        sqlx::query("UPDATE notifications SET read = 1 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get a single notification by ID
    pub async fn get_notification(&self, id: &str) -> Result<Option<Notification>, AppError> {
        let notification =
            sqlx::query_as::<_, Notification>("SELECT * FROM notifications WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;

        Ok(notification)
    }

    /// Mark all notifications as read
    pub async fn mark_all_notifications_read(&self) -> Result<(), AppError> {
        sqlx::query("UPDATE notifications SET read = 1")
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    // =========================================================================
    // Favourites / Bookmarks / Reposts
    // =========================================================================

    /// Insert favourite
    pub async fn insert_favourite(&self, status_id: &str) -> Result<String, AppError> {
        let id = EntityId::new().0;
        sqlx::query(
            "INSERT INTO favourites (id, status_id, created_at) VALUES (?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(status_id)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    #[cfg(test)]
    pub async fn set_favourite_created_at_for_test(
        &self,
        status_id: &str,
        created_at: &str,
    ) -> Result<(), AppError> {
        sqlx::query("UPDATE favourites SET created_at = ? WHERE status_id = ?")
            .bind(created_at)
            .bind(status_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Delete favourite
    pub async fn delete_favourite(&self, status_id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM favourites WHERE status_id = ?")
            .bind(status_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get favourite record ID for a status.
    pub async fn get_favourite_id(&self, status_id: &str) -> Result<Option<String>, AppError> {
        let id = sqlx::query_scalar::<_, String>(
            "SELECT id FROM favourites WHERE status_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(status_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(id)
    }

    /// Check if status is favourited
    pub async fn is_favourited(&self, status_id: &str) -> Result<bool, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM favourites WHERE status_id = ?")
            .bind(status_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(count > 0)
    }

    /// Get favourited status IDs
    pub async fn get_favourited_status_ids(&self, limit: usize) -> Result<Vec<String>, AppError> {
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT status_id FROM favourites ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(ids)
    }

    /// Get favourited status IDs among the provided IDs
    pub async fn get_favourited_status_ids_batch(
        &self,
        status_ids: &[String],
    ) -> Result<HashSet<String>, AppError> {
        if status_ids.is_empty() {
            return Ok(HashSet::new());
        }

        let mut query_builder =
            QueryBuilder::<Sqlite>::new("SELECT status_id FROM favourites WHERE status_id IN (");
        {
            let mut separated = query_builder.separated(", ");
            for status_id in status_ids {
                separated.push_bind(status_id);
            }
        }
        query_builder.push(")");

        let ids = query_builder
            .build_query_scalar::<String>()
            .fetch_all(&self.pool)
            .await?;

        Ok(ids.into_iter().collect())
    }

    /// Insert bookmark
    pub async fn insert_bookmark(&self, status_id: &str) -> Result<String, AppError> {
        let id = EntityId::new().0;
        sqlx::query(
            "INSERT INTO bookmarks (id, status_id, created_at) VALUES (?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(status_id)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    #[cfg(test)]
    pub async fn set_bookmark_created_at_for_test(
        &self,
        status_id: &str,
        created_at: &str,
    ) -> Result<(), AppError> {
        sqlx::query("UPDATE bookmarks SET created_at = ? WHERE status_id = ?")
            .bind(created_at)
            .bind(status_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Delete bookmark
    pub async fn delete_bookmark(&self, status_id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM bookmarks WHERE status_id = ?")
            .bind(status_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Check if status is bookmarked
    pub async fn is_bookmarked(&self, status_id: &str) -> Result<bool, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookmarks WHERE status_id = ?")
            .bind(status_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(count > 0)
    }

    /// Get bookmarked status IDs
    pub async fn get_bookmarked_status_ids(&self, limit: usize) -> Result<Vec<String>, AppError> {
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT status_id FROM bookmarks ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(ids)
    }

    /// Get bookmarked status IDs among the provided IDs
    pub async fn get_bookmarked_status_ids_batch(
        &self,
        status_ids: &[String],
    ) -> Result<HashSet<String>, AppError> {
        if status_ids.is_empty() {
            return Ok(HashSet::new());
        }

        let mut query_builder =
            QueryBuilder::<Sqlite>::new("SELECT status_id FROM bookmarks WHERE status_id IN (");
        {
            let mut separated = query_builder.separated(", ");
            for status_id in status_ids {
                separated.push_bind(status_id);
            }
        }
        query_builder.push(")");

        let ids = query_builder
            .build_query_scalar::<String>()
            .fetch_all(&self.pool)
            .await?;

        Ok(ids.into_iter().collect())
    }

    /// Get bookmarked statuses with JOIN (optimized, avoids N+1)
    pub async fn get_bookmarked_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let statuses = match max_id {
            Some(max_id) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT s.* FROM statuses s
                    INNER JOIN bookmarks b ON s.id = b.status_id
                    LEFT JOIN bookmarks cb ON cb.status_id = ?
                    WHERE (
                        cb.status_id IS NOT NULL
                        AND (
                            b.created_at < cb.created_at
                            OR (b.created_at = cb.created_at AND s.id < ?)
                        )
                    ) OR (
                        cb.status_id IS NULL
                        AND s.id < ?
                    )
                    ORDER BY b.created_at DESC, s.id DESC
                    LIMIT ?
                    "#,
                )
                .bind(max_id)
                .bind(max_id)
                .bind(max_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT s.* FROM statuses s
                    INNER JOIN bookmarks b ON s.id = b.status_id
                    ORDER BY b.created_at DESC, s.id DESC
                    LIMIT ?
                    "#,
                )
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(statuses)
    }

    /// Get favourited statuses with JOIN (optimized, avoids N+1)
    pub async fn get_favourited_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let statuses = match max_id {
            Some(max_id) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT s.* FROM statuses s
                    INNER JOIN favourites f ON s.id = f.status_id
                    LEFT JOIN favourites cf ON cf.status_id = ?
                    WHERE (
                        cf.status_id IS NOT NULL
                        AND (
                            f.created_at < cf.created_at
                            OR (f.created_at = cf.created_at AND s.id < ?)
                        )
                    ) OR (
                        cf.status_id IS NULL
                        AND s.id < ?
                    )
                    ORDER BY f.created_at DESC, s.id DESC
                    LIMIT ?
                    "#,
                )
                .bind(max_id)
                .bind(max_id)
                .bind(max_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT s.* FROM statuses s
                    INNER JOIN favourites f ON s.id = f.status_id
                    ORDER BY f.created_at DESC, s.id DESC
                    LIMIT ?
                    "#,
                )
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(statuses)
    }

    /// Insert repost
    pub async fn insert_repost(&self, status_id: &str, uri: &str) -> Result<String, AppError> {
        let id = EntityId::new().0;
        sqlx::query(
            "INSERT INTO reposts (id, status_id, uri, created_at) VALUES (?, ?, ?, datetime('now'))"
        )
        .bind(&id)
        .bind(status_id)
        .bind(uri)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Delete repost
    pub async fn delete_repost(&self, status_id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM reposts WHERE status_id = ?")
            .bind(status_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get repost activity URI for a status.
    pub async fn get_repost_uri(&self, status_id: &str) -> Result<Option<String>, AppError> {
        let uri = sqlx::query_scalar::<_, String>(
            "SELECT uri FROM reposts WHERE status_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(status_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(uri)
    }

    /// Check if status is reposted
    pub async fn is_reposted(&self, status_id: &str) -> Result<bool, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reposts WHERE status_id = ?")
            .bind(status_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(count > 0)
    }

    // =========================================================================
    // Domain blocks
    // =========================================================================

    /// Check if domain is blocked
    pub async fn is_domain_blocked(&self, domain: &str) -> Result<bool, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domain_blocks WHERE domain = ?")
            .bind(domain)
            .fetch_one(&self.pool)
            .await?;

        Ok(count > 0)
    }

    /// Get all blocked domains
    pub async fn get_blocked_domains(&self) -> Result<Vec<String>, AppError> {
        let domains = sqlx::query_scalar::<_, String>(
            "SELECT domain FROM domain_blocks ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(domains)
    }

    /// Block a domain
    pub async fn block_domain(&self, domain: &str) -> Result<(), AppError> {
        let id = EntityId::new().0;
        sqlx::query(
            "INSERT INTO domain_blocks (id, domain, created_at) VALUES (?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(domain)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Unblock a domain
    pub async fn unblock_domain(&self, domain: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM domain_blocks WHERE domain = ?")
            .bind(domain)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get all domain blocks with details
    pub async fn get_all_domain_blocks(
        &self,
    ) -> Result<Vec<(String, String, chrono::DateTime<chrono::Utc>)>, AppError> {
        let blocks = sqlx::query_as::<_, (String, String, chrono::DateTime<chrono::Utc>)>(
            "SELECT id, domain, created_at FROM domain_blocks ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(blocks)
    }

    /// Insert domain block (alias for block_domain)
    pub async fn insert_domain_block(&self, domain: &str) -> Result<(), AppError> {
        self.block_domain(domain).await
    }

    // =========================================================================
    // Settings
    // =========================================================================

    /// Get setting value
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>, AppError> {
        let value = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;

        Ok(value)
    }

    /// Set setting value
    pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), AppError> {
        sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    // =========================================================================
    // OAuth Apps and Tokens
    // =========================================================================

    /// Insert OAuth app
    pub async fn insert_oauth_app(&self, app: &OAuthApp) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO oauth_apps (
                id, name, website, redirect_uri, client_id, client_secret, scopes, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&app.id)
        .bind(&app.name)
        .bind(&app.website)
        .bind(&app.redirect_uri)
        .bind(&app.client_id)
        .bind(&app.client_secret)
        .bind(&app.scopes)
        .bind(&app.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get OAuth app by client ID
    pub async fn get_oauth_app_by_client_id(
        &self,
        client_id: &str,
    ) -> Result<Option<OAuthApp>, AppError> {
        let app = sqlx::query_as::<_, OAuthApp>("SELECT * FROM oauth_apps WHERE client_id = ?")
            .bind(client_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(app)
    }

    /// Insert OAuth authorization code
    pub async fn insert_oauth_authorization_code(
        &self,
        code: &OAuthAuthorizationCode,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO oauth_authorization_codes (
                id, app_id, code, redirect_uri, scopes, created_at, expires_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&code.id)
        .bind(&code.app_id)
        .bind(&code.code)
        .bind(&code.redirect_uri)
        .bind(&code.scopes)
        .bind(&code.created_at)
        .bind(&code.expires_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get OAuth authorization code by code value
    pub async fn get_oauth_authorization_code(
        &self,
        code: &str,
    ) -> Result<Option<OAuthAuthorizationCode>, AppError> {
        let auth_code = sqlx::query_as::<_, OAuthAuthorizationCode>(
            "SELECT * FROM oauth_authorization_codes WHERE code = ?",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await?;

        Ok(auth_code)
    }

    /// Consume (single-use) OAuth authorization code with strict binding checks
    pub async fn consume_oauth_authorization_code(
        &self,
        code: &str,
        app_id: &str,
        redirect_uri: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuthAuthorizationCode>, AppError> {
        let Some(auth_code) = self.get_oauth_authorization_code(code).await? else {
            return Ok(None);
        };

        if auth_code.expires_at <= now {
            // Purge expired code on redemption attempt to avoid unbounded table growth.
            sqlx::query("DELETE FROM oauth_authorization_codes WHERE id = ?")
                .bind(&auth_code.id)
                .execute(&self.pool)
                .await?;
            return Ok(None);
        }

        if auth_code.app_id != app_id || auth_code.redirect_uri != redirect_uri {
            return Ok(None);
        }

        let result = sqlx::query("DELETE FROM oauth_authorization_codes WHERE id = ?")
            .bind(&auth_code.id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        Ok(Some(auth_code))
    }

    /// Insert OAuth token
    pub async fn insert_oauth_token(&self, token: &OAuthToken) -> Result<(), AppError> {
        let access_token_hash = hash_oauth_access_token(&token.access_token);
        sqlx::query(
            r#"
            INSERT INTO oauth_tokens (
                id, app_id, access_token, grant_type, scopes, created_at, revoked
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&token.id)
        .bind(&token.app_id)
        .bind(&access_token_hash)
        .bind(&token.grant_type)
        .bind(&token.scopes)
        .bind(&token.created_at)
        .bind(token.revoked)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get OAuth token by access token
    pub async fn get_oauth_token(
        &self,
        access_token: &str,
    ) -> Result<Option<OAuthToken>, AppError> {
        let access_token_hash = hash_oauth_access_token(access_token);
        let token = sqlx::query_as::<_, OAuthToken>(
            "SELECT * FROM oauth_tokens WHERE access_token = ? AND revoked = 0",
        )
        .bind(&access_token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(token)
    }

    /// Revoke OAuth token
    pub async fn revoke_oauth_token(&self, access_token: &str) -> Result<(), AppError> {
        let access_token_hash = hash_oauth_access_token(access_token);
        sqlx::query("UPDATE oauth_tokens SET revoked = 1 WHERE access_token = ?")
            .bind(&access_token_hash)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    // =========================================================================
    // Account Blocks & Mutes (Phase 2)
    // =========================================================================

    /// Block an account
    pub async fn block_account(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let existing_blocks =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_blocks")
                .fetch_all(&self.pool)
                .await?;
        let existing_match =
            find_matching_addresses(&existing_blocks, target_address, default_port)
                .into_iter()
                .next();
        if existing_match.is_none() {
            let id = EntityId::new().0;
            sqlx::query(
                "INSERT INTO account_blocks (id, target_address, created_at) VALUES (?, ?, datetime('now'))",
            )
            .bind(&id)
            .bind(target_address)
            .execute(&self.pool)
            .await?;
        }

        // Also remove any existing equivalent follow relationship.
        self.delete_follow(target_address, default_port).await?;

        Ok(existing_match.is_none())
    }

    /// Unblock an account
    pub async fn unblock_account(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let existing_blocks =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_blocks")
                .fetch_all(&self.pool)
                .await?;
        let matches = find_matching_addresses(&existing_blocks, target_address, default_port);
        let mut removed = false;
        for existing in matches {
            let result =
                sqlx::query("DELETE FROM account_blocks WHERE target_address COLLATE NOCASE = ?")
                    .bind(existing)
                    .execute(&self.pool)
                    .await?;
            removed |= result.rows_affected() > 0;
        }

        Ok(removed)
    }

    /// Check if account is blocked
    pub async fn is_account_blocked(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let existing_blocks =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_blocks")
                .fetch_all(&self.pool)
                .await?;
        Ok(existing_blocks
            .iter()
            .any(|existing| account_addresses_match(existing, target_address, default_port)))
    }

    /// Get blocked account addresses
    pub async fn get_blocked_accounts(&self, limit: usize) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT target_address FROM account_blocks ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    /// Mute an account
    pub async fn mute_account(
        &self,
        target_address: &str,
        mute_notifications: bool,
        duration: Option<i64>,
        default_port: Option<u16>,
    ) -> Result<(), AppError> {
        let existing_mutes =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_mutes")
                .fetch_all(&self.pool)
                .await?;
        let stored_target_address =
            find_matching_addresses(&existing_mutes, target_address, default_port)
                .into_iter()
                .next()
                .unwrap_or_else(|| target_address.to_string());

        let id = EntityId::new().0;
        sqlx::query(
            "INSERT OR REPLACE INTO account_mutes (id, target_address, notifications, duration, created_at) VALUES (?, ?, ?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(&stored_target_address)
        .bind(mute_notifications as i64)
        .bind(duration)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Unmute an account
    pub async fn unmute_account(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<(), AppError> {
        let existing_mutes =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_mutes")
                .fetch_all(&self.pool)
                .await?;
        let matches = find_matching_addresses(&existing_mutes, target_address, default_port);
        for existing in matches {
            sqlx::query("DELETE FROM account_mutes WHERE target_address COLLATE NOCASE = ?")
                .bind(existing)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    /// Check if account is muted
    pub async fn is_account_muted(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let existing_mutes =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_mutes")
                .fetch_all(&self.pool)
                .await?;
        Ok(existing_mutes
            .iter()
            .any(|existing| account_addresses_match(existing, target_address, default_port)))
    }

    /// Get muted account addresses
    pub async fn get_muted_accounts(&self, limit: usize) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT target_address FROM account_mutes ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    // =========================================================================
    // Follow Requests (Phase 2)
    // =========================================================================

    /// Get follow request addresses
    pub async fn get_follow_request_addresses(
        &self,
        limit: usize,
    ) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT requester_address FROM follow_requests ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    /// Check if follow request exists
    pub async fn has_follow_request(&self, requester_address: &str) -> Result<bool, AppError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM follow_requests WHERE requester_address = ?")
                .bind(requester_address)
                .fetch_one(&self.pool)
                .await?;

        Ok(count > 0)
    }

    /// Get follow request details
    pub async fn get_follow_request(
        &self,
        requester_address: &str,
    ) -> Result<Option<(String, String)>, AppError> {
        let result = sqlx::query_as::<_, (String, String)>(
            "SELECT inbox_uri, uri FROM follow_requests WHERE requester_address = ?",
        )
        .bind(requester_address)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }

    /// Insert follow request
    pub async fn insert_follow_request(
        &self,
        requester_address: &str,
        inbox_uri: &str,
        uri: &str,
    ) -> Result<(), AppError> {
        let id = EntityId::new().0;
        sqlx::query(
            "INSERT OR REPLACE INTO follow_requests (id, requester_address, inbox_uri, uri, created_at) VALUES (?, ?, ?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(requester_address)
        .bind(inbox_uri)
        .bind(uri)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Accept follow request
    pub async fn accept_follow_request(&self, requester_address: &str) -> Result<bool, AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<bool, AppError> = async {
            let follow_request = sqlx::query_as::<_, (String, String)>(
                "SELECT inbox_uri, uri FROM follow_requests WHERE requester_address = ?",
            )
            .bind(requester_address)
            .fetch_optional(&mut *conn)
            .await?;

            let Some((inbox_uri, uri)) = follow_request else {
                return Ok(false);
            };

            let follower_id = EntityId::new().0;
            sqlx::query(
                "INSERT INTO followers (id, follower_address, inbox_uri, uri, created_at) VALUES (?, ?, ?, ?, datetime('now'))",
            )
            .bind(&follower_id)
            .bind(requester_address)
            .bind(&inbox_uri)
            .bind(&uri)
            .execute(&mut *conn)
            .await?;

            sqlx::query("DELETE FROM follow_requests WHERE requester_address = ?")
                .bind(requester_address)
                .execute(&mut *conn)
                .await?;

            Ok(true)
        }
        .await;

        match result {
            Ok(accepted) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(accepted)
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }

    /// Reject follow request
    pub async fn reject_follow_request(&self, requester_address: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM follow_requests WHERE requester_address = ?")
            .bind(requester_address)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // =========================================================================
    // Lists (Phase 2)
    // =========================================================================

    /// Create a new list
    pub async fn create_list(&self, title: &str, replies_policy: &str) -> Result<String, AppError> {
        let id = EntityId::new().0;
        sqlx::query(
            r#"
            INSERT INTO lists (id, title, replies_policy, created_at, updated_at)
            VALUES (?, ?, ?, datetime('now'), datetime('now'))
            "#,
        )
        .bind(&id)
        .bind(title)
        .bind(replies_policy)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Get list by ID
    pub async fn get_list(&self, id: &str) -> Result<Option<(String, String, String)>, AppError> {
        let result = sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, title, replies_policy FROM lists WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }

    /// Get all lists
    pub async fn get_all_lists(&self) -> Result<Vec<(String, String, String)>, AppError> {
        let lists = sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, title, replies_policy FROM lists ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(lists)
    }

    /// Update list
    pub async fn update_list(
        &self,
        id: &str,
        title: &str,
        replies_policy: &str,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            "UPDATE lists SET title = ?, replies_policy = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(title)
        .bind(replies_policy)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete list
    pub async fn delete_list(&self, id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM lists WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Add account to list
    pub async fn add_account_to_list(
        &self,
        list_id: &str,
        account_address: &str,
    ) -> Result<(), AppError> {
        let id = EntityId::new().0;
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO list_accounts (id, list_id, account_address, created_at)
            VALUES (?, ?, ?, datetime('now'))
            "#,
        )
        .bind(&id)
        .bind(list_id)
        .bind(account_address)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Add multiple accounts to list atomically
    pub async fn add_accounts_to_list(
        &self,
        list_id: &str,
        account_addresses: &[String],
    ) -> Result<(), AppError> {
        if account_addresses.is_empty() {
            return Ok(());
        }

        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            for account_address in account_addresses {
                let id = EntityId::new().0;
                sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO list_accounts (id, list_id, account_address, created_at)
                    VALUES (?, ?, ?, datetime('now'))
                    "#,
                )
                .bind(&id)
                .bind(list_id)
                .bind(account_address)
                .execute(&mut *conn)
                .await?;
            }

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }

    /// Remove account from list
    pub async fn remove_account_from_list(
        &self,
        list_id: &str,
        account_address: &str,
    ) -> Result<bool, AppError> {
        let result =
            sqlx::query("DELETE FROM list_accounts WHERE list_id = ? AND account_address = ?")
                .bind(list_id)
                .bind(account_address)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Remove multiple accounts from list atomically
    pub async fn remove_accounts_from_list(
        &self,
        list_id: &str,
        account_addresses: &[String],
    ) -> Result<(), AppError> {
        if account_addresses.is_empty() {
            return Ok(());
        }

        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            for account_address in account_addresses {
                sqlx::query("DELETE FROM list_accounts WHERE list_id = ? AND account_address = ?")
                    .bind(list_id)
                    .bind(account_address)
                    .execute(&mut *conn)
                    .await?;
            }

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }

    /// Get accounts in list
    pub async fn get_list_accounts(&self, list_id: &str) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT account_address FROM list_accounts WHERE list_id = ? ORDER BY created_at DESC",
        )
        .bind(list_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    /// Check if account is in list
    pub async fn is_account_in_list(
        &self,
        list_id: &str,
        account_address: &str,
    ) -> Result<bool, AppError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM list_accounts WHERE list_id = ? AND account_address = ?",
        )
        .bind(list_id)
        .bind(account_address)
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }

    // =========================================================================
    // Filters (Phase 2)
    // =========================================================================

    /// Create a filter (v1 API)
    pub async fn create_filter(
        &self,
        phrase: &str,
        context: &str,
        expires_at: Option<&str>,
        irreversible: bool,
        whole_word: bool,
    ) -> Result<String, AppError> {
        let id = EntityId::new().0;
        sqlx::query(
            r#"
            INSERT INTO filters (id, phrase, context, expires_at, irreversible, whole_word, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            "#,
        )
        .bind(&id)
        .bind(phrase)
        .bind(context)
        .bind(expires_at)
        .bind(irreversible as i64)
        .bind(whole_word as i64)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Get filter by ID
    pub async fn get_filter(
        &self,
        id: &str,
    ) -> Result<Option<(String, String, String, Option<String>, bool, bool)>, AppError> {
        let result = sqlx::query_as::<_, (String, String, String, Option<String>, i64, i64)>(
            "SELECT id, phrase, context, expires_at, irreversible, whole_word FROM filters WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(
            |(id, phrase, context, expires_at, irreversible, whole_word)| {
                (
                    id,
                    phrase,
                    context,
                    expires_at,
                    irreversible != 0,
                    whole_word != 0,
                )
            },
        ))
    }

    /// Get all filters
    pub async fn get_all_filters(
        &self,
    ) -> Result<Vec<(String, String, String, Option<String>, bool, bool)>, AppError> {
        let filters = sqlx::query_as::<_, (String, String, String, Option<String>, i64, i64)>(
            "SELECT id, phrase, context, expires_at, irreversible, whole_word FROM filters ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(filters
            .into_iter()
            .map(
                |(id, phrase, context, expires_at, irreversible, whole_word)| {
                    (
                        id,
                        phrase,
                        context,
                        expires_at,
                        irreversible != 0,
                        whole_word != 0,
                    )
                },
            )
            .collect())
    }

    /// Update filter
    pub async fn update_filter(
        &self,
        id: &str,
        phrase: &str,
        context: &str,
        expires_at: Option<&str>,
        irreversible: bool,
        whole_word: bool,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE filters 
            SET phrase = ?, context = ?, expires_at = ?, irreversible = ?, whole_word = ?, updated_at = datetime('now')
            WHERE id = ?
            "#,
        )
        .bind(phrase)
        .bind(context)
        .bind(expires_at)
        .bind(irreversible as i64)
        .bind(whole_word as i64)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete filter
    pub async fn delete_filter(&self, id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM filters WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // =========================================================================
    // Polls (Phase 3)
    // =========================================================================

    /// Create a poll
    pub async fn create_poll(
        &self,
        status_id: &str,
        options: &[String],
        expires_in: i64,
        multiple: bool,
    ) -> Result<String, AppError> {
        let poll_id = EntityId::new().0;
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in);

        sqlx::query(
            r#"
            INSERT INTO polls (id, status_id, expires_at, expired, multiple, votes_count, voters_count, created_at)
            VALUES (?, ?, ?, 0, ?, 0, 0, datetime('now'))
            "#,
        )
        .bind(&poll_id)
        .bind(status_id)
        .bind(expires_at.to_rfc3339())
        .bind(multiple as i64)
        .execute(&self.pool)
        .await?;

        // Create poll options
        for (index, option) in options.iter().enumerate() {
            let option_id = EntityId::new().0;
            sqlx::query(
                r#"
                INSERT INTO poll_options (id, poll_id, title, votes_count, option_index, created_at)
                VALUES (?, ?, ?, 0, ?, datetime('now'))
                "#,
            )
            .bind(&option_id)
            .bind(&poll_id)
            .bind(option)
            .bind(index as i64)
            .execute(&self.pool)
            .await?;
        }

        Ok(poll_id)
    }

    /// Get poll by ID
    pub async fn get_poll(
        &self,
        poll_id: &str,
    ) -> Result<Option<(String, String, bool, bool, i64, i64)>, AppError> {
        let result = sqlx::query_as::<_, (String, String, i64, i64, i64, i64)>(
            "SELECT id, expires_at, expired, multiple, votes_count, voters_count FROM polls WHERE id = ?",
        )
        .bind(poll_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(
            |(id, expires_at, expired, multiple, votes_count, voters_count)| {
                (
                    id,
                    expires_at.clone(),
                    poll_is_expired(&expires_at, expired),
                    multiple != 0,
                    votes_count,
                    voters_count,
                )
            },
        ))
    }

    /// Get poll by status ID
    pub async fn get_poll_by_status_id(
        &self,
        status_id: &str,
    ) -> Result<Option<(String, String, bool, bool, i64, i64)>, AppError> {
        let result = sqlx::query_as::<_, (String, String, i64, i64, i64, i64)>(
            "SELECT id, expires_at, expired, multiple, votes_count, voters_count FROM polls WHERE status_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(status_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(
            |(id, expires_at, expired, multiple, votes_count, voters_count)| {
                (
                    id,
                    expires_at.clone(),
                    poll_is_expired(&expires_at, expired),
                    multiple != 0,
                    votes_count,
                    voters_count,
                )
            },
        ))
    }

    /// Get poll options
    pub async fn get_poll_options(
        &self,
        poll_id: &str,
    ) -> Result<Vec<(String, String, i64)>, AppError> {
        let options = sqlx::query_as::<_, (String, String, i64)>(
            "SELECT id, title, votes_count FROM poll_options WHERE poll_id = ? ORDER BY option_index",
        )
        .bind(poll_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(options)
    }

    /// Vote in poll
    pub async fn vote_in_poll(
        &self,
        poll_id: &str,
        voter_address: &str,
        option_ids: &[String],
    ) -> Result<(), AppError> {
        if option_ids.is_empty() {
            return Err(AppError::Validation(
                "At least one choice is required".to_string(),
            ));
        }

        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            let poll = sqlx::query_as::<_, (String, String, i64, i64, i64, i64)>(
                "SELECT id, expires_at, expired, multiple, votes_count, voters_count FROM polls WHERE id = ?",
            )
            .bind(poll_id)
            .fetch_optional(&mut *conn)
            .await?
            .map(
                |(id, expires_at, expired, multiple, votes_count, voters_count)| {
                    (
                        id,
                        expires_at.clone(),
                        poll_is_expired(&expires_at, expired),
                        multiple != 0,
                        votes_count,
                        voters_count,
                    )
                },
            )
            .ok_or(AppError::NotFound)?;

            if poll.2 {
                return Err(AppError::Validation("Poll has expired".to_string()));
            }
            if !poll.3 && option_ids.len() > 1 {
                return Err(AppError::Validation(
                    "Poll does not allow multiple choices".to_string(),
                ));
            }

            // A voter can submit at most one ballot per poll.
            // For multiple-choice polls, the ballot may include multiple options.
            let existing_vote: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM poll_votes WHERE poll_id = ? AND voter_address = ?",
            )
            .bind(poll_id)
            .bind(voter_address)
            .fetch_one(&mut *conn)
            .await?;

            if existing_vote > 0 {
                return Err(AppError::Validation(
                    "Already voted in this poll".to_string(),
                ));
            }

            for option_id in option_ids {
                let option_exists: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM poll_options WHERE id = ? AND poll_id = ?",
                )
                .bind(option_id)
                .bind(poll_id)
                .fetch_one(&mut *conn)
                .await?;
                if option_exists == 0 {
                    return Err(AppError::Validation("Invalid poll option".to_string()));
                }

                let vote_id = EntityId::new().0;
                let inserted = sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO poll_votes (id, poll_id, option_id, voter_address, created_at)
                    VALUES (?, ?, ?, ?, datetime('now'))
                    "#,
                )
                .bind(&vote_id)
                .bind(poll_id)
                .bind(option_id)
                .bind(voter_address)
                .execute(&mut *conn)
                .await?;
                if inserted.rows_affected() == 0 {
                    return Err(AppError::Validation(
                        "Already voted in this poll".to_string(),
                    ));
                }

                let updated = sqlx::query(
                    "UPDATE poll_options SET votes_count = votes_count + 1 WHERE id = ? AND poll_id = ?",
                )
                .bind(option_id)
                .bind(poll_id)
                .execute(&mut *conn)
                .await?;
                if updated.rows_affected() == 0 {
                    return Err(AppError::Validation("Invalid poll option".to_string()));
                }
            }

            // Update poll totals inside the same transaction.
            let total_votes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM poll_votes WHERE poll_id = ?")
                .bind(poll_id)
                .fetch_one(&mut *conn)
                .await?;
            let unique_voters: i64 = sqlx::query_scalar(
                "SELECT COUNT(DISTINCT voter_address) FROM poll_votes WHERE poll_id = ?",
            )
            .bind(poll_id)
            .fetch_one(&mut *conn)
            .await?;
            sqlx::query("UPDATE polls SET votes_count = ?, voters_count = ? WHERE id = ?")
                .bind(total_votes)
                .bind(unique_voters)
                .bind(poll_id)
                .execute(&mut *conn)
                .await?;

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }

    /// Update poll vote counts
    async fn update_poll_counts(&self, poll_id: &str) -> Result<(), AppError> {
        // Count total votes
        let total_votes: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM poll_votes WHERE poll_id = ?")
                .bind(poll_id)
                .fetch_one(&self.pool)
                .await?;

        // Count unique voters
        let unique_voters: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT voter_address) FROM poll_votes WHERE poll_id = ?",
        )
        .bind(poll_id)
        .fetch_one(&self.pool)
        .await?;

        sqlx::query("UPDATE polls SET votes_count = ?, voters_count = ? WHERE id = ?")
            .bind(total_votes)
            .bind(unique_voters)
            .bind(poll_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get user's votes in a poll
    pub async fn get_user_poll_votes(
        &self,
        poll_id: &str,
        voter_address: &str,
    ) -> Result<Vec<String>, AppError> {
        let option_ids = sqlx::query_scalar::<_, String>(
            "SELECT option_id FROM poll_votes WHERE poll_id = ? AND voter_address = ?",
        )
        .bind(poll_id)
        .bind(voter_address)
        .fetch_all(&self.pool)
        .await?;

        Ok(option_ids)
    }

    // =========================================================================
    // Scheduled Statuses (Phase 3)
    // =========================================================================

    /// Create scheduled status
    pub async fn create_scheduled_status(
        &self,
        scheduled_at: &str,
        status_text: &str,
        visibility: &str,
        content_warning: Option<&str>,
        in_reply_to_id: Option<&str>,
        media_ids: Option<&str>,
        poll_options: Option<&str>,
        poll_expires_in: Option<i64>,
        poll_multiple: bool,
    ) -> Result<String, AppError> {
        let id = EntityId::new().0;
        sqlx::query(
            r#"
            INSERT INTO scheduled_statuses (
                id, scheduled_at, status_text, visibility, content_warning,
                in_reply_to_id, media_ids, poll_options, poll_expires_in, poll_multiple,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            "#,
        )
        .bind(&id)
        .bind(scheduled_at)
        .bind(status_text)
        .bind(visibility)
        .bind(content_warning)
        .bind(in_reply_to_id)
        .bind(media_ids)
        .bind(poll_options)
        .bind(poll_expires_in)
        .bind(poll_multiple as i64)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Get scheduled status by ID
    pub async fn get_scheduled_status(
        &self,
        id: &str,
    ) -> Result<Option<serde_json::Value>, AppError> {
        let result = sqlx::query(
            r#"
            SELECT id, scheduled_at, status_text, visibility, content_warning,
                   in_reply_to_id, media_ids, poll_options, poll_expires_in, poll_multiple
            FROM scheduled_statuses WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = result {
            let media_ids = parse_json_value(row.get::<Option<String>, _>("media_ids"));
            let poll_options = parse_json_value(row.get::<Option<String>, _>("poll_options"));
            Ok(Some(serde_json::json!({
                "id": row.get::<String, _>("id"),
                "scheduled_at": row.get::<String, _>("scheduled_at"),
                "params": {
                    "text": row.get::<String, _>("status_text"),
                    "visibility": row.get::<String, _>("visibility"),
                    "spoiler_text": row.get::<Option<String>, _>("content_warning"),
                    "in_reply_to_id": row.get::<Option<String>, _>("in_reply_to_id"),
                    "media_ids": media_ids,
                    "poll": if poll_options.is_some() {
                        Some(serde_json::json!({
                            "options": poll_options,
                            "expires_in": row.get::<Option<i64>, _>("poll_expires_in"),
                            "multiple": row.get::<i64, _>("poll_multiple") != 0,
                        }))
                    } else {
                        None
                    }
                },
                "media_attachments": []
            })))
        } else {
            Ok(None)
        }
    }

    /// Get all scheduled statuses
    pub async fn get_all_scheduled_statuses(
        &self,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, AppError> {
        let rows = sqlx::query(
            r#"
            SELECT id, scheduled_at, status_text, visibility, content_warning,
                   in_reply_to_id, media_ids, poll_options, poll_expires_in, poll_multiple
            FROM scheduled_statuses
            ORDER BY scheduled_at ASC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut results = Vec::new();
        for row in rows {
            let media_ids = parse_json_value(row.get::<Option<String>, _>("media_ids"));
            let poll_options = parse_json_value(row.get::<Option<String>, _>("poll_options"));
            results.push(serde_json::json!({
                "id": row.get::<String, _>("id"),
                "scheduled_at": row.get::<String, _>("scheduled_at"),
                "params": {
                    "text": row.get::<String, _>("status_text"),
                    "visibility": row.get::<String, _>("visibility"),
                    "spoiler_text": row.get::<Option<String>, _>("content_warning"),
                    "in_reply_to_id": row.get::<Option<String>, _>("in_reply_to_id"),
                    "media_ids": media_ids,
                    "poll": if poll_options.is_some() {
                        Some(serde_json::json!({
                            "options": poll_options,
                            "expires_in": row.get::<Option<i64>, _>("poll_expires_in"),
                            "multiple": row.get::<i64, _>("poll_multiple") != 0,
                        }))
                    } else {
                        None
                    }
                },
                "media_attachments": []
            }));
        }

        Ok(results)
    }

    /// Update scheduled status time
    pub async fn update_scheduled_status(
        &self,
        id: &str,
        scheduled_at: &str,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            "UPDATE scheduled_statuses SET scheduled_at = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(scheduled_at)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete scheduled status
    pub async fn delete_scheduled_status(&self, id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM scheduled_statuses WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // =========================================================================
    // Conversations (Phase 3)
    // =========================================================================

    /// Create or get conversation for participants
    pub async fn get_or_create_conversation(
        &self,
        participant_addresses: &[String],
    ) -> Result<String, AppError> {
        // For simplicity, we'll create a new conversation
        // In a full implementation, we'd check for existing conversations with the same participants
        let conversation_id = EntityId::new().0;

        sqlx::query(
            "INSERT INTO conversations (id, unread, created_at, updated_at) VALUES (?, 1, datetime('now'), datetime('now'))",
        )
        .bind(&conversation_id)
        .execute(&self.pool)
        .await?;

        // Add participants
        for address in participant_addresses {
            let participant_id = EntityId::new().0;
            sqlx::query(
                "INSERT INTO conversation_participants (id, conversation_id, account_address, created_at) VALUES (?, ?, ?, datetime('now'))",
            )
            .bind(&participant_id)
            .bind(&conversation_id)
            .bind(address)
            .execute(&self.pool)
            .await?;
        }

        Ok(conversation_id)
    }

    /// Add status to conversation
    pub async fn add_status_to_conversation(
        &self,
        conversation_id: &str,
        status_id: &str,
    ) -> Result<(), AppError> {
        let id = EntityId::new().0;
        sqlx::query(
            "INSERT OR IGNORE INTO conversation_statuses (id, conversation_id, status_id, created_at) VALUES (?, ?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(conversation_id)
        .bind(status_id)
        .execute(&self.pool)
        .await?;

        // Update conversation's last_status_id and updated_at
        sqlx::query(
            "UPDATE conversations SET last_status_id = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(status_id)
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get all conversations
    pub async fn get_conversations(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, Option<String>, bool)>, AppError> {
        let conversations = sqlx::query_as::<_, (String, Option<String>, i64)>(
            "SELECT id, last_status_id, unread FROM conversations ORDER BY updated_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(conversations
            .into_iter()
            .map(|(id, last_status_id, unread)| (id, last_status_id, unread != 0))
            .collect())
    }

    /// Get conversation participants
    pub async fn get_conversation_participants(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT account_address FROM conversation_participants WHERE conversation_id = ?",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    /// Mark conversation as read
    pub async fn mark_conversation_read(&self, conversation_id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("UPDATE conversations SET unread = 0 WHERE id = ?")
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete conversation (hide from user)
    pub async fn delete_conversation(&self, conversation_id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM conversations WHERE id = ?")
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // =========================================================================
    // Search (Phase 3)
    // =========================================================================

    /// Search statuses using full-text search
    ///
    /// # Arguments
    /// * `query` - Search query string
    /// * `limit` - Maximum number of results
    /// * `offset` - Offset for pagination
    ///
    /// # Returns
    /// List of matching statuses
    pub async fn search_statuses(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Status>, AppError> {
        // Use FTS5 for full-text search
        let statuses = sqlx::query_as::<_, Status>(
            r#"
            SELECT s.*
            FROM statuses s
            INNER JOIN statuses_fts fts ON s.id = fts.status_id
            WHERE statuses_fts MATCH ?
            ORDER BY s.created_at DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(query)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(statuses)
    }

    /// Search hashtags by name
    ///
    /// # Arguments
    /// * `query` - Hashtag name to search (without #)
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    /// List of (hashtag_name, usage_count, last_used_at) tuples
    pub async fn search_hashtags(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, i64, Option<String>)>, AppError> {
        // Search hashtags using LIKE for partial matching
        let hashtags = sqlx::query_as::<_, (String, i64, Option<String>)>(
            r#"
            SELECT name, usage_count, last_used_at
            FROM hashtag_stats
            WHERE name LIKE ?
            ORDER BY usage_count DESC, name ASC
            LIMIT ?
            "#,
        )
        .bind(format!("%{}%", query))
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(hashtags)
    }

    /// Get trending hashtags
    ///
    /// # Arguments
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    /// List of (hashtag_name, usage_count, last_used_at) tuples
    pub async fn get_trending_hashtags(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, i64, Option<String>)>, AppError> {
        // Get most used hashtags in the last 7 days
        let hashtags = sqlx::query_as::<_, (String, i64, Option<String>)>(
            r#"
            SELECT name, usage_count, last_used_at
            FROM hashtag_stats
            WHERE last_used_at IS NOT NULL
              AND datetime(last_used_at) > datetime('now', '-7 days')
            ORDER BY usage_count DESC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(hashtags)
    }
}
