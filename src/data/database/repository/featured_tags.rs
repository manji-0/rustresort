use super::super::*;

impl Database {
    pub async fn list_featured_tags(
        &self,
    ) -> Result<Vec<(String, String, i64, Option<String>)>, AppError> {
        sqlx::query_as::<_, (String, String, i64, Option<String>)>(
            r#"
            SELECT
                ft.id,
                ft.name,
                COUNT(DISTINCT s.id) as statuses_count,
                MAX(s.created_at) as last_status_at
            FROM featured_tags ft
            LEFT JOIN hashtags h ON h.name = ft.name COLLATE NOCASE
            LEFT JOIN status_hashtags sh ON sh.hashtag_id = h.id
            LEFT JOIN statuses s
                ON s.id = sh.status_id
               AND s.is_local = 1
               AND s.visibility IN ('public', 'unlisted')
            GROUP BY ft.id, ft.name
            ORDER BY ft.created_at DESC, ft.name ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn create_featured_tag(
        &self,
        name: &str,
    ) -> Result<(String, String, i64, Option<String>), AppError> {
        let normalized = normalize_featured_tag_name(name)?;
        let id = EntityId::new_string();

        sqlx::query(
            "INSERT OR IGNORE INTO featured_tags (id, name, created_at) VALUES (?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(&normalized)
        .execute(&self.pool)
        .await?;

        self.get_featured_tag_by_name(&normalized)
            .await?
            .ok_or(AppError::NotFound)
    }

    pub async fn get_featured_tag_by_id(
        &self,
        id: &str,
    ) -> Result<Option<(String, String, i64, Option<String>)>, AppError> {
        let rows = self.list_featured_tags().await?;
        Ok(rows.into_iter().find(|(tag_id, _, _, _)| tag_id == id))
    }

    pub async fn get_featured_tag_by_name(
        &self,
        name: &str,
    ) -> Result<Option<(String, String, i64, Option<String>)>, AppError> {
        let normalized = normalize_featured_tag_name(name)?;
        let rows = self.list_featured_tags().await?;
        Ok(rows
            .into_iter()
            .find(|(_, tag_name, _, _)| tag_name.eq_ignore_ascii_case(&normalized)))
    }

    pub async fn delete_featured_tag(&self, id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM featured_tags WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn count_featured_tags(&self) -> Result<i64, AppError> {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM featured_tags")
            .fetch_one(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn suggested_featured_tags(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, String, i64, Option<String>)>, AppError> {
        let featured_names = self
            .list_featured_tags()
            .await?
            .into_iter()
            .map(|(_, name, _, _)| name.to_ascii_lowercase())
            .collect::<std::collections::HashSet<_>>();

        let tags = sqlx::query_as::<_, (String, String, i64, Option<String>)>(
            r#"
            SELECT
                h.id,
                h.name,
                COUNT(DISTINCT s.id) as statuses_count,
                MAX(s.created_at) as last_status_at
            FROM hashtags h
            INNER JOIN status_hashtags sh ON sh.hashtag_id = h.id
            INNER JOIN statuses s
                ON s.id = sh.status_id
               AND s.is_local = 1
               AND s.visibility IN ('public', 'unlisted')
            GROUP BY h.id, h.name
            ORDER BY last_status_at DESC, statuses_count DESC, h.name ASC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(tags
            .into_iter()
            .filter(|(_, name, _, _)| !featured_names.contains(&name.to_ascii_lowercase()))
            .take(limit)
            .collect())
    }
}

fn normalize_featured_tag_name(name: &str) -> Result<String, AppError> {
    let normalized = name.trim().trim_start_matches('#');
    if normalized.is_empty() {
        return Err(AppError::Validation(
            "Validation failed: Tag is invalid".to_string(),
        ));
    }
    if normalized.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(AppError::Validation(
            "Validation failed: Tag is invalid".to_string(),
        ));
    }
    if !normalized
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(AppError::Validation(
            "Validation failed: Tag is invalid".to_string(),
        ));
    }

    Ok(normalized.to_string())
}
