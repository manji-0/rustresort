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

    pub async fn create_filter_keyword(
        &self,
        filter_id: &str,
        keyword: &str,
        whole_word: bool,
    ) -> Result<String, AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            r#"
            INSERT INTO filter_keywords (id, filter_id, keyword, whole_word, created_at)
            VALUES (?, ?, ?, ?, datetime('now'))
            "#,
        )
        .bind(&id)
        .bind(filter_id)
        .bind(keyword)
        .bind(whole_word as i64)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    pub async fn get_filter_keyword(
        &self,
        keyword_id: &str,
    ) -> Result<Option<(String, String, String, bool)>, AppError> {
        let row = sqlx::query_as::<_, (String, String, String, i64)>(
            "SELECT id, filter_id, keyword, whole_word FROM filter_keywords WHERE id = ?",
        )
        .bind(keyword_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row
            .map(|(id, filter_id, keyword, whole_word)| (id, filter_id, keyword, whole_word != 0)))
    }

    pub async fn get_filter_keywords(
        &self,
        filter_id: &str,
    ) -> Result<Vec<(String, String, bool)>, AppError> {
        let rows = sqlx::query_as::<_, (String, String, i64)>(
            "SELECT id, keyword, whole_word FROM filter_keywords WHERE filter_id = ? ORDER BY created_at ASC, id ASC",
        )
        .bind(filter_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, keyword, whole_word)| (id, keyword, whole_word != 0))
            .collect())
    }

    pub async fn replace_filter_keywords(
        &self,
        filter_id: &str,
        keywords: &[(String, bool)],
    ) -> Result<(), AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            sqlx::query("DELETE FROM filter_keywords WHERE filter_id = ?")
                .bind(filter_id)
                .execute(&mut *conn)
                .await?;

            for (keyword, whole_word) in keywords {
                let keyword_id = EntityId::new_string();
                sqlx::query(
                    r#"
                    INSERT INTO filter_keywords (id, filter_id, keyword, whole_word, created_at)
                    VALUES (?, ?, ?, ?, datetime('now'))
                    "#,
                )
                .bind(&keyword_id)
                .bind(filter_id)
                .bind(keyword)
                .bind(*whole_word as i64)
                .execute(&mut *conn)
                .await?;
            }

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => {
                super::rollback_with_log(&mut conn, "replace_filter_keywords").await;
                Err(error)
            }
        }
    }

    pub async fn update_filter_keyword(
        &self,
        keyword_id: &str,
        keyword: &str,
        whole_word: bool,
    ) -> Result<bool, AppError> {
        let result =
            sqlx::query("UPDATE filter_keywords SET keyword = ?, whole_word = ? WHERE id = ?")
                .bind(keyword)
                .bind(whole_word as i64)
                .bind(keyword_id)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_filter_keyword(&self, keyword_id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM filter_keywords WHERE id = ?")
            .bind(keyword_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn create_filter_status(
        &self,
        filter_id: &str,
        status_id: &str,
    ) -> Result<String, AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO filter_statuses (id, filter_id, status_id, created_at)
            VALUES (?, ?, ?, datetime('now'))
            "#,
        )
        .bind(&id)
        .bind(filter_id)
        .bind(status_id)
        .execute(&self.pool)
        .await?;

        let existing = sqlx::query_scalar::<_, String>(
            "SELECT id FROM filter_statuses WHERE filter_id = ? AND status_id = ?",
        )
        .bind(filter_id)
        .bind(status_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(existing)
    }

    pub async fn get_filter_statuses(
        &self,
        filter_id: &str,
    ) -> Result<Vec<(String, String)>, AppError> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT id, status_id FROM filter_statuses WHERE filter_id = ? ORDER BY created_at ASC, id ASC",
        )
        .bind(filter_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn delete_filter_status(&self, id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM filter_statuses WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
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
