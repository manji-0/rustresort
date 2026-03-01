use super::super::*;

impl Database {
    // =========================================================================
    // Settings
    // =========================================================================

    /// Get setting value
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>, AppError> {
        let value = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;

        Ok(value)
    }

    /// Set setting value
    pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), AppError> {
        sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}
