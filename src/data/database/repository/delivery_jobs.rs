use super::super::*;

impl Database {
    /// Enqueue a new outbound federation delivery job.
    pub async fn enqueue_delivery_job(
        &self,
        inbox_url: &str,
        activity_json: &str,
        actor_key_id: &str,
    ) -> Result<(), AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            r#"
            INSERT INTO delivery_jobs (
                id, inbox_url, activity_json, actor_key_id, attempts, next_attempt_at,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, 0, datetime('now'), datetime('now'), datetime('now'))
            "#,
        )
        .bind(id)
        .bind(inbox_url)
        .bind(activity_json)
        .bind(actor_key_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Claim a batch of due delivery jobs for processing.
    pub async fn claim_pending_delivery_jobs(
        &self,
        limit: usize,
    ) -> Result<Vec<DeliveryJob>, AppError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<Vec<DeliveryJob>, AppError> = async {
            let ids = sqlx::query_scalar::<_, String>(
                r#"
                SELECT id
                FROM delivery_jobs
                WHERE delivered_at IS NULL
                  AND next_attempt_at <= datetime('now')
                  AND (
                    claimed_at IS NULL
                    OR claimed_at <= datetime('now', '-5 minutes')
                  )
                ORDER BY next_attempt_at ASC, created_at ASC
                LIMIT ?
                "#,
            )
            .bind(limit as i64)
            .fetch_all(&mut *conn)
            .await?;

            if ids.is_empty() {
                return Ok(Vec::new());
            }

            let mut update_builder = QueryBuilder::<Sqlite>::new(
                "UPDATE delivery_jobs SET claimed_at = datetime('now'), updated_at = datetime('now') WHERE id IN (",
            );
            {
                let mut separated = update_builder.separated(", ");
                for id in &ids {
                    separated.push_bind(id);
                }
            }
            update_builder.push(")");
            update_builder.build().execute(&mut *conn).await?;

            let mut select_builder =
                QueryBuilder::<Sqlite>::new("SELECT * FROM delivery_jobs WHERE id IN (");
            {
                let mut separated = select_builder.separated(", ");
                for id in &ids {
                    separated.push_bind(id);
                }
            }
            select_builder.push(") ORDER BY next_attempt_at ASC, created_at ASC");

            let jobs = select_builder
                .build_query_as::<DeliveryJob>()
                .fetch_all(&mut *conn)
                .await?;

            Ok(jobs)
        }
        .await;

        match result {
            Ok(jobs) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(jobs)
            }
            Err(error) => {
                super::rollback_with_log(&mut conn, "claim_pending_delivery_jobs").await;
                Err(error)
            }
        }
    }

    /// Mark a delivery job as successfully delivered.
    pub async fn mark_delivery_job_delivered(&self, job_id: &str) -> Result<(), AppError> {
        sqlx::query(
            r#"
            UPDATE delivery_jobs
            SET delivered_at = datetime('now'),
                claimed_at = NULL,
                updated_at = datetime('now')
            WHERE id = ?
            "#,
        )
        .bind(job_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Record a failed delivery attempt and schedule the next retry with backoff.
    pub async fn mark_delivery_job_failed(
        &self,
        job_id: &str,
        error: &str,
    ) -> Result<(), AppError> {
        let attempts =
            sqlx::query_scalar::<_, i64>("SELECT attempts FROM delivery_jobs WHERE id = ?")
                .bind(job_id)
                .fetch_optional(&self.pool)
                .await?
                .ok_or(AppError::NotFound)?;

        let next_attempt = std::cmp::min(300_i64, 2_i64.pow((attempts as u32).min(7)));
        let modifier = format!("+{} seconds", next_attempt);

        sqlx::query(
            r#"
            UPDATE delivery_jobs
            SET attempts = attempts + 1,
                last_error = ?,
                next_attempt_at = datetime('now', ?),
                claimed_at = NULL,
                updated_at = datetime('now')
            WHERE id = ?
            "#,
        )
        .bind(error)
        .bind(modifier)
        .bind(job_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete undelivered jobs that exceeded the retry threshold.
    pub async fn reap_dead_delivery_jobs(&self, max_attempts: u32) -> Result<u64, AppError> {
        let result =
            sqlx::query("DELETE FROM delivery_jobs WHERE delivered_at IS NULL AND attempts >= ?")
                .bind(max_attempts as i64)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected())
    }
}
