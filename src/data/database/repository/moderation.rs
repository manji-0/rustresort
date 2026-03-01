use super::super::*;

impl Database {
    // =========================================================================
    // Account Blocks & Mutes (Phase 2)
    // =========================================================================

    /// Block an account
    pub async fn block_account(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let existing_blocks =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_blocks")
                .fetch_all(&self.pool)
                .await?;
        let existing_match =
            find_matching_addresses(&existing_blocks, target_address, default_port)
                .into_iter()
                .next();
        if existing_match.is_none() {
            let id = EntityId::new_string();
            sqlx::query(
                "INSERT INTO account_blocks (id, target_address, created_at) VALUES (?, ?, datetime('now'))",
            )
            .bind(&id)
            .bind(target_address)
            .execute(&self.pool)
            .await?;
        }

        // Also remove any existing equivalent follow relationship.
        self.delete_follow(target_address, default_port).await?;

        Ok(existing_match.is_none())
    }

    /// Unblock an account
    pub async fn unblock_account(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let existing_blocks =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_blocks")
                .fetch_all(&self.pool)
                .await?;
        let matches = find_matching_addresses(&existing_blocks, target_address, default_port);
        let mut removed = false;
        for existing in matches {
            let result =
                sqlx::query("DELETE FROM account_blocks WHERE target_address COLLATE NOCASE = ?")
                    .bind(existing)
                    .execute(&self.pool)
                    .await?;
            removed |= result.rows_affected() > 0;
        }

        Ok(removed)
    }

    /// Check if account is blocked
    pub async fn is_account_blocked(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let existing_blocks =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_blocks")
                .fetch_all(&self.pool)
                .await?;
        Ok(existing_blocks
            .iter()
            .any(|existing| account_addresses_match(existing, target_address, default_port)))
    }

    /// Get blocked account addresses
    pub async fn get_blocked_accounts(&self, limit: usize) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT target_address FROM account_blocks ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    /// Mute an account
    pub async fn mute_account(
        &self,
        target_address: &str,
        mute_notifications: bool,
        duration: Option<i64>,
        default_port: Option<u16>,
    ) -> Result<(), AppError> {
        let existing_mutes =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_mutes")
                .fetch_all(&self.pool)
                .await?;
        let stored_target_address =
            find_matching_addresses(&existing_mutes, target_address, default_port)
                .into_iter()
                .next()
                .unwrap_or_else(|| target_address.to_string());

        let id = EntityId::new_string();
        sqlx::query(
            "INSERT OR REPLACE INTO account_mutes (id, target_address, notifications, duration, created_at) VALUES (?, ?, ?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(&stored_target_address)
        .bind(mute_notifications as i64)
        .bind(duration)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Unmute an account
    pub async fn unmute_account(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<(), AppError> {
        let existing_mutes =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_mutes")
                .fetch_all(&self.pool)
                .await?;
        let matches = find_matching_addresses(&existing_mutes, target_address, default_port);
        for existing in matches {
            sqlx::query("DELETE FROM account_mutes WHERE target_address COLLATE NOCASE = ?")
                .bind(existing)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    /// Check if account is muted
    pub async fn is_account_muted(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let existing_mutes =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_mutes")
                .fetch_all(&self.pool)
                .await?;
        Ok(existing_mutes
            .iter()
            .any(|existing| account_addresses_match(existing, target_address, default_port)))
    }

    /// Get mute notifications preference for an account.
    pub async fn get_account_mute_notifications(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<Option<bool>, AppError> {
        let existing_mutes = sqlx::query_as::<_, (String, i64)>(
            "SELECT target_address, notifications FROM account_mutes",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(existing_mutes
            .into_iter()
            .find(|(existing, _)| account_addresses_match(existing, target_address, default_port))
            .map(|(_, notifications)| notifications != 0))
    }

    /// Get muted account addresses
    pub async fn get_muted_accounts(&self, limit: usize) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT target_address FROM account_mutes ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }
}
