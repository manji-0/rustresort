use super::super::*;

impl Database {
    // =========================================================================
    // Follow relationships
    // =========================================================================

    /// Get all follow addresses
    ///
    /// # Returns
    /// List of addresses (user@domain) that the user follows
    pub async fn get_all_follow_addresses(&self) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT target_address FROM follows ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    /// Get all follows with canonical actor URIs when available.
    pub async fn get_all_follows(&self) -> Result<Vec<Follow>, AppError> {
        let follows = sqlx::query_as::<_, Follow>("SELECT * FROM follows ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;

        Ok(follows)
    }

    /// Count follows.
    pub async fn count_follow_addresses(&self) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM follows")
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    /// Get all follower addresses
    ///
    /// # Returns
    /// List of addresses (user@domain) that follow the user
    pub async fn get_all_follower_addresses(&self) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT follower_address FROM followers ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    /// Get all followers with canonical actor URIs when available.
    pub async fn get_all_followers(&self) -> Result<Vec<Follower>, AppError> {
        let followers =
            sqlx::query_as::<_, Follower>("SELECT * FROM followers ORDER BY created_at DESC")
                .fetch_all(&self.pool)
                .await?;

        Ok(followers)
    }

    /// Count followers.
    pub async fn count_follower_addresses(&self) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM followers")
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    /// Count locally-authored statuses.
    pub async fn count_local_statuses(&self) -> Result<i64, AppError> {
        let count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM statuses WHERE is_local = 1")
                .fetch_one(&self.pool)
                .await?;
        Ok(count)
    }

    /// Count locally-authored statuses created within a time range.
    pub async fn count_local_statuses_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM statuses WHERE is_local = 1 AND created_at >= ? AND created_at < ?",
        )
        .bind(start)
        .bind(end)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    /// Count statuses attributed to a specific account address (case-insensitive exact match).
    pub async fn count_statuses_by_account_address(
        &self,
        account_address: &str,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM statuses WHERE account_address COLLATE NOCASE = ?",
        )
        .bind(account_address)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    /// Count statuses for an account address, including default-port-equivalent variants.
    pub async fn count_statuses_by_account_address_with_default_port(
        &self,
        account_address: &str,
        default_port: Option<u16>,
    ) -> Result<i64, AppError> {
        let candidates = equivalent_account_address_candidates(account_address, default_port);
        if candidates.is_empty() {
            return Ok(0);
        }

        let mut total = 0_i64;
        for candidate in candidates {
            total += self.count_statuses_by_account_address(&candidate).await?;
        }
        Ok(total)
    }

    /// Get follower inbox URIs for activity delivery
    pub async fn get_follower_inboxes(&self) -> Result<Vec<String>, AppError> {
        let inboxes = sqlx::query_scalar::<_, String>(
            r#"
            SELECT DISTINCT followers.inbox_uri
            FROM followers
            LEFT JOIN remote_blocks ON remote_blocks.actor_uri = followers.actor_uri
            WHERE remote_blocks.actor_uri IS NULL
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(inboxes)
    }

    /// Insert new follow relationship
    pub async fn insert_follow(&self, follow: &Follow) -> Result<(), AppError> {
        sqlx::query(
            "INSERT OR IGNORE INTO follows (id, target_address, actor_uri, uri, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&follow.id)
        .bind(&follow.target_address)
        .bind(&follow.actor_uri)
        .bind(&follow.uri)
        .bind(follow.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Insert follow relationship when no equivalent address exists.
    ///
    /// Uses an IMMEDIATE transaction so the equivalence check and insert are atomic.
    pub async fn insert_follow_if_absent(
        &self,
        follow: &Follow,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<bool, AppError> = async {
            let existing_addresses =
                sqlx::query_scalar::<_, String>("SELECT target_address FROM follows")
                    .fetch_all(&mut *conn)
                    .await?;
            if existing_addresses
                .iter()
                .any(|existing| account_addresses_match(existing, &follow.target_address, default_port))
            {
                return Ok(false);
            }

            let inserted = sqlx::query(
                "INSERT OR IGNORE INTO follows (id, target_address, actor_uri, uri, created_at) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&follow.id)
            .bind(&follow.target_address)
            .bind(&follow.actor_uri)
            .bind(&follow.uri)
            .bind(follow.created_at)
            .execute(&mut *conn)
            .await?;

            Ok(inserted.rows_affected() > 0)
        }
        .await;

        match result {
            Ok(inserted) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(inserted)
            }
            Err(error) => {
                super::rollback_with_log(&mut conn, "insert_follow_if_absent").await;
                Err(error)
            }
        }
    }

    /// Get Follow activity URI for a target address.
    pub async fn get_follow_uri(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<Option<String>, AppError> {
        let candidates = equivalent_account_address_candidates(target_address, default_port);
        if candidates.is_empty() {
            return Ok(None);
        }

        let mut query_builder = QueryBuilder::<Sqlite>::new(
            "SELECT uri FROM follows WHERE target_address COLLATE NOCASE IN (",
        );
        {
            let mut separated = query_builder.separated(", ");
            for candidate in &candidates {
                separated.push_bind(candidate);
            }
        }
        query_builder.push(") ORDER BY created_at DESC LIMIT 1");

        let uri = query_builder
            .build_query_scalar::<String>()
            .fetch_optional(&self.pool)
            .await?;

        Ok(uri)
    }

    /// Get the most recent follow row for a target address.
    pub async fn get_follow(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<Option<Follow>, AppError> {
        let candidates = equivalent_account_address_candidates(target_address, default_port);
        if candidates.is_empty() {
            return Ok(None);
        }

        let mut query_builder = QueryBuilder::<Sqlite>::new(
            "SELECT * FROM follows WHERE target_address COLLATE NOCASE IN (",
        );
        {
            let mut separated = query_builder.separated(", ");
            for candidate in &candidates {
                separated.push_bind(candidate);
            }
        }
        query_builder.push(") ORDER BY created_at DESC LIMIT 1");

        let follow = query_builder
            .build_query_as::<Follow>()
            .fetch_optional(&self.pool)
            .await?;

        Ok(follow)
    }

    /// Update canonical actor URI for an existing follow relationship.
    pub async fn update_follow_actor_uri(
        &self,
        target_address: &str,
        actor_uri: &str,
        default_port: Option<u16>,
    ) -> Result<(), AppError> {
        let existing_addresses = self.get_all_follow_addresses().await?;
        let matches = find_matching_addresses(&existing_addresses, target_address, default_port);
        for existing in matches {
            sqlx::query("UPDATE follows SET actor_uri = ? WHERE target_address COLLATE NOCASE = ?")
                .bind(actor_uri)
                .bind(existing)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    /// Mark a follow relationship as accepted and refresh the canonical actor URI.
    pub async fn mark_follow_accepted(
        &self,
        target_address: &str,
        actor_uri: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let existing_addresses = self.get_all_follow_addresses().await?;
        let matches = find_matching_addresses(&existing_addresses, target_address, default_port);
        let mut updated = false;
        for existing in matches {
            let result = sqlx::query(
                "UPDATE follows SET actor_uri = ?, accepted_at = datetime('now') WHERE target_address COLLATE NOCASE = ?",
            )
            .bind(actor_uri)
            .bind(existing)
            .execute(&self.pool)
            .await?;
            updated |= result.rows_affected() > 0;
        }

        Ok(updated)
    }

    /// Return whether a follow relationship has been marked as accepted.
    pub async fn is_follow_accepted(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let candidates = equivalent_account_address_candidates(target_address, default_port);
        if candidates.is_empty() {
            return Ok(false);
        }

        let mut query_builder = QueryBuilder::<Sqlite>::new(
            "SELECT COUNT(*) FROM follows WHERE accepted_at IS NOT NULL AND target_address COLLATE NOCASE IN (",
        );
        {
            let mut separated = query_builder.separated(", ");
            for candidate in &candidates {
                separated.push_bind(candidate);
            }
        }
        query_builder.push(")");

        let count: i64 = query_builder
            .build_query_scalar()
            .fetch_one(&self.pool)
            .await?;

        Ok(count > 0)
    }

    /// Delete follow relationship
    pub async fn delete_follow(
        &self,
        target_address: &str,
        default_port: Option<u16>,
    ) -> Result<(), AppError> {
        let existing_addresses = self.get_all_follow_addresses().await?;
        let matches = find_matching_addresses(&existing_addresses, target_address, default_port);
        for existing in matches {
            sqlx::query("DELETE FROM follows WHERE target_address COLLATE NOCASE = ?")
                .bind(existing)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    /// Insert new follower
    pub async fn insert_follower(&self, follower: &Follower) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO followers (id, follower_address, actor_uri, inbox_uri, uri, created_at) VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(follower_address) DO UPDATE SET actor_uri = excluded.actor_uri, inbox_uri = excluded.inbox_uri, uri = excluded.uri, created_at = excluded.created_at"
        )
        .bind(&follower.id)
        .bind(&follower.follower_address)
        .bind(&follower.actor_uri)
        .bind(&follower.inbox_uri)
        .bind(&follower.uri)
        .bind(follower.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete follower
    pub async fn delete_follower(
        &self,
        follower_address: &str,
        default_port: Option<u16>,
    ) -> Result<(), AppError> {
        let existing_addresses = self.get_all_follower_addresses().await?;
        let matches = find_matching_addresses(&existing_addresses, follower_address, default_port);
        for existing in matches {
            sqlx::query("DELETE FROM followers WHERE follower_address COLLATE NOCASE = ?")
                .bind(existing)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    /// Delete follower by follower address and Follow activity URI
    pub async fn delete_follower_by_address_and_uri(
        &self,
        follower_address: &str,
        follow_uri: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let existing_addresses = self.get_all_follower_addresses().await?;
        let matches = find_matching_addresses(&existing_addresses, follower_address, default_port);
        let mut removed = false;
        for existing in matches {
            let result = sqlx::query(
                "DELETE FROM followers WHERE follower_address COLLATE NOCASE = ? AND uri = ?",
            )
            .bind(existing)
            .bind(follow_uri)
            .execute(&self.pool)
            .await?;
            removed |= result.rows_affected() > 0;
        }

        Ok(removed)
    }

    /// Get a follower row matching the provided address, accounting for
    /// case-insensitive and default-port-equivalent variants.
    pub async fn get_follower(
        &self,
        follower_address: &str,
        default_port: Option<u16>,
    ) -> Result<Option<Follower>, AppError> {
        let candidates = equivalent_account_address_candidates(follower_address, default_port);
        if candidates.is_empty() {
            return Ok(None);
        }

        let mut query_builder = QueryBuilder::<Sqlite>::new(
            "SELECT * FROM followers WHERE follower_address COLLATE NOCASE IN (",
        );
        {
            let mut separated = query_builder.separated(", ");
            for candidate in &candidates {
                separated.push_bind(candidate);
            }
        }
        query_builder.push(") ORDER BY created_at DESC LIMIT 1");

        let follower = query_builder
            .build_query_as::<Follower>()
            .fetch_optional(&self.pool)
            .await?;

        Ok(follower)
    }
}
