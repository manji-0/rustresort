use super::super::*;

impl Database {
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
        .bind(notification.notification_type)
        .bind(&notification.origin_account_address)
        .bind(&notification.status_uri)
        .bind(notification.read)
        .bind(notification.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Insert notification once for a specific ActivityPub activity URI.
    ///
    /// Re-delivery of the same remote activity should not create duplicate
    /// notifications, so callers can provide the upstream activity URI and rely
    /// on a unique partial index.
    pub async fn insert_notification_if_new(
        &self,
        notification: &Notification,
        activity_uri: Option<&str>,
    ) -> Result<bool, AppError> {
        let inserted = sqlx::query(
            r#"
            INSERT INTO notifications (
                id, notification_type, origin_account_address, status_uri, read, created_at, activity_uri
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(&notification.id)
        .bind(notification.notification_type)
        .bind(&notification.origin_account_address)
        .bind(&notification.status_uri)
        .bind(notification.read)
        .bind(notification.created_at)
        .bind(activity_uri)
        .execute(&self.pool)
        .await?;

        Ok(inserted.rows_affected() > 0)
    }

    /// Get notifications (paginated)
    pub async fn get_notifications(
        &self,
        limit: usize,
        max_id: Option<&str>,
        unread_only: bool,
    ) -> Result<Vec<Notification>, AppError> {
        let cursor = match max_id {
            Some(id) => sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
                "SELECT created_at FROM notifications WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .map(|created_at| (id, created_at)),
            None => None,
        };

        let notifications = match (cursor, unread_only) {
            (Some((max_id, created_at)), true) => {
                sqlx::query_as::<_, Notification>(
                    r#"
                    SELECT * FROM notifications
                    WHERE read = 0
                      AND (created_at < ? OR (created_at = ? AND id < ?))
                    ORDER BY created_at DESC, id DESC
                    LIMIT ?
                    "#,
                )
                .bind(created_at)
                .bind(created_at)
                .bind(max_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (Some((max_id, created_at)), false) => {
                sqlx::query_as::<_, Notification>(
                    r#"
                    SELECT * FROM notifications
                    WHERE created_at < ? OR (created_at = ? AND id < ?)
                    ORDER BY created_at DESC, id DESC
                    LIMIT ?
                    "#,
                )
                .bind(created_at)
                .bind(created_at)
                .bind(max_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, true) => {
                sqlx::query_as::<_, Notification>(
                    "SELECT * FROM notifications WHERE read = 0 ORDER BY created_at DESC, id DESC LIMIT ?"
                )
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, false) => {
                sqlx::query_as::<_, Notification>(
                    "SELECT * FROM notifications ORDER BY created_at DESC, id DESC LIMIT ?"
                )
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(notifications)
    }

    /// Count unread notifications exactly.
    pub async fn count_unread_notifications(&self) -> Result<i64, AppError> {
        let count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM notifications WHERE read = 0")
                .fetch_one(&self.pool)
                .await?;
        Ok(count)
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

    /// Get notifications belonging to a semantic notification group.
    pub async fn get_notifications_by_group_scope(
        &self,
        notification_type: NotificationType,
        scope: &str,
    ) -> Result<Vec<Notification>, AppError> {
        sqlx::query_as::<_, Notification>(
            r#"
            SELECT *
            FROM notifications
            WHERE notification_type = ?
              AND (
                    status_uri = ?
                 OR (status_uri IS NULL AND origin_account_address = ?)
              )
            ORDER BY created_at DESC, id DESC
            "#,
        )
        .bind(notification_type)
        .bind(scope)
        .bind(scope)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Mark all notifications as read
    pub async fn mark_all_notifications_read(&self) -> Result<(), AppError> {
        sqlx::query("UPDATE notifications SET read = 1")
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Mark notifications up to and including the cursor notification as read.
    pub async fn mark_notifications_read_through(&self, id: &str) -> Result<(), AppError> {
        let Some(created_at) = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
            "SELECT created_at FROM notifications WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(());
        };

        sqlx::query(
            r#"
            UPDATE notifications
            SET read = 1
            WHERE created_at < ?
               OR (created_at = ? AND id <= ?)
            "#,
        )
        .bind(created_at)
        .bind(created_at)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete a single notification by ID.
    pub async fn delete_notification(&self, id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM notifications WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delete all notifications.
    pub async fn clear_notifications(&self) -> Result<u64, AppError> {
        let result = sqlx::query("DELETE FROM notifications")
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Delete notifications belonging to a semantic notification group.
    pub async fn delete_notifications_by_group_scope(
        &self,
        notification_type: NotificationType,
        scope: &str,
    ) -> Result<u64, AppError> {
        let result = sqlx::query(
            r#"
            DELETE FROM notifications
            WHERE notification_type = ?
              AND (
                    status_uri = ?
                 OR (status_uri IS NULL AND origin_account_address = ?)
              )
            "#,
        )
        .bind(notification_type)
        .bind(scope)
        .bind(scope)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Delete notifications created for a specific upstream ActivityPub activity URI.
    pub async fn delete_notifications_by_activity_uri(
        &self,
        activity_uri: &str,
    ) -> Result<u64, AppError> {
        let result = sqlx::query("DELETE FROM notifications WHERE activity_uri = ?")
            .bind(activity_uri)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Delete notifications by semantic identity when the upstream activity URI
    /// is not available in compact Undo payloads.
    pub async fn delete_notifications_by_identity(
        &self,
        notification_type: NotificationType,
        origin_account_address: &str,
        status_uri: Option<&str>,
    ) -> Result<u64, AppError> {
        let result = sqlx::query(
            r#"
            DELETE FROM notifications
            WHERE notification_type = ?
              AND origin_account_address = ?
              AND (
                    (status_uri IS NULL AND ? IS NULL)
                 OR status_uri = ?
              )
            "#,
        )
        .bind(notification_type)
        .bind(origin_account_address)
        .bind(status_uri)
        .bind(status_uri)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}
