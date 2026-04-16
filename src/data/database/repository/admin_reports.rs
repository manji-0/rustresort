use super::super::*;

impl Database {
    /// Load mutable state for an admin report notification.
    pub async fn get_admin_report_state(
        &self,
        report_id: &str,
    ) -> Result<Option<AdminReportState>, AppError> {
        let state = sqlx::query_as::<_, AdminReportState>(
            r#"
            SELECT
                report_id,
                category,
                comment,
                forwarded,
                rule_ids_json,
                assigned_account_id,
                action_taken,
                action_taken_at,
                action_taken_by_account_id,
                updated_at
            FROM admin_report_states
            WHERE report_id = ?
            LIMIT 1
            "#,
        )
        .bind(report_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(state)
    }

    /// Insert or replace mutable state for an admin report notification.
    pub async fn save_admin_report_state(
        &self,
        state: &AdminReportState,
    ) -> Result<AdminReportState, AppError> {
        sqlx::query(
            r#"
            INSERT INTO admin_report_states (
                report_id,
                category,
                comment,
                forwarded,
                rule_ids_json,
                assigned_account_id,
                action_taken,
                action_taken_at,
                action_taken_by_account_id,
                updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(report_id) DO UPDATE SET
                category = excluded.category,
                comment = excluded.comment,
                forwarded = excluded.forwarded,
                rule_ids_json = excluded.rule_ids_json,
                assigned_account_id = excluded.assigned_account_id,
                action_taken = excluded.action_taken,
                action_taken_at = excluded.action_taken_at,
                action_taken_by_account_id = excluded.action_taken_by_account_id,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&state.report_id)
        .bind(&state.category)
        .bind(&state.comment)
        .bind(state.forwarded)
        .bind(&state.rule_ids_json)
        .bind(&state.assigned_account_id)
        .bind(state.action_taken)
        .bind(state.action_taken_at)
        .bind(&state.action_taken_by_account_id)
        .bind(state.updated_at)
        .execute(&self.pool)
        .await?;

        self.get_admin_report_state(&state.report_id)
            .await?
            .ok_or_else(|| AppError::Validation("failed to persist admin report state".to_string()))
    }
}
