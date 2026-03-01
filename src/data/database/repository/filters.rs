use super::super::*;

impl Database {
    // =========================================================================
    // Filters (Phase 2)
    // =========================================================================

    /// Create a filter (v1 API)
    pub async fn create_filter(
        &self,
        phrase: &str,
        context: &str,
        expires_at: Option<&str>,
        irreversible: bool,
        whole_word: bool,
    ) -> Result<String, AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            r#"
            INSERT INTO filters (id, phrase, context, expires_at, irreversible, whole_word, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            "#,
        )
        .bind(&id)
        .bind(phrase)
        .bind(context)
        .bind(expires_at)
        .bind(irreversible as i64)
        .bind(whole_word as i64)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Get filter by ID
    pub async fn get_filter(
        &self,
        id: &str,
    ) -> Result<Option<(String, String, String, Option<String>, bool, bool)>, AppError> {
        let result = sqlx::query_as::<_, (String, String, String, Option<String>, i64, i64)>(
            "SELECT id, phrase, context, expires_at, irreversible, whole_word FROM filters WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(
            |(id, phrase, context, expires_at, irreversible, whole_word)| {
                (
                    id,
                    phrase,
                    context,
                    expires_at,
                    irreversible != 0,
                    whole_word != 0,
                )
            },
        ))
    }

    /// Get all filters
    pub async fn get_all_filters(
        &self,
    ) -> Result<Vec<(String, String, String, Option<String>, bool, bool)>, AppError> {
        let filters = sqlx::query_as::<_, (String, String, String, Option<String>, i64, i64)>(
            "SELECT id, phrase, context, expires_at, irreversible, whole_word FROM filters ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(filters
            .into_iter()
            .map(
                |(id, phrase, context, expires_at, irreversible, whole_word)| {
                    (
                        id,
                        phrase,
                        context,
                        expires_at,
                        irreversible != 0,
                        whole_word != 0,
                    )
                },
            )
            .collect())
    }

    /// Update filter
    pub async fn update_filter(
        &self,
        id: &str,
        phrase: &str,
        context: &str,
        expires_at: Option<&str>,
        irreversible: bool,
        whole_word: bool,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE filters 
            SET phrase = ?, context = ?, expires_at = ?, irreversible = ?, whole_word = ?, updated_at = datetime('now')
            WHERE id = ?
            "#,
        )
        .bind(phrase)
        .bind(context)
        .bind(expires_at)
        .bind(irreversible as i64)
        .bind(whole_word as i64)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete filter
    pub async fn delete_filter(&self, id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM filters WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }
}
