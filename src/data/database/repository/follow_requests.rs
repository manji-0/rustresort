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

    /// Insert follow request
    pub async fn insert_follow_request(
        &self,
        requester_address: &str,
        inbox_uri: &str,
        uri: &str,
    ) -> Result<(), AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            "INSERT OR REPLACE INTO follow_requests (id, requester_address, inbox_uri, uri, created_at) VALUES (?, ?, ?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(requester_address)
        .bind(inbox_uri)
        .bind(uri)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Accept follow request
    pub async fn accept_follow_request(&self, requester_address: &str) -> Result<bool, AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<bool, AppError> = async {
            let follow_request = sqlx::query_as::<_, (String, String)>(
                "SELECT inbox_uri, uri FROM follow_requests WHERE requester_address = ?",
            )
            .bind(requester_address)
            .fetch_optional(&mut *conn)
            .await?;

            let Some((inbox_uri, uri)) = follow_request else {
                return Ok(false);
            };

            let follower_id = EntityId::new_string();
            sqlx::query(
                "INSERT INTO followers (id, follower_address, inbox_uri, uri, created_at) VALUES (?, ?, ?, ?, datetime('now'))",
            )
            .bind(&follower_id)
            .bind(requester_address)
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
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
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
}
