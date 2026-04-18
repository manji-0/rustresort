use super::super::*;
use crate::data::{ListTimelineQuery, TimelineCursorKey};

impl Database {
    // =========================================================================
    // Status
    // =========================================================================

    /// Get status by ID
    pub async fn get_status(&self, id: &str) -> Result<Option<Status>, AppError> {
        let status = sqlx::query_as::<_, Status>("SELECT * FROM statuses WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(status)
    }

    /// Get status by ActivityPub URI
    pub async fn get_status_by_uri(&self, uri: &str) -> Result<Option<Status>, AppError> {
        let status = sqlx::query_as::<_, Status>("SELECT * FROM statuses WHERE uri = ?")
            .bind(uri)
            .fetch_optional(&self.pool)
            .await?;

        Ok(status)
    }

    async fn status_cursor_by_id(
        &self,
        id: &str,
    ) -> Result<Option<(chrono::DateTime<chrono::Utc>, String)>, AppError> {
        let cursor = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
            "SELECT created_at FROM statuses WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(|created_at| (created_at, id.to_string()));
        Ok(cursor)
    }

    fn push_status_window_clauses(
        query_builder: &mut QueryBuilder<Sqlite>,
        max_cursor: Option<&TimelineCursorKey>,
        min_cursor: Option<&TimelineCursorKey>,
        table_alias: &str,
    ) {
        if let Some(cursor) = max_cursor {
            query_builder.push(format!(" AND ({table_alias}created_at < "));
            query_builder.push_bind(cursor.created_at);
            query_builder.push(format!(" OR ({table_alias}created_at = "));
            query_builder.push_bind(cursor.created_at);
            query_builder.push(format!(" AND {table_alias}id < "));
            query_builder.push_bind(cursor.id.clone());
            query_builder.push("))");
        }

        if let Some(cursor) = min_cursor {
            query_builder.push(format!(" AND ({table_alias}created_at > "));
            query_builder.push_bind(cursor.created_at);
            query_builder.push(format!(" OR ({table_alias}created_at = "));
            query_builder.push_bind(cursor.created_at);
            query_builder.push(format!(" AND {table_alias}id > "));
            query_builder.push_bind(cursor.id.clone());
            query_builder.push("))");
        }
    }

    /// Get local statuses that quote the specified status URI.
    pub async fn get_local_statuses_by_quote_of_uri(
        &self,
        quote_of_uri: &str,
    ) -> Result<Vec<Status>, AppError> {
        let statuses = sqlx::query_as::<_, Status>(
            r#"
            SELECT * FROM statuses
            WHERE quote_of_uri = ? AND is_local = 1
            ORDER BY created_at DESC, id DESC
            "#,
        )
        .bind(quote_of_uri)
        .fetch_all(&self.pool)
        .await?;

        Ok(statuses)
    }

    /// Get recent persisted remote statuses for cache hydration.
    pub async fn get_recent_remote_statuses(&self, limit: usize) -> Result<Vec<Status>, AppError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let statuses = sqlx::query_as::<_, Status>(
            r#"
            SELECT * FROM statuses
            WHERE is_local = 0
              AND persisted_reason NOT IN ('reposted', 'favourited', 'bookmarked')
            ORDER BY created_at DESC, id DESC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(statuses)
    }

    /// Get home timeline statuses from durable storage.
    ///
    /// Returns local statuses plus persisted remote statuses from followed accounts.
    pub async fn get_home_statuses_in_window(
        &self,
        followee_addresses: &[String],
        limit: usize,
        max_cursor: Option<&TimelineCursorKey>,
        min_cursor: Option<&TimelineCursorKey>,
    ) -> Result<Vec<Status>, AppError> {
        let mut remote_candidates = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for address in followee_addresses {
            let lowered = address.trim().to_ascii_lowercase();
            if !lowered.is_empty() && seen.insert(lowered.clone()) {
                remote_candidates.push(lowered);
            }
        }

        let mut query_builder = QueryBuilder::<Sqlite>::new(
            "SELECT DISTINCT s.* FROM statuses s WHERE (s.is_local = 1",
        );
        if !remote_candidates.is_empty() {
            query_builder.push(
                " OR (s.is_local = 0 AND s.account_address <> '' AND LOWER(s.account_address) IN (",
            );
            {
                let mut separated = query_builder.separated(", ");
                for candidate in remote_candidates {
                    separated.push_bind(candidate);
                }
            }
            query_builder.push("))");
        }
        query_builder.push(")");
        Self::push_status_window_clauses(&mut query_builder, max_cursor, min_cursor, "s.");
        query_builder.push(" ORDER BY s.created_at DESC, s.id DESC LIMIT ");
        query_builder.push_bind(limit as i64);

        query_builder
            .build_query_as::<Status>()
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// Get public timeline statuses from durable storage.
    ///
    /// Remote statuses persisted only for local interactions must not become
    /// timeline-visible after restart.
    pub async fn get_public_statuses_in_window(
        &self,
        limit: usize,
        max_cursor: Option<&TimelineCursorKey>,
        min_cursor: Option<&TimelineCursorKey>,
    ) -> Result<Vec<Status>, AppError> {
        let mut query_builder = QueryBuilder::<Sqlite>::new(
            "SELECT s.* FROM statuses s WHERE s.visibility = 'public' AND (s.is_local = 1 OR s.persisted_reason NOT IN ('reposted', 'favourited', 'bookmarked'))",
        );
        Self::push_status_window_clauses(&mut query_builder, max_cursor, min_cursor, "s.");
        query_builder.push(" ORDER BY s.created_at DESC, s.id DESC LIMIT ");
        query_builder.push_bind(limit as i64);

        query_builder
            .build_query_as::<Status>()
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// Get statuses authored by a specific account in a pagination window.
    pub async fn get_statuses_by_account_address_in_window(
        &self,
        account_address: &str,
        default_port: Option<u16>,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let candidates = equivalent_account_address_candidates(account_address, default_port);
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let lowered_candidates = candidates
            .iter()
            .map(|candidate| candidate.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let placeholders = vec!["?"; lowered_candidates.len()].join(", ");
        let mut query =
            format!("SELECT * FROM statuses WHERE LOWER(account_address) IN ({placeholders})");
        let max_cursor = match max_id {
            Some(id) => self.status_cursor_by_id(id).await?,
            None => None,
        };
        let min_cursor = match min_id {
            Some(id) => self.status_cursor_by_id(id).await?,
            None => None,
        };
        if max_cursor.is_some() {
            query.push_str(" AND (created_at < ? OR (created_at = ? AND id < ?))");
        }
        if min_cursor.is_some() {
            query.push_str(" AND (created_at > ? OR (created_at = ? AND id > ?))");
        }
        query.push_str(" ORDER BY created_at DESC, id DESC LIMIT ?");

        let mut builder = sqlx::query_as::<_, Status>(&query);
        for candidate in &lowered_candidates {
            builder = builder.bind(candidate);
        }
        if let Some((created_at, id)) = max_cursor {
            builder = builder.bind(created_at).bind(created_at).bind(id);
        }
        if let Some((created_at, id)) = min_cursor {
            builder = builder.bind(created_at).bind(created_at).bind(id);
        }

        let statuses = builder.bind(limit as i64).fetch_all(&self.pool).await?;
        Ok(statuses)
    }

    /// Resolve thread root URI by walking the reply chain from a status.
    ///
    /// Returns the top-most known ancestor URI, or an unknown parent URI when
    /// the chain leaves local persistence.
    pub async fn resolve_thread_root_uri(&self, status: &Status) -> Result<String, AppError> {
        // Resolve the reply chain in one SQL statement so reads observe one snapshot.
        let thread_uri = sqlx::query_scalar::<_, String>(
            r#"
            WITH RECURSIVE thread(uri, parent_uri, visited, depth) AS (
                SELECT ?, ?, printf('|%s|', ?), 0
                UNION ALL
                SELECT
                    COALESCE(parent.uri, thread.parent_uri),
                    CASE
                        WHEN parent.uri IS NULL THEN NULL
                        ELSE parent.in_reply_to_uri
                    END,
                    thread.visited || COALESCE(parent.uri, thread.parent_uri) || '|',
                    thread.depth + 1
                FROM thread
                LEFT JOIN statuses AS parent ON parent.uri = thread.parent_uri
                WHERE thread.parent_uri IS NOT NULL
                  AND instr(
                        thread.visited,
                        printf('|%s|', COALESCE(parent.uri, thread.parent_uri))
                  ) = 0
            )
            SELECT uri
            FROM thread
            ORDER BY depth DESC
            LIMIT 1
            "#,
        )
        .bind(&status.uri)
        .bind(&status.in_reply_to_uri)
        .bind(&status.uri)
        .fetch_one(&self.pool)
        .await?;

        Ok(thread_uri)
    }

    /// Get replies for a given parent status URI.
    pub async fn get_status_replies(&self, in_reply_to_uri: &str) -> Result<Vec<Status>, AppError> {
        let replies = sqlx::query_as::<_, Status>(
            r#"
            SELECT * FROM statuses
            WHERE in_reply_to_uri = ?
            ORDER BY created_at ASC, id ASC
            "#,
        )
        .bind(in_reply_to_uri)
        .fetch_all(&self.pool)
        .await?;

        Ok(replies)
    }

    /// Get replies for a given parent status URI, capped at `limit`.
    pub async fn get_status_replies_limited(
        &self,
        in_reply_to_uri: &str,
        limit: usize,
    ) -> Result<Vec<Status>, AppError> {
        let replies = sqlx::query_as::<_, Status>(
            r#"
            SELECT * FROM statuses
            WHERE in_reply_to_uri = ?
            ORDER BY created_at ASC, id ASC
            LIMIT ?
            "#,
        )
        .bind(in_reply_to_uri)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(replies)
    }

    /// Get multiple statuses by URIs (batch operation to avoid N+1)
    pub async fn get_statuses_by_uris(&self, uris: &[String]) -> Result<Vec<Status>, AppError> {
        if uris.is_empty() {
            return Ok(vec![]);
        }

        // SQLiteのIN句には制限があるため、チャンク化して処理
        let mut all_statuses = Vec::new();

        for chunk in uris.chunks(100) {
            let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");

            let query = format!("SELECT * FROM statuses WHERE uri IN ({})", placeholders);

            let mut query_builder = sqlx::query_as::<_, Status>(&query);
            for uri in chunk {
                query_builder = query_builder.bind(uri);
            }

            let statuses = query_builder.fetch_all(&self.pool).await?;
            all_statuses.extend(statuses);
        }

        Ok(all_statuses)
    }

    async fn replace_status_hashtags_in_connection(
        &self,
        conn: &mut sqlx::pool::PoolConnection<Sqlite>,
        status_id: &str,
        content: &str,
    ) -> Result<(), AppError> {
        sqlx::query("DELETE FROM status_hashtags WHERE status_id = ?")
            .bind(status_id)
            .execute(&mut **conn)
            .await?;

        let hashtags = extract_hashtags_from_content(content);
        for hashtag in hashtags {
            let hashtag_id = sqlx::query_scalar::<_, String>(
                r#"
                INSERT INTO hashtags (id, name, created_at)
                VALUES (?, ?, datetime('now'))
                ON CONFLICT(name) DO UPDATE SET name = excluded.name
                RETURNING id
                "#,
            )
            .bind(EntityId::new_string())
            .bind(&hashtag)
            .fetch_one(&mut **conn)
            .await?;

            let status_hashtag_id = EntityId::new_string();
            sqlx::query(
                "INSERT OR IGNORE INTO status_hashtags (id, status_id, hashtag_id, created_at) VALUES (?, ?, ?, datetime('now'))",
            )
            .bind(&status_hashtag_id)
            .bind(status_id)
            .bind(&hashtag_id)
            .execute(&mut **conn)
            .await?;
        }

        Ok(())
    }

    /// Insert a new status
    pub async fn insert_status(&self, status: &Status) -> Result<(), AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            sqlx::query(
                r#"
                INSERT INTO statuses (
                    id, uri, content, content_warning, visibility, language,
                    account_address, is_local, in_reply_to_uri, boost_of_uri, quote_of_uri,
                    persisted_reason, created_at, fetched_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&status.id)
            .bind(&status.uri)
            .bind(&status.content)
            .bind(&status.content_warning)
            .bind(status.visibility)
            .bind(&status.language)
            .bind(&status.account_address)
            .bind(status.is_local)
            .bind(&status.in_reply_to_uri)
            .bind(&status.boost_of_uri)
            .bind(&status.quote_of_uri)
            .bind(status.persisted_reason)
            .bind(status.created_at)
            .bind(status.fetched_at)
            .execute(&mut *conn)
            .await?;

            self.replace_status_hashtags_in_connection(&mut conn, &status.id, &status.content)
                .await?;

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => {
                super::rollback_with_log(&mut conn, "insert_status").await;
                Err(error)
            }
        }
    }

    /// Insert a new status and attach media atomically.
    pub async fn insert_status_with_media(
        &self,
        status: &Status,
        media_ids: &[String],
    ) -> Result<(), AppError> {
        self.insert_status_with_media_and_poll(status, media_ids, None)
            .await
    }

    /// Insert a new status with optional media and poll atomically.
    pub async fn insert_status_with_media_and_poll(
        &self,
        status: &Status,
        media_ids: &[String],
        poll: Option<(&[String], i64, bool, bool)>,
    ) -> Result<(), AppError> {
        if media_ids.is_empty() && poll.is_none() {
            return self.insert_status(status).await;
        }

        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            sqlx::query(
                r#"
                INSERT INTO statuses (
                    id, uri, content, content_warning, visibility, language,
                    account_address, is_local, in_reply_to_uri, boost_of_uri, quote_of_uri,
                    persisted_reason, created_at, fetched_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&status.id)
            .bind(&status.uri)
            .bind(&status.content)
            .bind(&status.content_warning)
            .bind(status.visibility)
            .bind(&status.language)
            .bind(&status.account_address)
            .bind(status.is_local)
            .bind(&status.in_reply_to_uri)
            .bind(&status.boost_of_uri)
            .bind(&status.quote_of_uri)
            .bind(status.persisted_reason)
            .bind(status.created_at)
            .bind(status.fetched_at)
            .execute(&mut *conn)
            .await?;

            self.replace_status_hashtags_in_connection(&mut conn, &status.id, &status.content)
                .await?;

            for media_id in media_ids {
                let updated = sqlx::query(
                    "UPDATE media_attachments SET status_id = ? WHERE id = ? AND status_id IS NULL",
                )
                .bind(&status.id)
                .bind(media_id)
                .execute(&mut *conn)
                .await?;

                if updated.rows_affected() == 0 {
                    return Err(AppError::Validation(format!(
                        "media attachment is unavailable: {}",
                        media_id
                    )));
                }
            }

            if let Some((poll_options, expires_in, multiple, hide_totals)) = poll {
                let poll_id = EntityId::new_string();
                let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in);
                sqlx::query(
                    r#"
                    INSERT INTO polls (id, status_id, expires_at, expired, multiple, hide_totals, votes_count, voters_count, created_at)
                    VALUES (?, ?, ?, 0, ?, ?, 0, 0, datetime('now'))
                    "#,
                )
                .bind(&poll_id)
                .bind(&status.id)
                .bind(expires_at.to_rfc3339())
                .bind(multiple as i64)
                .bind(hide_totals as i64)
                .execute(&mut *conn)
                .await?;

                for (index, option) in poll_options.iter().enumerate() {
                    let option_id = EntityId::new_string();
                    sqlx::query(
                        r#"
                        INSERT INTO poll_options (id, poll_id, title, votes_count, option_index, created_at)
                        VALUES (?, ?, ?, 0, ?, datetime('now'))
                        "#,
                    )
                    .bind(&option_id)
                    .bind(&poll_id)
                    .bind(option)
                    .bind(index as i64)
                    .execute(&mut *conn)
                    .await?;
                }
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
                super::rollback_with_log(&mut conn, "insert_status_with_media_and_poll").await;
                Err(error)
            }
        }
    }

    /// Update an existing status
    pub async fn update_status(&self, status: &Status) -> Result<(), AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            sqlx::query(
                r#"
                UPDATE statuses
                SET content = ?, content_warning = ?, visibility = ?, language = ?,
                    in_reply_to_uri = ?, boost_of_uri = ?, quote_of_uri = ?, persisted_reason = ?, fetched_at = ?
                WHERE id = ?
                "#,
            )
            .bind(&status.content)
            .bind(&status.content_warning)
            .bind(status.visibility)
            .bind(&status.language)
            .bind(&status.in_reply_to_uri)
            .bind(&status.boost_of_uri)
            .bind(&status.quote_of_uri)
            .bind(status.persisted_reason)
            .bind(status.fetched_at)
            .bind(&status.id)
            .execute(&mut *conn)
            .await?;

            self.replace_status_hashtags_in_connection(&mut conn, &status.id, &status.content)
                .await?;

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => {
                super::rollback_with_log(&mut conn, "update_status").await;
                Err(error)
            }
        }
    }

    /// Atomically snapshot previous status content then apply updated fields.
    pub async fn update_status_with_edit_snapshot(
        &self,
        previous: &Status,
        updated: &Status,
    ) -> Result<(), AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            let edit_id = EntityId::new_string();
            sqlx::query(
                r#"
                INSERT INTO status_edits (id, status_id, content, content_warning, created_at)
                VALUES (?, ?, ?, ?, datetime('now'))
                "#,
            )
            .bind(edit_id)
            .bind(&previous.id)
            .bind(&previous.content)
            .bind(&previous.content_warning)
            .execute(&mut *conn)
            .await?;

            let update_result = sqlx::query(
                r#"
                UPDATE statuses
                SET content = ?, content_warning = ?, visibility = ?, language = ?,
                    in_reply_to_uri = ?, boost_of_uri = ?, quote_of_uri = ?, persisted_reason = ?, fetched_at = ?
                WHERE id = ?
                "#,
            )
            .bind(&updated.content)
            .bind(&updated.content_warning)
            .bind(updated.visibility)
            .bind(&updated.language)
            .bind(&updated.in_reply_to_uri)
            .bind(&updated.boost_of_uri)
            .bind(&updated.quote_of_uri)
            .bind(updated.persisted_reason)
            .bind(updated.fetched_at)
            .bind(&updated.id)
            .execute(&mut *conn)
            .await?;
            if update_result.rows_affected() != 1 {
                return Err(AppError::NotFound);
            }

            self.replace_status_hashtags_in_connection(&mut conn, &updated.id, &updated.content)
                .await?;

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => {
                super::rollback_with_log(&mut conn, "update_status_with_edit_snapshot").await;
                Err(error)
            }
        }
    }

    /// Atomically replace status media (optional), snapshot previous content,
    /// and apply updated status fields.
    pub async fn update_status_with_edit_snapshot_and_media(
        &self,
        previous: &Status,
        updated: &Status,
        media_ids: Option<&[String]>,
        media_attachments_json: Option<&str>,
        poll_json: Option<&str>,
        quote_json: Option<&str>,
    ) -> Result<(), AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            if let Some(media_ids) = media_ids {
                if media_ids.is_empty() {
                    sqlx::query("UPDATE media_attachments SET status_id = NULL WHERE status_id = ?")
                        .bind(&updated.id)
                        .execute(&mut *conn)
                        .await?;
                } else {
                    let placeholders = std::iter::repeat_n("?", media_ids.len())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let sql = format!(
                        "UPDATE media_attachments SET status_id = NULL WHERE status_id = ? AND id NOT IN ({})",
                        placeholders
                    );
                    let mut query = sqlx::query(&sql).bind(&updated.id);
                    for media_id in media_ids {
                        query = query.bind(media_id);
                    }
                    query.execute(&mut *conn).await?;
                }

                for media_id in media_ids {
                    let attach_result = sqlx::query(
                        "UPDATE media_attachments SET status_id = ? WHERE id = ? AND (status_id IS NULL OR status_id = ?)",
                    )
                    .bind(&updated.id)
                    .bind(media_id)
                    .bind(&updated.id)
                    .execute(&mut *conn)
                    .await?;

                    if attach_result.rows_affected() == 0 {
                        return Err(AppError::Validation(format!(
                            "media attachment is already attached to another status: {}",
                            media_id
                        )));
                    }
                }
            }

            let edit_id = EntityId::new_string();
            sqlx::query(
                r#"
                INSERT INTO status_edits (
                    id, status_id, content, content_warning,
                    media_attachments_json, poll_json, quote_json, created_at
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'))
                "#,
            )
            .bind(edit_id)
            .bind(&previous.id)
            .bind(&previous.content)
            .bind(&previous.content_warning)
            .bind(media_attachments_json)
            .bind(poll_json)
            .bind(quote_json)
            .execute(&mut *conn)
            .await?;

            let update_result = sqlx::query(
                r#"
                UPDATE statuses
                SET content = ?, content_warning = ?, visibility = ?, language = ?,
                    in_reply_to_uri = ?, boost_of_uri = ?, quote_of_uri = ?, persisted_reason = ?, fetched_at = ?
                WHERE id = ?
                "#,
            )
            .bind(&updated.content)
            .bind(&updated.content_warning)
            .bind(updated.visibility)
            .bind(&updated.language)
            .bind(&updated.in_reply_to_uri)
            .bind(&updated.boost_of_uri)
            .bind(&updated.quote_of_uri)
            .bind(updated.persisted_reason)
            .bind(updated.fetched_at)
            .bind(&updated.id)
            .execute(&mut *conn)
            .await?;
            if update_result.rows_affected() != 1 {
                return Err(AppError::NotFound);
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
                super::rollback_with_log(&mut conn, "update_status_with_edit_snapshot_and_media")
                    .await;
                Err(error)
            }
        }
    }

    /// Insert a status edit-history snapshot.
    pub async fn insert_status_edit(
        &self,
        status_id: &str,
        content: &str,
        content_warning: Option<&str>,
        media_attachments_json: Option<&str>,
        poll_json: Option<&str>,
        quote_json: Option<&str>,
    ) -> Result<String, AppError> {
        let edit_id = EntityId::new_string();
        sqlx::query(
            r#"
            INSERT INTO status_edits (
                id, status_id, content, content_warning,
                media_attachments_json, poll_json, quote_json, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'))
            "#,
        )
        .bind(&edit_id)
        .bind(status_id)
        .bind(content)
        .bind(content_warning)
        .bind(media_attachments_json)
        .bind(poll_json)
        .bind(quote_json)
        .execute(&self.pool)
        .await?;

        Ok(edit_id)
    }

    /// Get status edit-history snapshots ordered by newest first.
    pub async fn get_status_edits(
        &self,
        status_id: &str,
        limit: usize,
    ) -> Result<
        Vec<(
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            DateTime<Utc>,
        )>,
        AppError,
    > {
        let edits = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                DateTime<Utc>,
            ),
        >(
            r#"
            SELECT
                id,
                content,
                content_warning,
                media_attachments_json,
                poll_json,
                quote_json,
                created_at
            FROM status_edits
            WHERE status_id = ?
            ORDER BY created_at DESC, id DESC
            LIMIT ?
            "#,
        )
        .bind(status_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(edits)
    }

    /// Count replies by parent status URI.
    pub async fn count_replies_by_uri(&self, status_uri: &str) -> Result<i64, AppError> {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM statuses WHERE in_reply_to_uri = ?")
            .bind(status_uri)
            .fetch_one(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// Count quotes targeting a status URI.
    pub async fn count_quotes_by_uri(&self, status_uri: &str) -> Result<i64, AppError> {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM statuses WHERE quote_of_uri = ?")
            .bind(status_uri)
            .fetch_one(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// Count local public/unlisted statuses safe to expose in ActivityPub outbox.
    pub async fn count_local_outbox_statuses(&self) -> Result<i64, AppError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM statuses WHERE is_local = 1 AND visibility IN ('public', 'unlisted')",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Return the timestamp of the most recent stored edit for a status.
    pub async fn get_latest_status_edit_at(
        &self,
        status_id: &str,
    ) -> Result<Option<DateTime<Utc>>, AppError> {
        sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
            "SELECT MAX(created_at) FROM status_edits WHERE status_id = ?",
        )
        .bind(status_id)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Delete status by ID
    pub async fn delete_status(&self, id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM statuses WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get a cached idempotency response for an endpoint and key.
    pub async fn get_idempotency_response(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<Option<serde_json::Value>, AppError> {
        let response_json = sqlx::query_scalar::<_, Option<String>>(
            "SELECT response_json FROM idempotency_keys WHERE endpoint = ? AND key = ?",
        )
        .bind(endpoint)
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await?
        .flatten();

        response_json
            .map(|raw| {
                serde_json::from_str::<serde_json::Value>(&raw).map_err(|error| {
                    AppError::serialization("idempotency response deserialization", error)
                })
            })
            .transpose()
    }

    /// Try to reserve an idempotency key for processing.
    ///
    /// Returns `true` when this request successfully reserved the key and should
    /// proceed, or `false` when another request already owns/owned the key.
    pub async fn reserve_idempotency_key(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            INSERT INTO idempotency_keys (endpoint, key, response_json, created_at)
            VALUES (?, ?, NULL, datetime('now'))
            ON CONFLICT(endpoint, key) DO UPDATE
            SET response_json = NULL, created_at = datetime('now')
            WHERE idempotency_keys.response_json IS NULL
              AND idempotency_keys.created_at < datetime('now', '-5 minutes')
            "#,
        )
        .bind(endpoint)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Store an idempotency response payload for an endpoint and key.
    pub async fn store_idempotency_response(
        &self,
        endpoint: &str,
        idempotency_key: &str,
        response: &serde_json::Value,
    ) -> Result<(), AppError> {
        let response_json = serde_json::to_string(response).map_err(|error| {
            AppError::serialization("idempotency response serialization", error)
        })?;

        let result = sqlx::query(
            "UPDATE idempotency_keys SET response_json = ? WHERE endpoint = ? AND key = ?",
        )
        .bind(&response_json)
        .bind(endpoint)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            sqlx::query(
                "INSERT OR IGNORE INTO idempotency_keys (endpoint, key, response_json) VALUES (?, ?, ?)",
            )
            .bind(endpoint)
            .bind(idempotency_key)
            .bind(&response_json)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Delete a pending idempotency reservation with no stored response.
    pub async fn clear_pending_idempotency_key(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<(), AppError> {
        sqlx::query(
            "DELETE FROM idempotency_keys WHERE endpoint = ? AND key = ? AND response_json IS NULL",
        )
        .bind(endpoint)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn backdate_pending_idempotency_key_for_test(
        &self,
        endpoint: &str,
        idempotency_key: &str,
        minutes: i64,
    ) -> Result<(), AppError> {
        let modifier = format!("-{} minutes", minutes);
        sqlx::query(
            "UPDATE idempotency_keys SET created_at = datetime('now', ?) WHERE endpoint = ? AND key = ? AND response_json IS NULL",
        )
        .bind(modifier)
        .bind(endpoint)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get user's own statuses (paginated)
    ///
    /// # Arguments
    /// * `limit` - Maximum number of results
    /// * `max_id` - Return statuses older than this ID (for pagination)
    pub async fn get_local_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let statuses = if let Some(max_id) = max_id {
            sqlx::query_as::<_, Status>(
                r#"
                SELECT * FROM statuses 
                WHERE is_local = 1 AND id < ?
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(max_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, Status>(
                r#"
                SELECT * FROM statuses 
                WHERE is_local = 1
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(statuses)
    }

    /// Get user's own statuses (paginated) with optional min/max ID window
    ///
    /// # Arguments
    /// * `limit` - Maximum number of results
    /// * `max_id` - Return statuses older than this ID (exclusive)
    /// * `min_id` - Return statuses newer than this ID (exclusive)
    pub async fn get_local_statuses_in_window(
        &self,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let max_cursor = match max_id {
            Some(id) => self.status_cursor_by_id(id).await?,
            None => None,
        };
        let min_cursor = match min_id {
            Some(id) => self.status_cursor_by_id(id).await?,
            None => None,
        };

        let statuses = match (max_cursor, min_cursor) {
            (Some((max_created_at, max_id)), Some((min_created_at, min_id))) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT * FROM statuses 
                    WHERE is_local = 1
                      AND (created_at < ? OR (created_at = ? AND id < ?))
                      AND (created_at > ? OR (created_at = ? AND id > ?))
                    ORDER BY created_at DESC, id DESC
                    LIMIT ?
                    "#,
                )
                .bind(max_created_at)
                .bind(max_created_at)
                .bind(max_id)
                .bind(min_created_at)
                .bind(min_created_at)
                .bind(min_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (Some((max_created_at, max_id)), None) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT * FROM statuses 
                    WHERE is_local = 1
                      AND (created_at < ? OR (created_at = ? AND id < ?))
                    ORDER BY created_at DESC, id DESC
                    LIMIT ?
                    "#,
                )
                .bind(max_created_at)
                .bind(max_created_at)
                .bind(max_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, Some((min_created_at, min_id))) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT * FROM statuses 
                    WHERE is_local = 1
                      AND (created_at > ? OR (created_at = ? AND id > ?))
                    ORDER BY created_at DESC, id DESC
                    LIMIT ?
                    "#,
                )
                .bind(min_created_at)
                .bind(min_created_at)
                .bind(min_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, None) => self.get_local_statuses(limit, None).await?,
        };

        Ok(statuses)
    }

    /// Get user's own public statuses (paginated)
    ///
    /// # Arguments
    /// * `limit` - Maximum number of results
    /// * `max_id` - Return statuses older than this ID (for pagination)
    pub async fn get_local_public_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let max_cursor = match max_id {
            Some(id) => self.status_cursor_by_id(id).await?,
            None => None,
        };
        let min_cursor = match min_id {
            Some(id) => self.status_cursor_by_id(id).await?,
            None => None,
        };

        let statuses = match (max_cursor, min_cursor) {
            (Some((max_created_at, max_id)), Some((min_created_at, min_id))) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT * FROM statuses 
                    WHERE is_local = 1
                      AND visibility = 'public'
                      AND (created_at < ? OR (created_at = ? AND id < ?))
                      AND (created_at > ? OR (created_at = ? AND id > ?))
                    ORDER BY created_at DESC, id DESC
                    LIMIT ?
                    "#,
                )
                .bind(max_created_at)
                .bind(max_created_at)
                .bind(max_id)
                .bind(min_created_at)
                .bind(min_created_at)
                .bind(min_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (Some((max_created_at, max_id)), None) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT * FROM statuses 
                    WHERE is_local = 1
                      AND visibility = 'public'
                      AND (created_at < ? OR (created_at = ? AND id < ?))
                    ORDER BY created_at DESC, id DESC
                    LIMIT ?
                    "#,
                )
                .bind(max_created_at)
                .bind(max_created_at)
                .bind(max_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, Some((min_created_at, min_id))) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT * FROM statuses 
                    WHERE is_local = 1
                      AND visibility = 'public'
                      AND (created_at > ? OR (created_at = ? AND id > ?))
                    ORDER BY created_at DESC, id DESC
                    LIMIT ?
                    "#,
                )
                .bind(min_created_at)
                .bind(min_created_at)
                .bind(min_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, None) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT * FROM statuses 
                    WHERE is_local = 1 AND visibility = 'public'
                    ORDER BY created_at DESC, id DESC
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

    /// Get public statuses that contain the specified hashtag.
    pub async fn get_statuses_by_hashtag_in_window(
        &self,
        hashtag: &str,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let hashtag = hashtag.trim().trim_start_matches('#');
        if hashtag.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let max_cursor = match max_id {
            Some(id) => self.status_cursor_by_id(id).await?,
            None => None,
        };
        let min_cursor = match min_id {
            Some(id) => self.status_cursor_by_id(id).await?,
            None => None,
        };

        let statuses = match (max_cursor, min_cursor) {
            (Some((max_created_at, max_id)), Some((min_created_at, min_id))) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT s.*
                    FROM statuses s
                    INNER JOIN status_hashtags sh ON sh.status_id = s.id
                    INNER JOIN hashtags h ON h.id = sh.hashtag_id
                    WHERE h.name = ? COLLATE NOCASE
                      AND s.visibility = 'public'
                      AND (s.created_at < ? OR (s.created_at = ? AND s.id < ?))
                      AND (s.created_at > ? OR (s.created_at = ? AND s.id > ?))
                    ORDER BY s.created_at DESC, s.id DESC
                    LIMIT ?
                    "#,
                )
                .bind(hashtag)
                .bind(max_created_at)
                .bind(max_created_at)
                .bind(max_id)
                .bind(min_created_at)
                .bind(min_created_at)
                .bind(min_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (Some((max_created_at, max_id)), None) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT s.*
                    FROM statuses s
                    INNER JOIN status_hashtags sh ON sh.status_id = s.id
                    INNER JOIN hashtags h ON h.id = sh.hashtag_id
                    WHERE h.name = ? COLLATE NOCASE
                      AND s.visibility = 'public'
                      AND (s.created_at < ? OR (s.created_at = ? AND s.id < ?))
                    ORDER BY s.created_at DESC, s.id DESC
                    LIMIT ?
                    "#,
                )
                .bind(hashtag)
                .bind(max_created_at)
                .bind(max_created_at)
                .bind(max_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, Some((min_created_at, min_id))) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT s.*
                    FROM statuses s
                    INNER JOIN status_hashtags sh ON sh.status_id = s.id
                    INNER JOIN hashtags h ON h.id = sh.hashtag_id
                    WHERE h.name = ? COLLATE NOCASE
                      AND s.visibility = 'public'
                      AND (s.created_at > ? OR (s.created_at = ? AND s.id > ?))
                    ORDER BY s.created_at DESC, s.id DESC
                    LIMIT ?
                    "#,
                )
                .bind(hashtag)
                .bind(min_created_at)
                .bind(min_created_at)
                .bind(min_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
            (None, None) => {
                sqlx::query_as::<_, Status>(
                    r#"
                    SELECT s.*
                    FROM statuses s
                    INNER JOIN status_hashtags sh ON sh.status_id = s.id
                    INNER JOIN hashtags h ON h.id = sh.hashtag_id
                    WHERE h.name = ? COLLATE NOCASE
                      AND s.visibility = 'public'
                    ORDER BY s.created_at DESC, s.id DESC
                    LIMIT ?
                    "#,
                )
                .bind(hashtag)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(statuses)
    }

    /// Get statuses for a list timeline by matching account addresses.
    pub async fn get_list_timeline_statuses_in_window(
        &self,
        query: &ListTimelineQuery,
    ) -> Result<Vec<Status>, AppError> {
        if query.limit == 0 {
            return Ok(Vec::new());
        }

        let list_accounts = self.get_list_accounts(query.list_id.as_str()).await?;
        if list_accounts.is_empty() {
            return Ok(Vec::new());
        }

        let mut include_local_statuses = false;
        let mut remote_candidates = Vec::new();
        let mut seen_remote = HashSet::new();

        for account_address in list_accounts {
            if account_address.eq_ignore_ascii_case(query.local_account_address.as_str())
                || account_address == query.local_account_id
            {
                include_local_statuses = true;
                continue;
            }

            for candidate in
                equivalent_account_address_candidates(&account_address, query.default_port)
            {
                let lowered = candidate.to_ascii_lowercase();
                if seen_remote.insert(lowered.clone()) {
                    remote_candidates.push(lowered);
                }
            }
        }

        if remote_candidates.is_empty() && !include_local_statuses {
            return Ok(Vec::new());
        }

        let mut query_builder =
            QueryBuilder::<Sqlite>::new("SELECT DISTINCT s.* FROM statuses s WHERE (");
        let mut has_clause = false;

        if !remote_candidates.is_empty() {
            has_clause = true;
            query_builder.push("(s.account_address <> '' AND LOWER(s.account_address) IN (");
            {
                let mut separated = query_builder.separated(", ");
                for candidate in remote_candidates {
                    separated.push_bind(candidate);
                }
            }
            query_builder.push("))");
        }

        if include_local_statuses {
            if has_clause {
                query_builder.push(" OR ");
            }
            query_builder.push("(s.is_local = 1 AND s.account_address = '')");
        }

        query_builder.push(")");
        query_builder.push(" AND s.visibility <> 'direct'");

        let max_cursor = match query.max_id.as_deref() {
            Some(id) => self.status_cursor_by_id(id).await?,
            None => None,
        };
        let min_cursor = match query.min_id.as_deref() {
            Some(id) => self.status_cursor_by_id(id).await?,
            None => None,
        };

        if let Some((created_at, id)) = max_cursor {
            query_builder.push(" AND (s.created_at < ");
            query_builder.push_bind(created_at);
            query_builder.push(" OR (s.created_at = ");
            query_builder.push_bind(created_at);
            query_builder.push(" AND s.id < ");
            query_builder.push_bind(id);
            query_builder.push("))");
        }
        if let Some((created_at, id)) = min_cursor {
            query_builder.push(" AND (s.created_at > ");
            query_builder.push_bind(created_at);
            query_builder.push(" OR (s.created_at = ");
            query_builder.push_bind(created_at);
            query_builder.push(" AND s.id > ");
            query_builder.push_bind(id);
            query_builder.push("))");
        }

        query_builder.push(" ORDER BY s.created_at DESC, s.id DESC LIMIT ");
        query_builder.push_bind(query.limit as i64);

        let statuses = query_builder
            .build_query_as::<Status>()
            .fetch_all(&self.pool)
            .await?;
        Ok(statuses)
    }

    /// Get statuses safe to expose in ActivityPub outbox.
    ///
    /// Outbox must never leak private/direct statuses.
    pub async fn get_local_outbox_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let statuses = if let Some(max_id) = max_id {
            sqlx::query_as::<_, Status>(
                r#"
                SELECT * FROM statuses
                WHERE is_local = 1 AND visibility IN ('public', 'unlisted') AND id < ?
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(max_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, Status>(
                r#"
                SELECT * FROM statuses
                WHERE is_local = 1 AND visibility IN ('public', 'unlisted')
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(statuses)
    }
}
