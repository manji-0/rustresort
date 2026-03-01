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
        // For simplicity, we'll create a new conversation
        // In a full implementation, we'd check for existing conversations with the same participants
        let conversation_id = EntityId::new_string();

        sqlx::query(
            "INSERT INTO conversations (id, unread, created_at, updated_at) VALUES (?, 1, datetime('now'), datetime('now'))",
        )
        .bind(&conversation_id)
        .execute(&self.pool)
        .await?;

        // Add participants
        for address in participant_addresses {
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
}
