use super::super::*;

impl Database {
    /// Replace the current push subscription for the single local user.
    pub async fn upsert_push_subscription(
        &self,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        alerts: &PushAlerts,
        policy: &str,
    ) -> Result<PushSubscription, AppError> {
        let now = chrono::Utc::now();
        let subscription = PushSubscription {
            id: EntityId::new_string(),
            endpoint: endpoint.to_string(),
            p256dh: p256dh.to_string(),
            auth: auth.to_string(),
            alerts_json: serde_json::to_string(alerts).map_err(|error| {
                AppError::Validation(format!("Failed to serialize push alerts: {}", error))
            })?,
            policy: policy.to_string(),
            created_at: now,
            updated_at: now,
        };

        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM push_subscriptions")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            r#"
            INSERT INTO push_subscriptions (
                id, endpoint, p256dh, auth, alerts_json, policy, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&subscription.id)
        .bind(&subscription.endpoint)
        .bind(&subscription.p256dh)
        .bind(&subscription.auth)
        .bind(&subscription.alerts_json)
        .bind(&subscription.policy)
        .bind(subscription.created_at)
        .bind(subscription.updated_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(subscription)
    }

    /// Get the single stored push subscription, if any.
    pub async fn get_push_subscription(&self) -> Result<Option<PushSubscription>, AppError> {
        let subscription = sqlx::query_as::<_, PushSubscription>(
            r#"
            SELECT id, endpoint, p256dh, auth, alerts_json, policy, created_at, updated_at
            FROM push_subscriptions
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(subscription)
    }

    /// Delete the stored push subscription.
    pub async fn delete_push_subscription(&self) -> Result<(), AppError> {
        sqlx::query("DELETE FROM push_subscriptions")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
