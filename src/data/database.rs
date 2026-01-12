//! SQLite database operations
//!
//! All database access goes through this module.
//! Uses SQLx for compile-time checked queries.

use sqlx::{Pool, Row, Sqlite, SqlitePool};
use std::path::Path;

use super::models::*;
use crate::error::AppError;

/// Database connection pool wrapper
pub struct Database {
    pool: Pool<Sqlite>,
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
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::Database(sqlx::Error::Io(e)))?;
        }

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

        tracing::info!("Database connected and migrated successfully");

        Ok(Self { pool })
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
    pub async fn get_statuses_by_uris(
        &self,
        uris: &[String],
    ) -> Result<Vec<Status>, AppError> {
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

    /// Delete status by ID
    pub async fn delete_status(&self, id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM statuses WHERE id = ?")
            .bind(id)
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
        sqlx::query("UPDATE media_attachments SET status_id = ? WHERE id = ?")
            .bind(status_id)
            .bind(media_id)
            .execute(&self.pool)
            .await?;

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
            "INSERT INTO follows (id, target_address, uri, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&follow.id)
        .bind(&follow.target_address)
        .bind(&follow.uri)
        .bind(&follow.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete follow relationship
    pub async fn delete_follow(&self, target_address: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM follows WHERE target_address = ?")
            .bind(target_address)
            .execute(&self.pool)
            .await?;

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
    pub async fn delete_follower(&self, follower_address: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM followers WHERE follower_address = ?")
            .bind(follower_address)
            .execute(&self.pool)
            .await?;

        Ok(())
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

    /// Delete favourite
    pub async fn delete_favourite(&self, status_id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM favourites WHERE status_id = ?")
            .bind(status_id)
            .execute(&self.pool)
            .await?;

        Ok(())
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
                    WHERE b.id < ?
                    ORDER BY b.created_at DESC
                    LIMIT ?
                    "#
                )
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
                    ORDER BY b.created_at DESC
                    LIMIT ?
                    "#
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
                    WHERE f.id < ?
                    ORDER BY f.created_at DESC
                    LIMIT ?
                    "#
                )
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
                    ORDER BY f.created_at DESC
                    LIMIT ?
                    "#
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
    pub async fn get_all_domain_blocks(&self) -> Result<Vec<(String, String, chrono::DateTime<chrono::Utc>)>, AppError> {
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

    /// Insert OAuth token
    pub async fn insert_oauth_token(&self, token: &OAuthToken) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO oauth_tokens (
                id, app_id, access_token, scopes, created_at, revoked
            ) VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&token.id)
        .bind(&token.app_id)
        .bind(&token.access_token)
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
        let token = sqlx::query_as::<_, OAuthToken>(
            "SELECT * FROM oauth_tokens WHERE access_token = ? AND revoked = 0",
        )
        .bind(access_token)
        .fetch_optional(&self.pool)
        .await?;

        Ok(token)
    }

    /// Revoke OAuth token
    pub async fn revoke_oauth_token(&self, access_token: &str) -> Result<(), AppError> {
        sqlx::query("UPDATE oauth_tokens SET revoked = 1 WHERE access_token = ?")
            .bind(access_token)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    // =========================================================================
    // Account Blocks & Mutes (Phase 2)
    // =========================================================================

    /// Block an account
    pub async fn block_account(&self, target_address: &str) -> Result<(), AppError> {
        let id = EntityId::new().0;
        sqlx::query(
            "INSERT OR REPLACE INTO account_blocks (id, target_address, created_at) VALUES (?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(target_address)
        .execute(&self.pool)
        .await?;

        // Also remove any existing follow relationship
        sqlx::query("DELETE FROM follows WHERE target_address = ?")
            .bind(target_address)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Unblock an account
    pub async fn unblock_account(&self, target_address: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM account_blocks WHERE target_address = ?")
            .bind(target_address)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Check if account is blocked
    pub async fn is_account_blocked(&self, target_address: &str) -> Result<bool, AppError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM account_blocks WHERE target_address = ?")
                .bind(target_address)
                .fetch_one(&self.pool)
                .await?;

        Ok(count > 0)
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
    ) -> Result<(), AppError> {
        let id = EntityId::new().0;
        sqlx::query(
            "INSERT OR REPLACE INTO account_mutes (id, target_address, notifications, duration, created_at) VALUES (?, ?, ?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(target_address)
        .bind(mute_notifications as i64)
        .bind(duration)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Unmute an account
    pub async fn unmute_account(&self, target_address: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM account_mutes WHERE target_address = ?")
            .bind(target_address)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Check if account is muted
    pub async fn is_account_muted(&self, target_address: &str) -> Result<bool, AppError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM account_mutes WHERE target_address = ?")
                .bind(target_address)
                .fetch_one(&self.pool)
                .await?;

        Ok(count > 0)
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

    /// Accept follow request
    pub async fn accept_follow_request(&self, requester_address: &str) -> Result<(), AppError> {
        // Get follow request details
        if let Some((inbox_uri, uri)) = self.get_follow_request(requester_address).await? {
            // Move to followers table
            let follower_id = EntityId::new().0;
            sqlx::query(
                "INSERT INTO followers (id, follower_address, inbox_uri, uri, created_at) VALUES (?, ?, ?, ?, datetime('now'))",
            )
            .bind(&follower_id)
            .bind(requester_address)
            .bind(&inbox_uri)
            .bind(&uri)
            .execute(&self.pool)
            .await?;

            // Remove from follow_requests
            sqlx::query("DELETE FROM follow_requests WHERE requester_address = ?")
                .bind(requester_address)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
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
                    expires_at,
                    expired != 0,
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
        // Check if poll allows multiple votes
        let poll = self.get_poll(poll_id).await?.ok_or(AppError::NotFound)?;

        if !poll.2 && option_ids.len() > 1 {
            return Err(AppError::Validation(
                "Poll does not allow multiple choices".to_string(),
            ));
        }

        // Check if already voted (for single-choice polls)
        if !poll.2 {
            let existing_vote: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM poll_votes WHERE poll_id = ? AND voter_address = ?",
            )
            .bind(poll_id)
            .bind(voter_address)
            .fetch_one(&self.pool)
            .await?;

            if existing_vote > 0 {
                return Err(AppError::Validation(
                    "Already voted in this poll".to_string(),
                ));
            }
        }

        // Record votes
        for option_id in option_ids {
            let vote_id = EntityId::new().0;
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO poll_votes (id, poll_id, option_id, voter_address, created_at)
                VALUES (?, ?, ?, ?, datetime('now'))
                "#,
            )
            .bind(&vote_id)
            .bind(poll_id)
            .bind(option_id)
            .bind(voter_address)
            .execute(&self.pool)
            .await?;

            // Update option vote count
            sqlx::query("UPDATE poll_options SET votes_count = votes_count + 1 WHERE id = ?")
                .bind(option_id)
                .execute(&self.pool)
                .await?;
        }

        // Update poll totals
        self.update_poll_counts(poll_id).await?;

        Ok(())
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
            Ok(Some(serde_json::json!({
                "id": row.get::<String, _>("id"),
                "scheduled_at": row.get::<String, _>("scheduled_at"),
                "params": {
                    "text": row.get::<String, _>("status_text"),
                    "visibility": row.get::<String, _>("visibility"),
                    "spoiler_text": row.get::<Option<String>, _>("content_warning"),
                    "in_reply_to_id": row.get::<Option<String>, _>("in_reply_to_id"),
                    "media_ids": row.get::<Option<String>, _>("media_ids"),
                    "poll": if row.get::<Option<String>, _>("poll_options").is_some() {
                        Some(serde_json::json!({
                            "options": row.get::<Option<String>, _>("poll_options"),
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
            results.push(serde_json::json!({
                "id": row.get::<String, _>("id"),
                "scheduled_at": row.get::<String, _>("scheduled_at"),
                "params": {
                    "text": row.get::<String, _>("status_text"),
                    "visibility": row.get::<String, _>("visibility"),
                    "spoiler_text": row.get::<Option<String>, _>("content_warning"),
                    "in_reply_to_id": row.get::<Option<String>, _>("in_reply_to_id"),
                    "media_ids": row.get::<Option<String>, _>("media_ids"),
                    "poll": if row.get::<Option<String>, _>("poll_options").is_some() {
                        Some(serde_json::json!({
                            "options": row.get::<Option<String>, _>("poll_options"),
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

