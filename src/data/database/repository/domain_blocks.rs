use super::super::*;

impl Database {
    // =========================================================================
    // Domain blocks
    // =========================================================================

    /// Check if domain is blocked
    pub async fn is_domain_blocked(&self, domain: &str) -> Result<bool, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domain_blocks WHERE domain = ?")
            .bind(domain)
            .fetch_one(&self.pool)
            .await?;

        Ok(count > 0)
    }

    /// Get all blocked domains
    pub async fn get_blocked_domains(&self) -> Result<Vec<String>, AppError> {
        let domains = sqlx::query_scalar::<_, String>(
            "SELECT domain FROM domain_blocks ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(domains)
    }

    /// Block a domain
    pub async fn block_domain(&self, domain: &str) -> Result<(), AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            "INSERT OR IGNORE INTO domain_blocks (id, domain, created_at) VALUES (?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(domain)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Unblock a domain
    pub async fn unblock_domain(&self, domain: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM domain_blocks WHERE domain = ?")
            .bind(domain)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get all domain blocks with details
    pub async fn get_all_domain_blocks(
        &self,
    ) -> Result<Vec<(String, String, chrono::DateTime<chrono::Utc>)>, AppError> {
        let blocks = sqlx::query_as::<_, (String, String, chrono::DateTime<chrono::Utc>)>(
            "SELECT id, domain, created_at FROM domain_blocks ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(blocks)
    }

    /// Insert domain block (alias for block_domain)
    pub async fn insert_domain_block(&self, domain: &str) -> Result<(), AppError> {
        self.block_domain(domain).await
    }
}
