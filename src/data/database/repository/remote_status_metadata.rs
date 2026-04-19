use super::super::*;

impl Database {
    pub async fn replace_remote_status_mentions(
        &self,
        status_id: &str,
        mentions: &[RemoteStatusMention],
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM remote_status_mentions WHERE status_id = ?")
            .bind(status_id)
            .execute(&mut *tx)
            .await?;

        for mention in mentions {
            sqlx::query(
                r#"
                INSERT INTO remote_status_mentions (
                    id,
                    status_id,
                    actor_uri,
                    username,
                    acct,
                    url,
                    created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&mention.id)
            .bind(&mention.status_id)
            .bind(&mention.actor_uri)
            .bind(&mention.username)
            .bind(&mention.acct)
            .bind(&mention.url)
            .bind(mention.created_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_remote_status_mentions(
        &self,
        status_id: &str,
    ) -> Result<Vec<RemoteStatusMention>, AppError> {
        sqlx::query_as::<_, RemoteStatusMention>(
            r#"
            SELECT id, status_id, actor_uri, username, acct, url, created_at
            FROM remote_status_mentions
            WHERE status_id = ?
            ORDER BY created_at ASC, id ASC
            "#,
        )
        .bind(status_id)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::from)
    }

    pub async fn replace_remote_status_tags(
        &self,
        status_id: &str,
        tags: &[RemoteStatusTag],
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM remote_status_tags WHERE status_id = ?")
            .bind(status_id)
            .execute(&mut *tx)
            .await?;

        for tag in tags {
            sqlx::query(
                r#"
                INSERT INTO remote_status_tags (
                    id,
                    status_id,
                    name,
                    url,
                    created_at
                ) VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(&tag.id)
            .bind(&tag.status_id)
            .bind(&tag.name)
            .bind(&tag.url)
            .bind(tag.created_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_remote_status_tags(
        &self,
        status_id: &str,
    ) -> Result<Vec<RemoteStatusTag>, AppError> {
        sqlx::query_as::<_, RemoteStatusTag>(
            r#"
            SELECT id, status_id, name, url, created_at
            FROM remote_status_tags
            WHERE status_id = ?
            ORDER BY created_at ASC, id ASC
            "#,
        )
        .bind(status_id)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::from)
    }
}
