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
}
