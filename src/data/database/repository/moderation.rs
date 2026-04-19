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
        self.block_account_with_remote_metadata(target_address, None, None, default_port)
            .await
    }

    /// Block an account and persist resolved remote metadata when available.
    pub async fn block_account_with_remote_metadata(
        &self,
        target_address: &str,
        actor_uri: Option<&str>,
        inbox_uri: Option<&str>,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<bool, AppError> = async {
            let existing_blocks =
                sqlx::query_scalar::<_, String>("SELECT target_address FROM account_blocks")
                    .fetch_all(&mut *conn)
                    .await?;
            let existing_match = find_matching_addresses(&existing_blocks, target_address, default_port)
                .into_iter()
                .next();

            if existing_match.is_none() {
                let id = EntityId::new_string();
                sqlx::query(
                    "INSERT INTO account_blocks (id, target_address, actor_uri, inbox_uri, created_at) VALUES (?, ?, ?, ?, datetime('now'))",
                )
                .bind(&id)
                .bind(target_address)
                .bind(actor_uri)
                .bind(inbox_uri)
                .execute(&mut *conn)
                .await?;
            }

            let existing_follows = sqlx::query_scalar::<_, String>("SELECT target_address FROM follows")
                .fetch_all(&mut *conn)
                .await?;
            let follow_matches = find_matching_addresses(&existing_follows, target_address, default_port);
            for existing in follow_matches {
                sqlx::query("DELETE FROM follows WHERE target_address COLLATE NOCASE = ?")
                    .bind(existing)
                    .execute(&mut *conn)
                    .await?;
            }

            let existing_followers = sqlx::query_scalar::<_, String>(
                "SELECT follower_address FROM followers",
            )
            .fetch_all(&mut *conn)
            .await?;
            let follower_matches =
                find_matching_addresses(&existing_followers, target_address, default_port);
            for existing in follower_matches {
                sqlx::query("DELETE FROM followers WHERE follower_address COLLATE NOCASE = ?")
                    .bind(existing)
                    .execute(&mut *conn)
                    .await?;
            }

            Ok(existing_match.is_none())
        }
        .await;

        match result {
            Ok(inserted) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(inserted)
            }
            Err(error) => {
                super::rollback_with_log(&mut conn, "block_account").await;
                Err(error)
            }
        }
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

    /// Get stored block metadata for an account when present.
    pub async fn get_block_target(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<Option<(String, Option<String>, Option<String>)>, AppError> {
        let existing_blocks =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_blocks")
                .fetch_all(&self.pool)
                .await?;
        let Some(stored_target_address) =
            find_matching_addresses(&existing_blocks, target_address, default_port)
                .into_iter()
                .next()
        else {
            return Ok(None);
        };

        let row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
            "SELECT target_address, actor_uri, inbox_uri FROM account_blocks WHERE target_address COLLATE NOCASE = ? LIMIT 1",
        )
        .bind(stored_target_address)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
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

    /// Get blocked account details with optional remote metadata.
    pub async fn get_blocked_account_details(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, Option<String>, Option<String>)>, AppError> {
        let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
            "SELECT target_address, actor_uri, inbox_uri FROM account_blocks ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_all_blocked_account_details(
        &self,
    ) -> Result<Vec<(String, Option<String>, Option<String>)>, AppError> {
        sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
            "SELECT target_address, actor_uri, inbox_uri FROM account_blocks ORDER BY created_at DESC, id DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::from)
    }

    /// Check if a remote actor URI is explicitly blocked locally.
    pub async fn is_actor_uri_blocked(&self, actor_uri: &str) -> Result<bool, AppError> {
        let normalized = actor_uri.trim().trim_end_matches('/');
        let is_blocked = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(
                SELECT 1
                FROM account_blocks
                WHERE lower(rtrim(actor_uri, '/')) = lower(?)
            )",
        )
        .bind(normalized)
        .fetch_one(&self.pool)
        .await?;

        Ok(is_blocked != 0)
    }

    /// Mute an account
    pub async fn mute_account(
        &self,
        target_address: &str,
        mute_notifications: bool,
        duration: Option<i64>,
        default_port: Option<u16>,
    ) -> Result<(), AppError> {
        self.mute_account_with_actor_uri(
            target_address,
            mute_notifications,
            duration,
            None,
            default_port,
        )
        .await
    }

    /// Mute an account and persist canonical actor URI when available.
    pub async fn mute_account_with_actor_uri(
        &self,
        target_address: &str,
        mute_notifications: bool,
        duration: Option<i64>,
        actor_uri: Option<&str>,
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
            "INSERT OR REPLACE INTO account_mutes (id, target_address, notifications, duration, actor_uri, created_at) VALUES (?, ?, ?, ?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(&stored_target_address)
        .bind(mute_notifications as i64)
        .bind(duration)
        .bind(actor_uri)
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

    /// Get muted account details with optional remote metadata.
    pub async fn get_muted_account_details(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, Option<String>)>, AppError> {
        let rows = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT target_address, actor_uri FROM account_mutes ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_all_muted_account_details(
        &self,
    ) -> Result<Vec<(String, Option<String>)>, AppError> {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT target_address, actor_uri FROM account_mutes ORDER BY created_at DESC, id DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::from)
    }

    /// Mark an account as sensitive for media/content warnings.
    pub async fn mark_account_sensitive_with_actor_uri(
        &self,
        target_address: &str,
        actor_uri: Option<&str>,
        default_port: Option<u16>,
    ) -> Result<(), AppError> {
        let existing =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_sensitives")
                .fetch_all(&self.pool)
                .await?;
        let stored_target_address =
            find_matching_addresses(&existing, target_address, default_port)
                .into_iter()
                .next()
                .unwrap_or_else(|| target_address.to_string());

        let id = EntityId::new_string();
        sqlx::query(
            "INSERT OR REPLACE INTO account_sensitives (id, target_address, actor_uri, created_at) VALUES (?, ?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(&stored_target_address)
        .bind(actor_uri)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Remove sensitive marking from an account.
    pub async fn unmark_account_sensitive(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<(), AppError> {
        let existing =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_sensitives")
                .fetch_all(&self.pool)
                .await?;
        let matches = find_matching_addresses(&existing, target_address, default_port);
        for stored_target_address in matches {
            sqlx::query("DELETE FROM account_sensitives WHERE target_address COLLATE NOCASE = ?")
                .bind(stored_target_address)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    /// Check if an account is marked sensitive.
    pub async fn is_account_sensitive(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let existing =
            sqlx::query_scalar::<_, String>("SELECT target_address FROM account_sensitives")
                .fetch_all(&self.pool)
                .await?;
        Ok(existing
            .iter()
            .any(|stored| account_addresses_match(stored, target_address, default_port)))
    }
}
