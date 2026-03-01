use super::super::*;

impl Database {
    // =========================================================================
    // Account (single user)
    // =========================================================================

    /// Get the single admin account
    ///
    /// # Returns
    /// The account or None if not initialized
    pub async fn get_account(&self) -> Result<Option<Account>, AppError> {
        let account = sqlx::query_as::<_, Account>("SELECT * FROM account LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;

        Ok(account)
    }

    /// Create or update the admin account
    ///
    /// # Arguments
    /// * `account` - Account data to upsert
    pub async fn upsert_account(&self, account: &Account) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO account (
                id, username, display_name, note, avatar_s3_key, header_s3_key,
                private_key_pem, public_key_pem, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&account.id)
        .bind(&account.username)
        .bind(&account.display_name)
        .bind(&account.note)
        .bind(&account.avatar_s3_key)
        .bind(&account.header_s3_key)
        .bind(&account.private_key_pem)
        .bind(&account.public_key_pem)
        .bind(account.created_at)
        .bind(account.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Insert the admin account only when the table is empty.
    ///
    /// This is atomic at the SQL statement level and prevents races where
    /// multiple initializers try to create the first account concurrently.
    ///
    /// # Returns
    /// `true` if inserted, `false` if an account already existed.
    pub async fn insert_account_if_empty(&self, account: &Account) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            INSERT INTO account (
                id, username, display_name, note, avatar_s3_key, header_s3_key,
                private_key_pem, public_key_pem, created_at, updated_at
            )
            SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
            WHERE NOT EXISTS (SELECT 1 FROM account)
            "#,
        )
        .bind(&account.id)
        .bind(&account.username)
        .bind(&account.display_name)
        .bind(&account.note)
        .bind(&account.avatar_s3_key)
        .bind(&account.header_s3_key)
        .bind(&account.private_key_pem)
        .bind(&account.public_key_pem)
        .bind(account.created_at)
        .bind(account.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Update account profile fields by account ID.
    ///
    /// # Returns
    /// `true` if updated, `false` if no matching account row exists.
    pub async fn update_account_profile(
        &self,
        account_id: &str,
        display_name: Option<&str>,
        note: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE account
            SET display_name = ?, note = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(display_name)
        .bind(note)
        .bind(updated_at)
        .bind(account_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Patch account profile fields by account ID.
    ///
    /// Use `None` for omitted fields (no change), and `Some(None)` to clear a field.
    ///
    /// # Returns
    /// `true` if updated, `false` if no matching account row exists.
    pub async fn patch_account_profile(
        &self,
        account_id: &str,
        display_name: Option<Option<&str>>,
        note: Option<Option<&str>>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError> {
        let result = match (display_name, note) {
            (Some(display_name), Some(note)) => {
                sqlx::query(
                    r#"
                    UPDATE account
                    SET display_name = ?, note = ?, updated_at = ?
                    WHERE id = ?
                    "#,
                )
                .bind(display_name)
                .bind(note)
                .bind(updated_at)
                .bind(account_id)
                .execute(&self.pool)
                .await?
            }
            (Some(display_name), None) => {
                sqlx::query(
                    r#"
                    UPDATE account
                    SET display_name = ?, updated_at = ?
                    WHERE id = ?
                    "#,
                )
                .bind(display_name)
                .bind(updated_at)
                .bind(account_id)
                .execute(&self.pool)
                .await?
            }
            (None, Some(note)) => {
                sqlx::query(
                    r#"
                    UPDATE account
                    SET note = ?, updated_at = ?
                    WHERE id = ?
                    "#,
                )
                .bind(note)
                .bind(updated_at)
                .bind(account_id)
                .execute(&self.pool)
                .await?
            }
            // Treat a no-op patch as success. Callers can still decide
            // whether they want to skip calling this API beforehand.
            (None, None) => return Ok(true),
        };

        Ok(result.rows_affected() == 1)
    }

    /// Update account avatar key by account ID.
    ///
    /// # Returns
    /// `true` if updated, `false` if no matching account row exists.
    pub async fn update_account_avatar_key_if_matches(
        &self,
        account_id: &str,
        expected_current_avatar_s3_key: Option<&str>,
        avatar_s3_key: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE account
            SET avatar_s3_key = ?, updated_at = ?
            WHERE id = ? AND avatar_s3_key IS ?
            "#,
        )
        .bind(avatar_s3_key)
        .bind(updated_at)
        .bind(account_id)
        .bind(expected_current_avatar_s3_key)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Update account header key by account ID.
    ///
    /// # Returns
    /// `true` if updated, `false` if no matching account row exists.
    pub async fn update_account_header_key_if_matches(
        &self,
        account_id: &str,
        expected_current_header_s3_key: Option<&str>,
        header_s3_key: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE account
            SET header_s3_key = ?, updated_at = ?
            WHERE id = ? AND header_s3_key IS ?
            "#,
        )
        .bind(header_s3_key)
        .bind(updated_at)
        .bind(account_id)
        .bind(expected_current_header_s3_key)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Atomically patch account credentials while guarding expected media keys.
    ///
    /// This updates avatar/header keys and optional profile fields in a single
    /// SQL statement so callers can avoid partial account updates.
    pub async fn patch_account_credentials_if_matches(
        &self,
        account_id: &str,
        expected_current_avatar_s3_key: Option<&str>,
        expected_current_header_s3_key: Option<&str>,
        avatar_s3_key: Option<&str>,
        header_s3_key: Option<&str>,
        display_name: Option<Option<&str>>,
        note: Option<Option<&str>>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE account
            SET
                avatar_s3_key = ?,
                header_s3_key = ?,
                display_name = CASE WHEN ? THEN ? ELSE display_name END,
                note = CASE WHEN ? THEN ? ELSE note END,
                updated_at = ?
            WHERE id = ?
              AND avatar_s3_key IS ?
              AND header_s3_key IS ?
            "#,
        )
        .bind(avatar_s3_key)
        .bind(header_s3_key)
        .bind(display_name.is_some())
        .bind(display_name.flatten())
        .bind(note.is_some())
        .bind(note.flatten())
        .bind(updated_at)
        .bind(account_id)
        .bind(expected_current_avatar_s3_key)
        .bind(expected_current_header_s3_key)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }
}
