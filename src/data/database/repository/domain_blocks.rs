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

    /// Get all blocked domains for a specific severity.
    pub async fn get_domains_by_severity(&self, severity: &str) -> Result<Vec<String>, AppError> {
        let domains = sqlx::query_scalar::<_, String>(
            "SELECT domain FROM domain_blocks WHERE severity = ? ORDER BY created_at DESC",
        )
        .bind(severity)
        .fetch_all(&self.pool)
        .await?;

        Ok(domains)
    }

    /// Block a domain
    pub async fn block_domain(&self, domain: &str) -> Result<(), AppError> {
        self.upsert_domain_block(domain, "suspend", true, true, None, None, false)
            .await?;
        Ok(())
    }

    /// Insert or update a Mastodon-compatible domain block configuration.
    pub async fn upsert_domain_block(
        &self,
        domain: &str,
        severity: &str,
        reject_media: bool,
        reject_reports: bool,
        private_comment: Option<&str>,
        public_comment: Option<&str>,
        obfuscate: bool,
    ) -> Result<DomainBlockRecord, AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            r#"
            INSERT INTO domain_blocks (
                id, domain, severity, reject_media, reject_reports,
                private_comment, public_comment, obfuscate, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
            ON CONFLICT(domain) DO UPDATE SET
                severity = excluded.severity,
                reject_media = excluded.reject_media,
                reject_reports = excluded.reject_reports,
                private_comment = excluded.private_comment,
                public_comment = excluded.public_comment,
                obfuscate = excluded.obfuscate
            "#,
        )
        .bind(&id)
        .bind(domain)
        .bind(severity)
        .bind(reject_media)
        .bind(reject_reports)
        .bind(private_comment)
        .bind(public_comment)
        .bind(obfuscate)
        .execute(&self.pool)
        .await?;

        self.get_domain_block_by_domain(domain)
            .await?
            .ok_or_else(|| AppError::Validation("failed to persist domain block".to_string()))
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
    pub async fn get_all_domain_blocks(&self) -> Result<Vec<DomainBlockRecord>, AppError> {
        let blocks = sqlx::query_as::<_, DomainBlockRecord>(
            r#"
            SELECT
                id, domain, severity, reject_media, reject_reports,
                private_comment, public_comment, obfuscate, created_at
            FROM domain_blocks
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(blocks)
    }

    /// Get a domain block by its persisted ID.
    pub async fn get_domain_block_by_id(
        &self,
        id: &str,
    ) -> Result<Option<DomainBlockRecord>, AppError> {
        let block = sqlx::query_as::<_, DomainBlockRecord>(
            r#"
            SELECT
                id, domain, severity, reject_media, reject_reports,
                private_comment, public_comment, obfuscate, created_at
            FROM domain_blocks
            WHERE id = ?
            LIMIT 1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(block)
    }

    /// Get a domain block by normalized domain.
    pub async fn get_domain_block_by_domain(
        &self,
        domain: &str,
    ) -> Result<Option<DomainBlockRecord>, AppError> {
        let block = sqlx::query_as::<_, DomainBlockRecord>(
            r#"
            SELECT
                id, domain, severity, reject_media, reject_reports,
                private_comment, public_comment, obfuscate, created_at
            FROM domain_blocks
            WHERE domain = ?
            LIMIT 1
            "#,
        )
        .bind(domain)
        .fetch_optional(&self.pool)
        .await?;

        Ok(block)
    }

    /// Resolve the first configured domain block matching any candidate domain.
    pub async fn find_domain_block_for_candidates<I, S>(
        &self,
        domains: I,
    ) -> Result<Option<DomainBlockRecord>, AppError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for domain in domains {
            if let Some(block) = self.get_domain_block_by_domain(domain.as_ref()).await? {
                return Ok(Some(block));
            }
        }

        Ok(None)
    }

    /// Delete a domain block by persisted ID.
    pub async fn delete_domain_block_by_id(&self, id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM domain_blocks WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Insert domain block (alias for block_domain)
    pub async fn insert_domain_block(&self, domain: &str) -> Result<(), AppError> {
        self.block_domain(domain).await
    }
}
