use super::super::*;

impl Database {
    // =========================================================================
    // Favourites / Bookmarks / Reposts
    // =========================================================================

    /// Insert favourite
    pub async fn insert_favourite(&self, status_id: &str) -> Result<String, AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            "INSERT INTO favourites (id, status_id, created_at) VALUES (?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(status_id)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    #[cfg(test)]
    pub async fn set_favourite_created_at_for_test(
        &self,
        status_id: &str,
        created_at: &str,
    ) -> Result<(), AppError> {
        sqlx::query("UPDATE favourites SET created_at = ? WHERE status_id = ?")
            .bind(created_at)
            .bind(status_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Delete favourite
    pub async fn delete_favourite(&self, status_id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM favourites WHERE status_id = ?")
            .bind(status_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get favourite record ID for a status.
    pub async fn get_favourite_id(&self, status_id: &str) -> Result<Option<String>, AppError> {
        let id = sqlx::query_scalar::<_, String>(
            "SELECT id FROM favourites WHERE status_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(status_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(id)
    }

    /// Check if status is favourited
    pub async fn is_favourited(&self, status_id: &str) -> Result<bool, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM favourites WHERE status_id = ?")
            .bind(status_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(count > 0)
    }

    /// Count favourites for a status.
    pub async fn count_favourites(&self, status_id: &str) -> Result<i64, AppError> {
        sqlx::query_scalar(
            r#"
            SELECT
                (SELECT COUNT(*) FROM favourites WHERE status_id = ?)
              + (SELECT COUNT(*) FROM remote_favourites WHERE status_id = ?)
            "#,
        )
        .bind(status_id)
        .bind(status_id)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Get favourited status IDs
    pub async fn get_favourited_status_ids(&self, limit: usize) -> Result<Vec<String>, AppError> {
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT status_id FROM favourites ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(ids)
    }

    /// Get favourited status IDs among the provided IDs
    pub async fn get_favourited_status_ids_batch(
        &self,
        status_ids: &[String],
    ) -> Result<HashSet<String>, AppError> {
        if status_ids.is_empty() {
            return Ok(HashSet::new());
        }

        let mut query_builder =
            QueryBuilder::<Sqlite>::new("SELECT status_id FROM favourites WHERE status_id IN (");
        {
            let mut separated = query_builder.separated(", ");
            for status_id in status_ids {
                separated.push_bind(status_id);
            }
        }
        query_builder.push(")");

        let ids = query_builder
            .build_query_scalar::<String>()
            .fetch_all(&self.pool)
            .await?;

        Ok(ids.into_iter().collect())
    }

    /// Insert bookmark
    pub async fn insert_bookmark(&self, status_id: &str) -> Result<String, AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            "INSERT INTO bookmarks (id, status_id, created_at) VALUES (?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(status_id)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    #[cfg(test)]
    pub async fn set_bookmark_created_at_for_test(
        &self,
        status_id: &str,
        created_at: &str,
    ) -> Result<(), AppError> {
        sqlx::query("UPDATE bookmarks SET created_at = ? WHERE status_id = ?")
            .bind(created_at)
            .bind(status_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Delete bookmark
    pub async fn delete_bookmark(&self, status_id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM bookmarks WHERE status_id = ?")
            .bind(status_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Check if status is bookmarked
    pub async fn is_bookmarked(&self, status_id: &str) -> Result<bool, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookmarks WHERE status_id = ?")
            .bind(status_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(count > 0)
    }

    /// Get bookmarked status IDs
    pub async fn get_bookmarked_status_ids(&self, limit: usize) -> Result<Vec<String>, AppError> {
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT status_id FROM bookmarks ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(ids)
    }

    /// Get bookmarked status IDs among the provided IDs
    pub async fn get_bookmarked_status_ids_batch(
        &self,
        status_ids: &[String],
    ) -> Result<HashSet<String>, AppError> {
        if status_ids.is_empty() {
            return Ok(HashSet::new());
        }

        let mut query_builder =
            QueryBuilder::<Sqlite>::new("SELECT status_id FROM bookmarks WHERE status_id IN (");
        {
            let mut separated = query_builder.separated(", ");
            for status_id in status_ids {
                separated.push_bind(status_id);
            }
        }
        query_builder.push(")");

        let ids = query_builder
            .build_query_scalar::<String>()
            .fetch_all(&self.pool)
            .await?;

        Ok(ids.into_iter().collect())
    }

    /// Get bookmarked statuses with JOIN (optimized, avoids N+1)
    pub async fn get_bookmarked_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let statuses = match max_id {
            Some(max_id) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT s.* FROM statuses s
                    INNER JOIN bookmarks b ON s.id = b.status_id
                    LEFT JOIN bookmarks cb ON cb.status_id = ?
                    WHERE (
                        cb.status_id IS NOT NULL
                        AND (
                            b.created_at < cb.created_at
                            OR (b.created_at = cb.created_at AND s.id < ?)
                        )
                    ) OR (
                        cb.status_id IS NULL
                        AND s.id < ?
                    )
                    ORDER BY b.created_at DESC, s.id DESC
                    LIMIT ?
                    "#,
                )
                .bind(max_id)
                .bind(max_id)
                .bind(max_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT s.* FROM statuses s
                    INNER JOIN bookmarks b ON s.id = b.status_id
                    ORDER BY b.created_at DESC, s.id DESC
                    LIMIT ?
                    "#,
                )
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(statuses)
    }

    /// Get favourited statuses with JOIN (optimized, avoids N+1)
    pub async fn get_favourited_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let statuses = match max_id {
            Some(max_id) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT s.* FROM statuses s
                    INNER JOIN favourites f ON s.id = f.status_id
                    LEFT JOIN favourites cf ON cf.status_id = ?
                    WHERE (
                        cf.status_id IS NOT NULL
                        AND (
                            f.created_at < cf.created_at
                            OR (f.created_at = cf.created_at AND s.id < ?)
                        )
                    ) OR (
                        cf.status_id IS NULL
                        AND s.id < ?
                    )
                    ORDER BY f.created_at DESC, s.id DESC
                    LIMIT ?
                    "#,
                )
                .bind(max_id)
                .bind(max_id)
                .bind(max_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT s.* FROM statuses s
                    INNER JOIN favourites f ON s.id = f.status_id
                    ORDER BY f.created_at DESC, s.id DESC
                    LIMIT ?
                    "#,
                )
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(statuses)
    }

    /// Insert repost
    pub async fn insert_repost(&self, status_id: &str, uri: &str) -> Result<String, AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            "INSERT INTO reposts (id, status_id, uri, created_at) VALUES (?, ?, ?, datetime('now'))"
        )
        .bind(&id)
        .bind(status_id)
        .bind(uri)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Insert or update a remote favourite interaction.
    pub async fn upsert_remote_favourite(
        &self,
        status_id: &str,
        actor_address: &str,
        activity_uri: Option<&str>,
    ) -> Result<(), AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            r#"
            INSERT INTO remote_favourites (id, status_id, actor_address, activity_uri, created_at)
            VALUES (?, ?, ?, ?, datetime('now'))
            ON CONFLICT(status_id, actor_address) DO UPDATE SET
                activity_uri = excluded.activity_uri
            "#,
        )
        .bind(&id)
        .bind(status_id)
        .bind(actor_address)
        .bind(activity_uri)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete a remote favourite by its activity URI.
    pub async fn delete_remote_favourite_by_activity_uri(
        &self,
        activity_uri: &str,
    ) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM remote_favourites WHERE activity_uri = ?")
            .bind(activity_uri)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Get remote favourite actor/status pair by activity URI.
    pub async fn get_remote_favourite_actor_and_status_by_activity_uri(
        &self,
        activity_uri: &str,
    ) -> Result<Option<(String, String)>, AppError> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT actor_address, status_id FROM remote_favourites WHERE activity_uri = ? LIMIT 1",
        )
        .bind(activity_uri)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Delete a remote favourite by actor and status.
    pub async fn delete_remote_favourite_by_actor_and_status(
        &self,
        actor_address: &str,
        status_id: &str,
    ) -> Result<bool, AppError> {
        let result =
            sqlx::query("DELETE FROM remote_favourites WHERE actor_address = ? AND status_id = ?")
                .bind(actor_address)
                .bind(status_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// List remote favourite actor addresses for a status.
    pub async fn list_remote_favourite_actor_addresses(
        &self,
        status_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, AppError> {
        sqlx::query_scalar::<_, String>(
            r#"
            SELECT actor_address
            FROM remote_favourites
            WHERE status_id = ?
            ORDER BY created_at DESC, id DESC
            LIMIT ?
            "#,
        )
        .bind(status_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Delete repost
    pub async fn delete_repost(&self, status_id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM reposts WHERE status_id = ?")
            .bind(status_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get repost activity URI for a status.
    pub async fn get_repost_uri(&self, status_id: &str) -> Result<Option<String>, AppError> {
        let uri = sqlx::query_scalar::<_, String>(
            "SELECT uri FROM reposts WHERE status_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(status_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(uri)
    }

    /// Get a repost row by its Announce activity URI.
    pub async fn get_repost_by_uri(&self, uri: &str) -> Result<Option<Repost>, AppError> {
        sqlx::query_as::<_, Repost>("SELECT * FROM reposts WHERE uri = ? LIMIT 1")
            .bind(uri)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// Get repost rows safe to expose in ActivityPub outbox.
    pub async fn get_local_outbox_reposts(&self, limit: usize) -> Result<Vec<Repost>, AppError> {
        sqlx::query_as::<_, Repost>(
            r#"
            SELECT r.*
            FROM reposts r
            INNER JOIN statuses s ON s.id = r.status_id
            WHERE s.visibility IN ('public', 'unlisted')
            ORDER BY r.created_at DESC, r.id DESC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Count reposts safe to expose in ActivityPub outbox.
    pub async fn count_local_outbox_reposts(&self) -> Result<i64, AppError> {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM reposts r
            INNER JOIN statuses s ON s.id = r.status_id
            WHERE s.visibility IN ('public', 'unlisted')
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Check if status is reposted
    pub async fn is_reposted(&self, status_id: &str) -> Result<bool, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reposts WHERE status_id = ?")
            .bind(status_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(count > 0)
    }

    /// Count reposts for a status.
    pub async fn count_reposts(&self, status_id: &str) -> Result<i64, AppError> {
        sqlx::query_scalar(
            r#"
            SELECT
                (SELECT COUNT(*) FROM reposts WHERE status_id = ?)
              + (SELECT COUNT(*) FROM remote_reposts WHERE status_id = ?)
            "#,
        )
        .bind(status_id)
        .bind(status_id)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Insert or update a remote reblog interaction.
    pub async fn upsert_remote_repost(
        &self,
        status_id: &str,
        actor_address: &str,
        activity_uri: Option<&str>,
    ) -> Result<(), AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            r#"
            INSERT INTO remote_reposts (id, status_id, actor_address, activity_uri, created_at)
            VALUES (?, ?, ?, ?, datetime('now'))
            ON CONFLICT(status_id, actor_address) DO UPDATE SET
                activity_uri = excluded.activity_uri
            "#,
        )
        .bind(&id)
        .bind(status_id)
        .bind(actor_address)
        .bind(activity_uri)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete a remote reblog by its activity URI.
    pub async fn delete_remote_repost_by_activity_uri(
        &self,
        activity_uri: &str,
    ) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM remote_reposts WHERE activity_uri = ?")
            .bind(activity_uri)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Get remote reblog actor/status pair by activity URI.
    pub async fn get_remote_repost_actor_and_status_by_activity_uri(
        &self,
        activity_uri: &str,
    ) -> Result<Option<(String, String)>, AppError> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT actor_address, status_id FROM remote_reposts WHERE activity_uri = ? LIMIT 1",
        )
        .bind(activity_uri)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Delete a remote reblog by actor and status.
    pub async fn delete_remote_repost_by_actor_and_status(
        &self,
        actor_address: &str,
        status_id: &str,
    ) -> Result<bool, AppError> {
        let result =
            sqlx::query("DELETE FROM remote_reposts WHERE actor_address = ? AND status_id = ?")
                .bind(actor_address)
                .bind(status_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// List remote reblog actor addresses for a status.
    pub async fn list_remote_repost_actor_addresses(
        &self,
        status_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, AppError> {
        sqlx::query_scalar::<_, String>(
            r#"
            SELECT actor_address
            FROM remote_reposts
            WHERE status_id = ?
            ORDER BY created_at DESC, id DESC
            LIMIT ?
            "#,
        )
        .bind(status_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Insert status pin marker.
    pub async fn insert_status_pin(&self, status_id: &str) -> Result<(), AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            "INSERT OR IGNORE INTO pinned_statuses (id, status_id, created_at) VALUES (?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(status_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete status pin marker.
    pub async fn delete_status_pin(&self, status_id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM pinned_statuses WHERE status_id = ?")
            .bind(status_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Check whether status is pinned.
    pub async fn is_status_pinned(&self, status_id: &str) -> Result<bool, AppError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pinned_statuses WHERE status_id = ?")
                .bind(status_id)
                .fetch_one(&self.pool)
                .await?;

        Ok(count > 0)
    }

    /// Insert conversation mute marker for a thread URI.
    pub async fn insert_muted_thread(&self, thread_uri: &str) -> Result<(), AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            "INSERT OR IGNORE INTO muted_conversations (id, thread_uri, created_at) VALUES (?, ?, datetime('now'))",
        )
        .bind(&id)
        .bind(thread_uri)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete conversation mute marker for a thread URI.
    pub async fn delete_muted_thread(&self, thread_uri: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM muted_conversations WHERE thread_uri = ?")
            .bind(thread_uri)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Check whether thread URI is muted.
    pub async fn is_thread_muted(&self, thread_uri: &str) -> Result<bool, AppError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM muted_conversations WHERE thread_uri = ?")
                .bind(thread_uri)
                .fetch_one(&self.pool)
                .await?;

        Ok(count > 0)
    }

    /// Get all muted thread URIs.
    pub async fn get_muted_thread_uris(&self) -> Result<HashSet<String>, AppError> {
        let uris = sqlx::query_scalar::<_, String>("SELECT thread_uri FROM muted_conversations")
            .fetch_all(&self.pool)
            .await?;
        Ok(uris.into_iter().collect())
    }
}
