use super::super::*;

impl Database {
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
