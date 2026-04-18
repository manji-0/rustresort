use super::super::*;

impl Database {
    // =========================================================================
    // Follow Requests (Phase 2)
    // =========================================================================

    /// Get follow request addresses
    pub async fn get_follow_request_addresses(
        &self,
        limit: usize,
    ) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT requester_address FROM follow_requests ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    /// Get follow request identities with actor URIs when available.
    pub async fn get_follow_request_details(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, Option<String>)>, AppError> {
        let rows = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT requester_address, actor_uri FROM follow_requests ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Resolve a follow-request path identity to the stored requester address.
    ///
    /// Accepts either the raw requester address or a stored canonical actor URI.
    /// Default-port-equivalent address variants are treated as matches.
    pub async fn resolve_follow_request_requester(
        &self,
        identity: &str,
        default_port: Option<u16>,
    ) -> Result<Option<String>, AppError> {
        let trimmed = identity.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let rows = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT requester_address, actor_uri FROM follow_requests ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        for (requester_address, actor_uri) in rows {
            if requester_address.eq_ignore_ascii_case(trimmed)
                || actor_uri
                    .as_deref()
                    .is_some_and(|stored| stored.eq_ignore_ascii_case(trimmed))
            {
                return Ok(Some(requester_address));
            }

            if account_addresses_match(&requester_address, trimmed, default_port) {
                return Ok(Some(requester_address));
            }

            if let Some(actor_uri) = actor_uri.as_deref()
                && account_addresses_match(actor_uri, trimmed, default_port)
            {
                return Ok(Some(requester_address));
            }
        }

        Ok(None)
    }

    /// Check if follow request exists
    pub async fn has_follow_request(&self, requester_address: &str) -> Result<bool, AppError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM follow_requests WHERE requester_address = ?")
                .bind(requester_address)
                .fetch_one(&self.pool)
                .await?;

        Ok(count > 0)
    }

    /// Check if follow request exists, treating default-port variants as equivalent.
    pub async fn has_follow_request_with_default_port(
        &self,
        requester_address: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let candidates = equivalent_account_address_candidates(requester_address, default_port);
        if candidates.is_empty() {
            return Ok(false);
        }

        let mut query_builder = QueryBuilder::<Sqlite>::new(
            "SELECT COUNT(*) as count FROM follow_requests WHERE LOWER(requester_address) IN (",
        );
        {
            let mut separated = query_builder.separated(", ");
            for candidate in candidates {
                separated.push_bind(candidate.to_ascii_lowercase());
            }
        }
        query_builder.push(")");

        let count: i64 = query_builder
            .build_query_scalar()
            .fetch_one(&self.pool)
            .await?;
        Ok(count > 0)
    }

    /// Get follow request details
    pub async fn get_follow_request(
        &self,
        requester_address: &str,
    ) -> Result<Option<(String, String)>, AppError> {
        let result = sqlx::query_as::<_, (String, String)>(
            "SELECT inbox_uri, uri FROM follow_requests WHERE requester_address = ?",
        )
        .bind(requester_address)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }

    /// Get follow request details including canonical actor URI.
    pub async fn get_follow_request_with_actor_uri(
        &self,
        requester_address: &str,
    ) -> Result<Option<(String, String, Option<String>)>, AppError> {
        let result = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT inbox_uri, uri, actor_uri FROM follow_requests WHERE requester_address = ?",
        )
        .bind(requester_address)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }

    /// Insert follow request
    pub async fn insert_follow_request(
        &self,
        requester_address: &str,
        inbox_uri: &str,
        uri: &str,
    ) -> Result<(), AppError> {
        self.insert_follow_request_with_actor_uri(requester_address, inbox_uri, uri, None)
            .await
    }

    /// Insert follow request with optional canonical actor URI.
    pub async fn insert_follow_request_with_actor_uri(
        &self,
        requester_address: &str,
        inbox_uri: &str,
        uri: &str,
        actor_uri: Option<&str>,
    ) -> Result<(), AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            "INSERT OR REPLACE INTO follow_requests (id, requester_address, inbox_uri, uri, actor_uri, created_at) VALUES (?, ?, ?, ?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(requester_address)
        .bind(inbox_uri)
        .bind(uri)
        .bind(actor_uri)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Accept follow request
    pub async fn accept_follow_request(&self, requester_address: &str) -> Result<bool, AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<bool, AppError> = async {
            let follow_request = sqlx::query_as::<_, (String, String, Option<String>)>(
                "SELECT inbox_uri, uri, actor_uri FROM follow_requests WHERE requester_address = ?",
            )
            .bind(requester_address)
            .fetch_optional(&mut *conn)
            .await?;

            let Some((inbox_uri, uri, actor_uri)) = follow_request else {
                return Ok(false);
            };

            let follower_id = EntityId::new_string();
            sqlx::query(
                "INSERT INTO followers (id, follower_address, actor_uri, inbox_uri, uri, created_at) VALUES (?, ?, ?, ?, ?, datetime('now'))",
            )
            .bind(&follower_id)
            .bind(requester_address)
            .bind(actor_uri)
            .bind(&inbox_uri)
            .bind(&uri)
            .execute(&mut *conn)
            .await?;

            sqlx::query("DELETE FROM follow_requests WHERE requester_address = ?")
                .bind(requester_address)
                .execute(&mut *conn)
                .await?;

            Ok(true)
        }
        .await;

        match result {
            Ok(accepted) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(accepted)
            }
            Err(error) => {
                super::rollback_with_log(&mut conn, "accept_follow_request").await;
                Err(error)
            }
        }
    }

    /// Reject follow request
    pub async fn reject_follow_request(&self, requester_address: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM follow_requests WHERE requester_address = ?")
            .bind(requester_address)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete any follow requests matching the requester address, accounting for
    /// default-port-equivalent address variants.
    pub async fn delete_follow_request_with_default_port(
        &self,
        requester_address: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let candidates = equivalent_account_address_candidates(requester_address, default_port);
        if candidates.is_empty() {
            return Ok(false);
        }

        let mut query_builder = QueryBuilder::<Sqlite>::new(
            "DELETE FROM follow_requests WHERE LOWER(requester_address) IN (",
        );
        {
            let mut separated = query_builder.separated(", ");
            for candidate in candidates {
                separated.push_bind(candidate.to_ascii_lowercase());
            }
        }
        query_builder.push(")");

        let result = query_builder.build().execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delete the specific follow request activity for a requester address,
    /// accounting for default-port-equivalent address variants.
    pub async fn delete_follow_request_by_address_and_uri(
        &self,
        requester_address: &str,
        uri: &str,
        default_port: Option<u16>,
    ) -> Result<bool, AppError> {
        let candidates = equivalent_account_address_candidates(requester_address, default_port);
        if candidates.is_empty() {
            return Ok(false);
        }

        let mut query_builder =
            QueryBuilder::<Sqlite>::new("DELETE FROM follow_requests WHERE uri = ");
        query_builder.push_bind(uri);
        query_builder.push(" AND LOWER(requester_address) IN (");
        {
            let mut separated = query_builder.separated(", ");
            for candidate in candidates {
                separated.push_bind(candidate.to_ascii_lowercase());
            }
        }
        query_builder.push(")");

        let result = query_builder.build().execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }
}
