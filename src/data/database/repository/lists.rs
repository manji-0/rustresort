use super::super::*;

impl Database {
    // =========================================================================
    // Lists (Phase 2)
    // =========================================================================

    /// Create a new list
    pub async fn create_list(
        &self,
        title: &str,
        replies_policy: &str,
        exclusive: bool,
    ) -> Result<String, AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            r#"
            INSERT INTO lists (id, title, replies_policy, exclusive, created_at, updated_at)
            VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))
            "#,
        )
        .bind(&id)
        .bind(title)
        .bind(replies_policy)
        .bind(exclusive)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Get list by ID
    pub async fn get_list(
        &self,
        id: &str,
    ) -> Result<Option<(String, String, String, bool)>, AppError> {
        let result = sqlx::query_as::<_, (String, String, String, bool)>(
            "SELECT id, title, replies_policy, exclusive FROM lists WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }

    /// Get all lists
    pub async fn get_all_lists(&self) -> Result<Vec<(String, String, String, bool)>, AppError> {
        let lists = sqlx::query_as::<_, (String, String, String, bool)>(
            "SELECT id, title, replies_policy, exclusive FROM lists ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(lists)
    }

    /// Update list
    pub async fn update_list(
        &self,
        id: &str,
        title: &str,
        replies_policy: &str,
        exclusive: bool,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            "UPDATE lists SET title = ?, replies_policy = ?, exclusive = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(title)
        .bind(replies_policy)
        .bind(exclusive)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete list
    pub async fn delete_list(&self, id: &str) -> Result<bool, AppError> {
        let result = sqlx::query("DELETE FROM lists WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Add account to list
    pub async fn add_account_to_list(
        &self,
        list_id: &str,
        account_address: &str,
    ) -> Result<(), AppError> {
        let id = EntityId::new_string();
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO list_accounts (id, list_id, account_address, created_at)
            VALUES (?, ?, ?, datetime('now'))
            "#,
        )
        .bind(&id)
        .bind(list_id)
        .bind(account_address)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Add multiple accounts to list atomically
    pub async fn add_accounts_to_list(
        &self,
        list_id: &str,
        account_addresses: &[String],
    ) -> Result<(), AppError> {
        if account_addresses.is_empty() {
            return Ok(());
        }

        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            for account_address in account_addresses {
                let id = EntityId::new_string();
                sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO list_accounts (id, list_id, account_address, created_at)
                    VALUES (?, ?, ?, datetime('now'))
                    "#,
                )
                .bind(&id)
                .bind(list_id)
                .bind(account_address)
                .execute(&mut *conn)
                .await?;
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
                super::rollback_with_log(&mut conn, "add_accounts_to_list").await;
                Err(error)
            }
        }
    }

    /// Remove account from list
    pub async fn remove_account_from_list(
        &self,
        list_id: &str,
        account_address: &str,
    ) -> Result<bool, AppError> {
        let result =
            sqlx::query("DELETE FROM list_accounts WHERE list_id = ? AND account_address = ?")
                .bind(list_id)
                .bind(account_address)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Remove multiple accounts from list atomically
    pub async fn remove_accounts_from_list(
        &self,
        list_id: &str,
        account_addresses: &[String],
    ) -> Result<(), AppError> {
        if account_addresses.is_empty() {
            return Ok(());
        }

        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), AppError> = async {
            for account_address in account_addresses {
                sqlx::query("DELETE FROM list_accounts WHERE list_id = ? AND account_address = ?")
                    .bind(list_id)
                    .bind(account_address)
                    .execute(&mut *conn)
                    .await?;
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
                super::rollback_with_log(&mut conn, "remove_accounts_from_list").await;
                Err(error)
            }
        }
    }

    /// Get accounts in list
    pub async fn get_list_accounts(&self, list_id: &str) -> Result<Vec<String>, AppError> {
        let addresses = sqlx::query_scalar::<_, String>(
            "SELECT account_address FROM list_accounts WHERE list_id = ? ORDER BY created_at DESC",
        )
        .bind(list_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    /// Check if account is in list
    pub async fn is_account_in_list(
        &self,
        list_id: &str,
        account_address: &str,
    ) -> Result<bool, AppError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM list_accounts WHERE list_id = ? AND account_address = ?",
        )
        .bind(list_id)
        .bind(account_address)
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }

    /// Get list IDs that contain the given account address.
    pub async fn get_list_ids_for_account(
        &self,
        account_address: &str,
        default_port: Option<u16>,
    ) -> Result<Vec<String>, AppError> {
        let normalized_candidates: Vec<String> =
            equivalent_account_address_candidates(account_address, default_port)
                .into_iter()
                .map(|candidate| candidate.to_ascii_lowercase())
                .collect();
        if normalized_candidates.is_empty() {
            return Ok(Vec::new());
        }

        let mut query_builder = QueryBuilder::<Sqlite>::new(
            "SELECT DISTINCT list_id FROM list_accounts WHERE LOWER(account_address) IN (",
        );
        {
            let mut separated = query_builder.separated(", ");
            for candidate in normalized_candidates {
                separated.push_bind(candidate);
            }
        }
        query_builder.push(")");

        let list_ids = query_builder
            .build_query_scalar::<String>()
            .fetch_all(&self.pool)
            .await?;

        Ok(list_ids)
    }
}
