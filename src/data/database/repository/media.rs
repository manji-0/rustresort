use super::super::*;

impl Database {
    // =========================================================================
    // Media Attachments
    // =========================================================================

    /// Insert media attachment
    pub async fn insert_media(&self, media: &MediaAttachment) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO media_attachments (
                id, status_id, s3_key, thumbnail_s3_key, content_type,
                file_size, description, blurhash, width, height, focus_x, focus_y, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        .bind(media.focus_x)
        .bind(media.focus_y)
        .bind(media.created_at)
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
            "SELECT * FROM media_attachments WHERE status_id = ? ORDER BY created_at ASC, id ASC",
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

    /// Replace all media attachments for a status.
    ///
    /// Existing attachments not listed in `media_ids` are detached. Each listed
    /// media attachment must be either unattached or already attached to the
    /// same status.
    pub async fn replace_status_media(
        &self,
        status_id: &str,
        media_ids: &[String],
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;

        if media_ids.is_empty() {
            sqlx::query("UPDATE media_attachments SET status_id = NULL WHERE status_id = ?")
                .bind(status_id)
                .execute(&mut *tx)
                .await?;
        } else {
            let placeholders = std::iter::repeat_n("?", media_ids.len())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "UPDATE media_attachments SET status_id = NULL WHERE status_id = ? AND id NOT IN ({})",
                placeholders
            );
            let mut query = sqlx::query(&sql).bind(status_id);
            for media_id in media_ids {
                query = query.bind(media_id);
            }
            query.execute(&mut *tx).await?;
        }

        for media_id in media_ids {
            let result = sqlx::query(
                "UPDATE media_attachments SET status_id = ? WHERE id = ? AND (status_id IS NULL OR status_id = ?)",
            )
            .bind(status_id)
            .bind(media_id)
            .bind(status_id)
            .execute(&mut *tx)
            .await?;

            if result.rows_affected() == 0 {
                return Err(AppError::Validation(format!(
                    "media attachment is already attached to another status: {}",
                    media_id
                )));
            }
        }

        tx.commit().await?;
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
            SET description = ?, thumbnail_s3_key = ?, blurhash = ?, width = ?, height = ?, focus_x = ?, focus_y = ?
            WHERE id = ?
            "#,
        )
        .bind(&media.description)
        .bind(&media.thumbnail_s3_key)
        .bind(&media.blurhash)
        .bind(media.width)
        .bind(media.height)
        .bind(media.focus_x)
        .bind(media.focus_y)
        .bind(&media.id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
