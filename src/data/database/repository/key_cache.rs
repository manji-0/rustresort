use super::super::*;

impl Database {
    /// Load a non-expired cached public key by key ID.
    pub async fn get_cached_public_key(
        &self,
        key_id: &str,
    ) -> Result<Option<PublicKeyCacheEntry>, AppError> {
        let entry = sqlx::query_as::<_, PublicKeyCacheEntry>(
            r#"
            SELECT key_id, pem, expires_at, created_at, updated_at
            FROM public_key_cache
            WHERE key_id = ? AND expires_at > CURRENT_TIMESTAMP
            "#,
        )
        .bind(key_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(entry)
    }

    /// Upsert a cached public key.
    pub async fn upsert_cached_public_key(
        &self,
        key_id: &str,
        pem: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO public_key_cache (key_id, pem, expires_at, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(key_id) DO UPDATE SET
                pem = excluded.pem,
                expires_at = excluded.expires_at,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(key_id)
        .bind(pem)
        .bind(expires_at)
        .bind(chrono::Utc::now())
        .bind(chrono::Utc::now())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete all expired cached public keys.
    pub async fn prune_expired_public_keys(&self) -> Result<u64, AppError> {
        let result =
            sqlx::query("DELETE FROM public_key_cache WHERE expires_at <= CURRENT_TIMESTAMP")
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected())
    }
}
