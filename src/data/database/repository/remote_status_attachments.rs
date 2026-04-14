use super::super::*;

impl Database {
    /// Replace the remote attachment metadata for a status.
    pub async fn replace_remote_status_attachments(
        &self,
        status_id: &str,
        attachments: &[RemoteStatusAttachment],
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM remote_status_attachments WHERE status_id = ?")
            .bind(status_id)
            .execute(&mut *tx)
            .await?;

        for attachment in attachments {
            sqlx::query(
                r#"
                INSERT INTO remote_status_attachments (
                    id,
                    status_id,
                    remote_url,
                    preview_url,
                    content_type,
                    description,
                    blurhash,
                    width,
                    height,
                    created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&attachment.id)
            .bind(&attachment.status_id)
            .bind(&attachment.remote_url)
            .bind(&attachment.preview_url)
            .bind(&attachment.content_type)
            .bind(&attachment.description)
            .bind(&attachment.blurhash)
            .bind(attachment.width)
            .bind(attachment.height)
            .bind(attachment.created_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Load remote attachment metadata for a status.
    pub async fn get_remote_status_attachments(
        &self,
        status_id: &str,
    ) -> Result<Vec<RemoteStatusAttachment>, AppError> {
        let attachments = sqlx::query_as::<_, RemoteStatusAttachment>(
            r#"
            SELECT
                id,
                status_id,
                remote_url,
                preview_url,
                content_type,
                description,
                blurhash,
                width,
                height,
                created_at
            FROM remote_status_attachments
            WHERE status_id = ?
            ORDER BY created_at ASC, id ASC
            "#,
        )
        .bind(status_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(attachments)
    }

    /// Return whether a status has any local or remote attachment metadata.
    pub async fn status_has_any_media(&self, status_id: &str) -> Result<bool, AppError> {
        let local_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM media_attachments WHERE status_id = ?",
        )
        .bind(status_id)
        .fetch_one(&self.pool)
        .await?;
        if local_count > 0 {
            return Ok(true);
        }

        let remote_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM remote_status_attachments WHERE status_id = ?",
        )
        .bind(status_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(remote_count > 0)
    }
}
