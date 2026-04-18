use super::super::*;

impl Database {
    // =========================================================================
    // Polls (Phase 3)
    // =========================================================================

    /// Create a poll
    pub async fn create_poll(
        &self,
        status_id: &str,
        options: &[String],
        expires_in: i64,
        multiple: bool,
    ) -> Result<String, AppError> {
        self.create_poll_with_hide_totals(status_id, options, expires_in, multiple, false)
            .await
    }

    /// Create a poll with explicit hidden-totals behavior.
    pub async fn create_poll_with_hide_totals(
        &self,
        status_id: &str,
        options: &[String],
        expires_in: i64,
        multiple: bool,
        hide_totals: bool,
    ) -> Result<String, AppError> {
        let poll_id = EntityId::new_string();
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in);

        sqlx::query(
            r#"
            INSERT INTO polls (id, status_id, expires_at, expired, multiple, hide_totals, votes_count, voters_count, created_at)
            VALUES (?, ?, ?, 0, ?, ?, 0, 0, datetime('now'))
            "#,
        )
        .bind(&poll_id)
        .bind(status_id)
        .bind(expires_at.to_rfc3339())
        .bind(multiple as i64)
        .bind(hide_totals as i64)
        .execute(&self.pool)
        .await?;

        // Create poll options
        for (index, option) in options.iter().enumerate() {
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
            .execute(&self.pool)
            .await?;
        }

        Ok(poll_id)
    }

    /// Get poll by ID
    pub async fn get_poll(
        &self,
        poll_id: &str,
    ) -> Result<Option<(String, String, bool, bool, bool, i64, i64)>, AppError> {
        let result = sqlx::query_as::<_, (String, String, i64, i64, i64, i64, i64)>(
            "SELECT id, expires_at, expired, multiple, hide_totals, votes_count, voters_count FROM polls WHERE id = ?",
        )
        .bind(poll_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(
            |(id, expires_at, expired, multiple, hide_totals, votes_count, voters_count)| {
                (
                    id,
                    expires_at.clone(),
                    poll_is_expired(&expires_at, expired),
                    multiple != 0,
                    hide_totals != 0,
                    votes_count,
                    voters_count,
                )
            },
        ))
    }

    /// Get poll by status ID
    pub async fn get_poll_by_status_id(
        &self,
        status_id: &str,
    ) -> Result<Option<(String, String, bool, bool, bool, i64, i64)>, AppError> {
        let result = sqlx::query_as::<_, (String, String, i64, i64, i64, i64, i64)>(
            "SELECT id, expires_at, expired, multiple, hide_totals, votes_count, voters_count FROM polls WHERE status_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(status_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(
            |(id, expires_at, expired, multiple, hide_totals, votes_count, voters_count)| {
                (
                    id,
                    expires_at.clone(),
                    poll_is_expired(&expires_at, expired),
                    multiple != 0,
                    hide_totals != 0,
                    votes_count,
                    voters_count,
                )
            },
        ))
    }

    /// Get status ID for a poll.
    pub async fn get_status_id_by_poll_id(
        &self,
        poll_id: &str,
    ) -> Result<Option<String>, AppError> {
        sqlx::query_scalar("SELECT status_id FROM polls WHERE id = ?")
            .bind(poll_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::from)
    }

    /// Get poll options
    pub async fn get_poll_options(
        &self,
        poll_id: &str,
    ) -> Result<Vec<(String, String, i64)>, AppError> {
        let options = sqlx::query_as::<_, (String, String, i64)>(
            "SELECT id, title, votes_count FROM poll_options WHERE poll_id = ? ORDER BY option_index",
        )
        .bind(poll_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(options)
    }

    /// Remove any poll rows associated with a status.
    pub async fn delete_poll_by_status_id(&self, status_id: &str) -> Result<(), AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            sqlx::query(
                "DELETE FROM poll_options WHERE poll_id IN (SELECT id FROM polls WHERE status_id = ?)",
            )
            .bind(status_id)
            .execute(&mut *conn)
            .await?;
            sqlx::query("DELETE FROM polls WHERE status_id = ?")
                .bind(status_id)
                .execute(&mut *conn)
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
                super::rollback_with_log(&mut conn, "delete_poll_by_status_id").await;
                Err(error)
            }
        }
    }

    /// Replace a status poll and all options atomically.
    pub async fn replace_poll_for_status(
        &self,
        status_id: &str,
        expires_at: &str,
        expired: bool,
        multiple: bool,
        hide_totals: bool,
        votes_count: i64,
        voters_count: i64,
        options: &[(String, i64)],
    ) -> Result<String, AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<String, AppError> = async {
            let existing_poll_id = sqlx::query_scalar::<_, String>(
                "SELECT id FROM polls WHERE status_id = ? ORDER BY created_at DESC LIMIT 1",
            )
            .bind(status_id)
            .fetch_optional(&mut *conn)
            .await?;
            let had_existing_poll = existing_poll_id.is_some();
            let poll_id = existing_poll_id.unwrap_or_else(EntityId::new_string);

            sqlx::query(
                "DELETE FROM poll_votes WHERE poll_id IN (SELECT id FROM polls WHERE status_id = ? AND id != ?)",
            )
            .bind(status_id)
            .bind(&poll_id)
            .execute(&mut *conn)
            .await?;
            sqlx::query(
                "DELETE FROM poll_options WHERE poll_id IN (SELECT id FROM polls WHERE status_id = ? AND id != ?)",
            )
            .bind(status_id)
            .bind(&poll_id)
            .execute(&mut *conn)
            .await?;
            sqlx::query("DELETE FROM polls WHERE status_id = ? AND id != ?")
                .bind(status_id)
                .bind(&poll_id)
                .execute(&mut *conn)
                .await?;

            let existing_options = sqlx::query_as::<_, (String, String, i64)>(
                "SELECT id, title, option_index FROM poll_options WHERE poll_id = ? ORDER BY option_index",
            )
            .bind(&poll_id)
            .fetch_all(&mut *conn)
            .await?;
            let mut existing_option_ids_by_title = std::collections::HashMap::new();
            for (option_id, title, _option_index) in &existing_options {
                existing_option_ids_by_title.insert(title.clone(), option_id.clone());
            }

            if had_existing_poll {
                sqlx::query(
                    r#"
                    UPDATE polls
                    SET expires_at = ?, expired = ?, multiple = ?, hide_totals = ?, votes_count = ?, voters_count = ?
                    WHERE id = ?
                    "#,
                )
                .bind(expires_at)
                .bind(expired as i64)
                .bind(multiple as i64)
                .bind(hide_totals as i64)
                .bind(votes_count)
                .bind(voters_count)
                .bind(&poll_id)
                .execute(&mut *conn)
                .await?;
            } else {
                sqlx::query(
                    r#"
                    INSERT INTO polls (id, status_id, expires_at, expired, multiple, hide_totals, votes_count, voters_count, created_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
                    "#,
                )
                .bind(&poll_id)
                .bind(status_id)
                .bind(expires_at)
                .bind(expired as i64)
                .bind(multiple as i64)
                .bind(hide_totals as i64)
                .bind(votes_count)
                .bind(voters_count)
                .execute(&mut *conn)
                .await?;
            }

            let mut retained_option_ids = Vec::with_capacity(options.len());

            for (index, (title, option_votes_count)) in options.iter().enumerate() {
                let option_id = existing_option_ids_by_title
                    .get(title)
                    .cloned()
                    .unwrap_or_else(EntityId::new_string);
                retained_option_ids.push(option_id.clone());

                if existing_option_ids_by_title.contains_key(title) {
                    sqlx::query(
                        r#"
                        UPDATE poll_options
                        SET title = ?, votes_count = ?, option_index = ?
                        WHERE id = ? AND poll_id = ?
                        "#,
                    )
                    .bind(title)
                    .bind(*option_votes_count)
                    .bind(index as i64)
                    .bind(&option_id)
                    .bind(&poll_id)
                    .execute(&mut *conn)
                    .await?;
                } else {
                    sqlx::query(
                        r#"
                        INSERT INTO poll_options (id, poll_id, title, votes_count, option_index, created_at)
                        VALUES (?, ?, ?, ?, ?, datetime('now'))
                        "#,
                    )
                    .bind(&option_id)
                    .bind(&poll_id)
                    .bind(title)
                    .bind(*option_votes_count)
                    .bind(index as i64)
                    .execute(&mut *conn)
                    .await?;
                }
            }

            for (option_id, title, _option_index) in existing_options {
                if retained_option_ids.contains(&option_id) {
                    continue;
                }
                sqlx::query("DELETE FROM poll_votes WHERE poll_id = ? AND option_id = ?")
                    .bind(&poll_id)
                    .bind(&option_id)
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("DELETE FROM poll_options WHERE id = ? AND poll_id = ?")
                    .bind(&option_id)
                    .bind(&poll_id)
                    .execute(&mut *conn)
                    .await?;
                let _ = title;
            }

            Ok(poll_id)
        }
        .await;

        match result {
            Ok(poll_id) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(poll_id)
            }
            Err(error) => {
                super::rollback_with_log(&mut conn, "replace_poll_for_status").await;
                Err(error)
            }
        }
    }

    /// Vote in poll
    pub async fn vote_in_poll(
        &self,
        poll_id: &str,
        voter_address: &str,
        option_ids: &[String],
    ) -> Result<(), AppError> {
        if option_ids.is_empty() {
            return Err(AppError::Validation(
                "At least one choice is required".to_string(),
            ));
        }

        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            let poll = sqlx::query_as::<_, (String, String, i64, i64, i64, i64)>(
                "SELECT id, expires_at, expired, multiple, votes_count, voters_count FROM polls WHERE id = ?",
            )
            .bind(poll_id)
            .fetch_optional(&mut *conn)
            .await?
            .map(
                |(id, expires_at, expired, multiple, votes_count, voters_count)| {
                    (
                        id,
                        expires_at.clone(),
                        poll_is_expired(&expires_at, expired),
                        multiple != 0,
                        votes_count,
                        voters_count,
                    )
                },
            )
            .ok_or(AppError::NotFound)?;

            if poll.2 {
                return Err(AppError::Validation("Poll has expired".to_string()));
            }
            if !poll.3 && option_ids.len() > 1 {
                return Err(AppError::Validation(
                    "Poll does not allow multiple choices".to_string(),
                ));
            }

            // A voter can submit at most one ballot per poll.
            // For multiple-choice polls, the ballot may include multiple options.
            let existing_vote: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM poll_votes WHERE poll_id = ? AND voter_address = ?",
            )
            .bind(poll_id)
            .bind(voter_address)
            .fetch_one(&mut *conn)
            .await?;

            if existing_vote > 0 {
                return Err(AppError::Validation(
                    "Already voted in this poll".to_string(),
                ));
            }

            for option_id in option_ids {
                let option_exists: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM poll_options WHERE id = ? AND poll_id = ?",
                )
                .bind(option_id)
                .bind(poll_id)
                .fetch_one(&mut *conn)
                .await?;
                if option_exists == 0 {
                    return Err(AppError::Validation("Invalid poll option".to_string()));
                }

                let vote_id = EntityId::new_string();
                let inserted = sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO poll_votes (id, poll_id, option_id, voter_address, created_at)
                    VALUES (?, ?, ?, ?, datetime('now'))
                    "#,
                )
                .bind(&vote_id)
                .bind(poll_id)
                .bind(option_id)
                .bind(voter_address)
                .execute(&mut *conn)
                .await?;
                if inserted.rows_affected() == 0 {
                    return Err(AppError::Validation(
                        "Already voted in this poll".to_string(),
                    ));
                }

                let updated = sqlx::query(
                    "UPDATE poll_options SET votes_count = votes_count + 1 WHERE id = ? AND poll_id = ?",
                )
                .bind(option_id)
                .bind(poll_id)
                .execute(&mut *conn)
                .await?;
                if updated.rows_affected() == 0 {
                    return Err(AppError::Validation("Invalid poll option".to_string()));
                }
            }

            // Update poll totals inside the same transaction.
            let total_votes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM poll_votes WHERE poll_id = ?")
                .bind(poll_id)
                .fetch_one(&mut *conn)
                .await?;
            let unique_voters: i64 = sqlx::query_scalar(
                "SELECT COUNT(DISTINCT voter_address) FROM poll_votes WHERE poll_id = ?",
            )
            .bind(poll_id)
            .fetch_one(&mut *conn)
            .await?;
            sqlx::query("UPDATE polls SET votes_count = ?, voters_count = ? WHERE id = ?")
                .bind(total_votes)
                .bind(unique_voters)
                .bind(poll_id)
                .execute(&mut *conn)
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
                super::rollback_with_log(&mut conn, "vote_in_poll").await;
                Err(error)
            }
        }
    }

    /// Record a remote ActivityPub poll vote, allowing multiple activities for
    /// multiple-choice polls while ignoring duplicate option votes from the same voter.
    pub async fn record_remote_poll_vote(
        &self,
        poll_id: &str,
        voter_address: &str,
        option_ids: &[String],
    ) -> Result<bool, AppError> {
        if option_ids.is_empty() {
            return Err(AppError::Validation(
                "At least one choice is required".to_string(),
            ));
        }

        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<bool, AppError> = async {
            let poll = sqlx::query_as::<_, (String, String, i64, i64, i64, i64)>(
                "SELECT id, expires_at, expired, multiple, votes_count, voters_count FROM polls WHERE id = ?",
            )
            .bind(poll_id)
            .fetch_optional(&mut *conn)
            .await?
            .map(
                |(id, expires_at, expired, multiple, votes_count, voters_count)| {
                    (
                        id,
                        expires_at.clone(),
                        poll_is_expired(&expires_at, expired),
                        multiple != 0,
                        votes_count,
                        voters_count,
                    )
                },
            )
            .ok_or(AppError::NotFound)?;

            if poll.2 {
                return Err(AppError::Validation("Poll has expired".to_string()));
            }
            if !poll.3 && option_ids.len() > 1 {
                return Err(AppError::Validation(
                    "Poll does not allow multiple choices".to_string(),
                ));
            }

            let existing_vote_option_ids = sqlx::query_scalar::<_, String>(
                "SELECT option_id FROM poll_votes WHERE poll_id = ? AND voter_address = ?",
            )
            .bind(poll_id)
            .bind(voter_address)
            .fetch_all(&mut *conn)
            .await?;
            let existing_vote_set = existing_vote_option_ids
                .iter()
                .cloned()
                .collect::<std::collections::HashSet<_>>();

            if !poll.3 && !existing_vote_set.is_empty() {
                return Ok(false);
            }

            let mut inserted_any = false;
            for option_id in option_ids {
                let option_exists: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM poll_options WHERE id = ? AND poll_id = ?",
                )
                .bind(option_id)
                .bind(poll_id)
                .fetch_one(&mut *conn)
                .await?;
                if option_exists == 0 {
                    return Err(AppError::Validation("Invalid poll option".to_string()));
                }
                if existing_vote_set.contains(option_id) {
                    continue;
                }

                let vote_id = EntityId::new_string();
                let inserted = sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO poll_votes (id, poll_id, option_id, voter_address, created_at)
                    VALUES (?, ?, ?, ?, datetime('now'))
                    "#,
                )
                .bind(&vote_id)
                .bind(poll_id)
                .bind(option_id)
                .bind(voter_address)
                .execute(&mut *conn)
                .await?;
                if inserted.rows_affected() == 0 {
                    continue;
                }

                inserted_any = true;
                sqlx::query(
                    "UPDATE poll_options SET votes_count = votes_count + 1 WHERE id = ? AND poll_id = ?",
                )
                .bind(option_id)
                .bind(poll_id)
                .execute(&mut *conn)
                .await?;
            }

            if inserted_any {
                let total_votes: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM poll_votes WHERE poll_id = ?")
                        .bind(poll_id)
                        .fetch_one(&mut *conn)
                        .await?;
                let unique_voters: i64 = sqlx::query_scalar(
                    "SELECT COUNT(DISTINCT voter_address) FROM poll_votes WHERE poll_id = ?",
                )
                .bind(poll_id)
                .fetch_one(&mut *conn)
                .await?;
                sqlx::query("UPDATE polls SET votes_count = ?, voters_count = ? WHERE id = ?")
                    .bind(total_votes)
                    .bind(unique_voters)
                    .bind(poll_id)
                    .execute(&mut *conn)
                    .await?;
            }

            Ok(inserted_any)
        }
        .await;

        match result {
            Ok(inserted_any) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(inserted_any)
            }
            Err(error) => {
                super::rollback_with_log(&mut conn, "record_remote_poll_vote").await;
                Err(error)
            }
        }
    }

    /// Get user's votes in a poll
    pub async fn get_user_poll_votes(
        &self,
        poll_id: &str,
        voter_address: &str,
    ) -> Result<Vec<String>, AppError> {
        let option_ids = sqlx::query_scalar::<_, String>(
            "SELECT option_id FROM poll_votes WHERE poll_id = ? AND voter_address = ?",
        )
        .bind(poll_id)
        .bind(voter_address)
        .fetch_all(&self.pool)
        .await?;

        Ok(option_ids)
    }
}
