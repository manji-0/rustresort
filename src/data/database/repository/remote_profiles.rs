use super::super::*;

impl Database {
    /// Load all persisted remote profiles ordered by most recent refresh.
    pub async fn list_remote_profiles(&self) -> Result<Vec<RemoteProfile>, AppError> {
        let profiles = sqlx::query_as::<_, RemoteProfile>(
            r#"
            SELECT
                address,
                uri,
                display_name,
                note,
                profile_fields_json,
                avatar_url,
                header_url,
                public_key_pem,
                inbox_uri,
                outbox_uri,
                followers_count,
                following_count,
                fetched_at,
                created_at,
                updated_at
            FROM remote_profiles
            ORDER BY fetched_at DESC, updated_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(profiles)
    }

    /// Persist a refreshed remote actor profile.
    pub async fn upsert_remote_profile(&self, profile: &RemoteProfile) -> Result<(), AppError> {
        let now = chrono::Utc::now();

        sqlx::query(
            r#"
            INSERT INTO remote_profiles (
                address,
                uri,
                display_name,
                note,
                profile_fields_json,
                avatar_url,
                header_url,
                public_key_pem,
                inbox_uri,
                outbox_uri,
                followers_count,
                following_count,
                fetched_at,
                created_at,
                updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(address) DO UPDATE SET
                uri = excluded.uri,
                display_name = excluded.display_name,
                note = excluded.note,
                profile_fields_json = excluded.profile_fields_json,
                avatar_url = excluded.avatar_url,
                header_url = excluded.header_url,
                public_key_pem = excluded.public_key_pem,
                inbox_uri = excluded.inbox_uri,
                outbox_uri = excluded.outbox_uri,
                followers_count = excluded.followers_count,
                following_count = excluded.following_count,
                fetched_at = excluded.fetched_at,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&profile.address)
        .bind(&profile.uri)
        .bind(&profile.display_name)
        .bind(&profile.note)
        .bind(&profile.profile_fields_json)
        .bind(&profile.avatar_url)
        .bind(&profile.header_url)
        .bind(&profile.public_key_pem)
        .bind(&profile.inbox_uri)
        .bind(&profile.outbox_uri)
        .bind(profile.followers_count)
        .bind(profile.following_count)
        .bind(profile.fetched_at)
        .bind(profile.created_at)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
