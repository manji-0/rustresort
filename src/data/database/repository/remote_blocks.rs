use super::super::*;

impl Database {
    /// Record that a remote actor has blocked the local account.
    pub async fn record_remote_block(&self, actor_uri: &str) -> Result<(), AppError> {
        sqlx::query(
            "INSERT OR REPLACE INTO remote_blocks (actor_uri, created_at) VALUES (?, CURRENT_TIMESTAMP)",
        )
        .bind(actor_uri)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Remove a previously recorded remote block.
    pub async fn remove_remote_block(&self, actor_uri: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM remote_blocks WHERE actor_uri = ?")
            .bind(actor_uri)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Return whether the remote actor has blocked the local account.
    pub async fn is_blocked_by_remote(&self, actor_uri: &str) -> Result<bool, AppError> {
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM remote_blocks WHERE actor_uri = ?)",
        )
        .bind(actor_uri)
        .fetch_one(&self.pool)
        .await?;
        Ok(exists != 0)
    }
}
