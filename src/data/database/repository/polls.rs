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
        let poll_id = EntityId::new_string();
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in);

        sqlx::query(
            r#"
            INSERT INTO polls (id, status_id, expires_at, expired, multiple, votes_count, voters_count, created_at)
            VALUES (?, ?, ?, 0, ?, 0, 0, datetime('now'))
            "#,
        )
        .bind(&poll_id)
        .bind(status_id)
        .bind(expires_at.to_rfc3339())
        .bind(multiple as i64)
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
    ) -> Result<Option<(String, String, bool, bool, i64, i64)>, AppError> {
        let result = sqlx::query_as::<_, (String, String, i64, i64, i64, i64)>(
            "SELECT id, expires_at, expired, multiple, votes_count, voters_count FROM polls WHERE id = ?",
        )
        .bind(poll_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(
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
        ))
    }

    /// Get poll by status ID
    pub async fn get_poll_by_status_id(
        &self,
        status_id: &str,
    ) -> Result<Option<(String, String, bool, bool, i64, i64)>, AppError> {
        let result = sqlx::query_as::<_, (String, String, i64, i64, i64, i64)>(
            "SELECT id, expires_at, expired, multiple, votes_count, voters_count FROM polls WHERE status_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(status_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(
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
        ))
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
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
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
