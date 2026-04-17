use super::super::*;
use crate::data::ScheduledStatusInsert;

impl Database {
    // =========================================================================
    // Scheduled Statuses (Phase 3)
    // =========================================================================

    /// Create scheduled status
    pub async fn create_scheduled_status(
        &self,
        request: &ScheduledStatusInsert,
    ) -> Result<String, AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            r#"
            INSERT INTO scheduled_statuses (
                id, scheduled_at, status_text, visibility, content_warning,
                in_reply_to_id, quoted_status_id, media_ids, poll_options, poll_expires_in, poll_multiple, language,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            "#,
        )
        .bind(&id)
        .bind(request.scheduled_at.as_str())
        .bind(request.status_text.as_str())
        .bind(request.visibility.as_str())
        .bind(request.content_warning.as_deref())
        .bind(request.in_reply_to_id.as_deref())
        .bind(request.quoted_status_id.as_deref())
        .bind(request.media_ids.as_deref())
        .bind(request.poll_options.as_deref())
        .bind(request.poll_expires_in)
        .bind(request.poll_multiple as i64)
        .bind(request.language.as_deref())
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
                   in_reply_to_id, quoted_status_id, media_ids, poll_options, poll_expires_in, poll_multiple,
                   language, error
            FROM scheduled_statuses
            WHERE id = ? AND published_at IS NULL
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = result {
            let media_ids = parse_json_value(row.get::<Option<String>, _>("media_ids"));
            let poll_options = parse_json_value(row.get::<Option<String>, _>("poll_options"));
            let media_attachments = self
                .load_scheduled_media_attachments(
                    media_ids.as_ref().and_then(|value| value.as_array()),
                )
                .await?;
            let sensitive = row.get::<Option<String>, _>("content_warning").is_some();
            Ok(Some(serde_json::json!({
                "id": row.get::<String, _>("id"),
                "scheduled_at": row.get::<String, _>("scheduled_at"),
                "params": {
                    "text": row.get::<String, _>("status_text"),
                    "visibility": row.get::<String, _>("visibility"),
                    "sensitive": sensitive,
                    "spoiler_text": row.get::<Option<String>, _>("content_warning"),
                    "in_reply_to_id": row.get::<Option<String>, _>("in_reply_to_id"),
                    "quoted_status_id": row.get::<Option<String>, _>("quoted_status_id"),
                    "language": row.get::<Option<String>, _>("language"),
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
                "media_attachments": media_attachments,
                "error": row.get::<Option<String>, _>("error")
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
                   in_reply_to_id, quoted_status_id, media_ids, poll_options, poll_expires_in, poll_multiple,
                   language, error
            FROM scheduled_statuses
            WHERE published_at IS NULL
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
            let media_attachments = self
                .load_scheduled_media_attachments(
                    media_ids.as_ref().and_then(|value| value.as_array()),
                )
                .await?;
            let sensitive = row.get::<Option<String>, _>("content_warning").is_some();
            results.push(serde_json::json!({
                "id": row.get::<String, _>("id"),
                "scheduled_at": row.get::<String, _>("scheduled_at"),
                "params": {
                    "text": row.get::<String, _>("status_text"),
                    "visibility": row.get::<String, _>("visibility"),
                    "sensitive": sensitive,
                    "spoiler_text": row.get::<Option<String>, _>("content_warning"),
                    "in_reply_to_id": row.get::<Option<String>, _>("in_reply_to_id"),
                    "quoted_status_id": row.get::<Option<String>, _>("quoted_status_id"),
                    "language": row.get::<Option<String>, _>("language"),
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
                "media_attachments": media_attachments,
                "error": row.get::<Option<String>, _>("error")
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
            "UPDATE scheduled_statuses SET scheduled_at = ?, error = NULL, updated_at = datetime('now') WHERE id = ? AND published_at IS NULL",
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

    /// Get due scheduled statuses that have not yet been published or failed.
    pub async fn get_due_scheduled_statuses(
        &self,
        limit: usize,
    ) -> Result<Vec<ScheduledStatus>, AppError> {
        let rows = sqlx::query_as::<_, ScheduledStatus>(
            r#"
            SELECT *
            FROM scheduled_statuses
            WHERE published_at IS NULL
              AND error IS NULL
              AND datetime(scheduled_at) <= datetime('now')
            ORDER BY datetime(scheduled_at) ASC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Mark a scheduled status as successfully published.
    pub async fn mark_scheduled_status_published(&self, id: &str) -> Result<(), AppError> {
        sqlx::query(
            r#"
            UPDATE scheduled_statuses
            SET published_at = datetime('now'),
                error = NULL,
                updated_at = datetime('now')
            WHERE id = ?
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Mark a scheduled status as failed so it will not be retried automatically.
    pub async fn mark_scheduled_status_failed(
        &self,
        id: &str,
        error: &str,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"
            UPDATE scheduled_statuses
            SET error = ?,
                updated_at = datetime('now')
            WHERE id = ?
            "#,
        )
        .bind(error)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn load_scheduled_media_attachments(
        &self,
        media_ids: Option<&Vec<serde_json::Value>>,
    ) -> Result<Vec<serde_json::Value>, AppError> {
        let Some(media_ids) = media_ids else {
            return Ok(Vec::new());
        };

        let mut attachments = Vec::new();
        for media_id in media_ids.iter().filter_map(|value| value.as_str()) {
            if let Some(media) = self.get_media(media_id).await? {
                attachments.push(serde_json::json!({
                    "id": media.id,
                    "type": if media.content_type.starts_with("image/") {
                        "image"
                    } else if media.content_type.starts_with("video/") {
                        "video"
                    } else {
                        "unknown"
                    },
                    "description": media.description,
                    "blurhash": media.blurhash,
                }));
            }
        }

        Ok(attachments)
    }
}
