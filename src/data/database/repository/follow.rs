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
        let inboxes = sqlx::query_scalar::<_, String>("SELECT DISTINCT inbox_uri FROM followers")
            .fetch_all(&self.pool)
            .await?;

        Ok(inboxes)
    }

    /// Insert new follow relationship
    pub async fn insert_follow(&self, follow: &Follow) -> Result<(), AppError> {
        sqlx::query(
            "INSERT OR IGNORE INTO follows (id, target_address, uri, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&follow.id)
        .bind(&follow.target_address)
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
                "INSERT OR IGNORE INTO follows (id, target_address, uri, created_at) VALUES (?, ?, ?, ?)",
            )
            .bind(&follow.id)
            .bind(&follow.target_address)
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
            "INSERT INTO followers (id, follower_address, inbox_uri, uri, created_at) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&follower.id)
        .bind(&follower.follower_address)
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
}
