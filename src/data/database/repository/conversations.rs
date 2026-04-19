use super::super::*;

impl Database {
    // =========================================================================
    // Conversations (Phase 3)
    // =========================================================================

    /// Create or get conversation for participants
    pub async fn get_or_create_conversation(
        &self,
        participant_addresses: &[String],
    ) -> Result<String, AppError> {
        let mut normalized_participants = participant_addresses
            .iter()
            .map(|address| address.trim())
            .filter(|address| !address.is_empty())
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        normalized_participants.sort();
        normalized_participants.dedup();
        if normalized_participants.is_empty() {
            return Err(AppError::Validation(
                "conversation participants must not be empty".to_string(),
            ));
        }

        let placeholders = vec!["?"; normalized_participants.len()].join(", ");
        let query = format!(
            r#"
            SELECT conversation_id
            FROM conversation_participants
            GROUP BY conversation_id
            HAVING COUNT(*) = ?
               AND SUM(CASE WHEN LOWER(account_address) IN ({placeholders}) THEN 1 ELSE 0 END) = ?
            LIMIT 1
            "#
        );
        let mut existing_query =
            sqlx::query_scalar::<_, String>(&query).bind(normalized_participants.len() as i64);
        for address in &normalized_participants {
            existing_query = existing_query.bind(address);
        }
        existing_query = existing_query.bind(normalized_participants.len() as i64);
        if let Some(existing_conversation_id) = existing_query.fetch_optional(&self.pool).await? {
            return Ok(existing_conversation_id);
        }

        let conversation_id = EntityId::new_string();
        sqlx::query(
            "INSERT INTO conversations (id, unread, created_at, updated_at) VALUES (?, 1, datetime('now'), datetime('now'))",
        )
        .bind(&conversation_id)
        .execute(&self.pool)
        .await?;

        // Add participants
        for address in &normalized_participants {
            let participant_id = EntityId::new_string();
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
        let id = EntityId::new_string();
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
        max_id: Option<&str>,
        since_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<(String, Option<String>, bool)>, AppError> {
        let conversations = match (max_id, since_id, min_id) {
            (Some(max_id), Some(since_id), _) => {
                sqlx::query_as::<_, (String, Option<String>, i64)>(
                    r#"
                    SELECT id, last_status_id, unread
                    FROM conversations
                    WHERE (
                        updated_at < (SELECT updated_at FROM conversations WHERE id = ?)
                        OR (
                            updated_at = (SELECT updated_at FROM conversations WHERE id = ?)
                            AND id < ?
                        )
                    )
                    AND (
                        updated_at > (SELECT updated_at FROM conversations WHERE id = ?)
                        OR (
                            updated_at = (SELECT updated_at FROM conversations WHERE id = ?)
                            AND id > ?
                        )
                    )
                    ORDER BY updated_at DESC, id DESC
                    LIMIT ?
                    "#,
                )
                .bind(max_id)
                .bind(max_id)
                .bind(max_id)
                .bind(since_id)
                .bind(since_id)
                .bind(since_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (Some(max_id), None, Some(min_id)) => {
                sqlx::query_as::<_, (String, Option<String>, i64)>(
                    r#"
                    SELECT id, last_status_id, unread
                    FROM conversations
                    WHERE (
                        updated_at < (SELECT updated_at FROM conversations WHERE id = ?)
                        OR (
                            updated_at = (SELECT updated_at FROM conversations WHERE id = ?)
                            AND id < ?
                        )
                    )
                    AND (
                        updated_at > (SELECT updated_at FROM conversations WHERE id = ?)
                        OR (
                            updated_at = (SELECT updated_at FROM conversations WHERE id = ?)
                            AND id > ?
                        )
                    )
                    ORDER BY updated_at DESC, id DESC
                    LIMIT ?
                    "#,
                )
                .bind(max_id)
                .bind(max_id)
                .bind(max_id)
                .bind(min_id)
                .bind(min_id)
                .bind(min_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (Some(max_id), None, None) => {
                sqlx::query_as::<_, (String, Option<String>, i64)>(
                    r#"
                    SELECT id, last_status_id, unread
                    FROM conversations
                    WHERE
                        updated_at < (SELECT updated_at FROM conversations WHERE id = ?)
                        OR (
                            updated_at = (SELECT updated_at FROM conversations WHERE id = ?)
                            AND id < ?
                        )
                    ORDER BY updated_at DESC, id DESC
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
            (None, Some(since_id), _) => {
                sqlx::query_as::<_, (String, Option<String>, i64)>(
                    r#"
                    SELECT id, last_status_id, unread
                    FROM conversations
                    WHERE
                        updated_at > (SELECT updated_at FROM conversations WHERE id = ?)
                        OR (
                            updated_at = (SELECT updated_at FROM conversations WHERE id = ?)
                            AND id > ?
                        )
                    ORDER BY updated_at DESC, id DESC
                    LIMIT ?
                    "#,
                )
                .bind(since_id)
                .bind(since_id)
                .bind(since_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, None, Some(min_id)) => {
                sqlx::query_as::<_, (String, Option<String>, i64)>(
                    r#"
                    SELECT id, last_status_id, unread
                    FROM conversations
                    WHERE
                        updated_at > (SELECT updated_at FROM conversations WHERE id = ?)
                        OR (
                            updated_at = (SELECT updated_at FROM conversations WHERE id = ?)
                            AND id > ?
                        )
                    ORDER BY updated_at DESC, id DESC
                    LIMIT ?
                    "#,
                )
                .bind(min_id)
                .bind(min_id)
                .bind(min_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, None, None) => {
                sqlx::query_as::<_, (String, Option<String>, i64)>(
                    "SELECT id, last_status_id, unread FROM conversations ORDER BY updated_at DESC, id DESC LIMIT ?",
                )
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(conversations
            .into_iter()
            .map(|(id, last_status_id, unread)| (id, last_status_id, unread != 0))
            .collect())
    }

    /// Get one conversation by ID.
    pub async fn get_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<(String, Option<String>, bool)>, AppError> {
        let conversation = sqlx::query_as::<_, (String, Option<String>, i64)>(
            "SELECT id, last_status_id, unread FROM conversations WHERE id = ? LIMIT 1",
        )
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(conversation.map(|(id, last_status_id, unread)| (id, last_status_id, unread != 0)))
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
}
