use super::super::*;

impl Database {
    // =========================================================================
    // OAuth Apps and Tokens
    // =========================================================================

    /// Insert OAuth app
    pub async fn insert_oauth_app(&self, app: &OAuthApp) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO oauth_apps (
                id, name, website, redirect_uri, client_id, client_secret, vapid_key, scopes, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&app.id)
        .bind(&app.name)
        .bind(&app.website)
        .bind(&app.redirect_uri)
        .bind(&app.client_id)
        .bind(&app.client_secret)
        .bind(&app.vapid_key)
        .bind(&app.scopes)
        .bind(app.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Count user-authorized OAuth tokens created within a time range.
    pub async fn count_user_oauth_tokens_created_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM oauth_tokens
            WHERE created_at >= ?
              AND created_at < ?
              AND grant_type != 'client_credentials'
            "#,
        )
        .bind(start)
        .bind(end)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    /// Get OAuth app by client ID
    pub async fn get_oauth_app_by_client_id(
        &self,
        client_id: &str,
    ) -> Result<Option<OAuthApp>, AppError> {
        let app = sqlx::query_as::<_, OAuthApp>("SELECT * FROM oauth_apps WHERE client_id = ?")
            .bind(client_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(app)
    }

    /// Get OAuth app by app ID
    pub async fn get_oauth_app_by_id(&self, app_id: &str) -> Result<Option<OAuthApp>, AppError> {
        let app = sqlx::query_as::<_, OAuthApp>("SELECT * FROM oauth_apps WHERE id = ?")
            .bind(app_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(app)
    }

    /// Get latest OAuth app by creation time.
    pub async fn get_latest_oauth_app(&self) -> Result<Option<OAuthApp>, AppError> {
        let app = sqlx::query_as::<_, OAuthApp>(
            "SELECT * FROM oauth_apps ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(app)
    }

    /// Insert OAuth authorization code
    pub async fn insert_oauth_authorization_code(
        &self,
        code: &OAuthAuthorizationCode,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO oauth_authorization_codes (
                id, app_id, code, redirect_uri, scopes, code_challenge, code_challenge_method, created_at, expires_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&code.id)
        .bind(&code.app_id)
        .bind(&code.code)
        .bind(&code.redirect_uri)
        .bind(&code.scopes)
        .bind(&code.code_challenge)
        .bind(&code.code_challenge_method)
        .bind(code.created_at)
        .bind(code.expires_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get OAuth authorization code by code value
    pub async fn get_oauth_authorization_code(
        &self,
        code: &str,
    ) -> Result<Option<OAuthAuthorizationCode>, AppError> {
        let auth_code = sqlx::query_as::<_, OAuthAuthorizationCode>(
            "SELECT * FROM oauth_authorization_codes WHERE code = ?",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await?;

        Ok(auth_code)
    }

    /// Consume (single-use) OAuth authorization code with strict binding checks
    pub async fn consume_oauth_authorization_code(
        &self,
        code: &str,
        app_id: &str,
        redirect_uri: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuthAuthorizationCode>, AppError> {
        let Some(auth_code) = self.get_oauth_authorization_code(code).await? else {
            return Ok(None);
        };

        if auth_code.expires_at <= now {
            // Purge expired code on redemption attempt to avoid unbounded table growth.
            sqlx::query("DELETE FROM oauth_authorization_codes WHERE id = ?")
                .bind(&auth_code.id)
                .execute(&self.pool)
                .await?;
            return Ok(None);
        }

        if auth_code.app_id != app_id || auth_code.redirect_uri != redirect_uri {
            return Ok(None);
        }

        let result = sqlx::query("DELETE FROM oauth_authorization_codes WHERE id = ?")
            .bind(&auth_code.id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        Ok(Some(auth_code))
    }

    /// Insert OAuth token
    pub async fn insert_oauth_token(&self, token: &OAuthToken) -> Result<(), AppError> {
        let access_token_hash = hash_oauth_access_token(&token.access_token);
        let refresh_token_hash = token.refresh_token.as_deref().map(hash_oauth_token_secret);
        sqlx::query(
            r#"
            INSERT INTO oauth_tokens (
                id, app_id, access_token, refresh_token, grant_type, scopes, created_at, expires_at, refresh_expires_at, revoked
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&token.id)
        .bind(&token.app_id)
        .bind(&access_token_hash)
        .bind(&refresh_token_hash)
        .bind(&token.grant_type)
        .bind(&token.scopes)
        .bind(token.created_at)
        .bind(token.expires_at)
        .bind(token.refresh_expires_at)
        .bind(token.revoked)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get OAuth token by access token
    pub async fn get_oauth_token(
        &self,
        access_token: &str,
    ) -> Result<Option<OAuthToken>, AppError> {
        let access_token_hash = hash_oauth_access_token(access_token);
        let now = Utc::now();
        let token = sqlx::query_as::<_, OAuthToken>(
            r#"
            SELECT *
            FROM oauth_tokens
            WHERE access_token = ?
              AND revoked = 0
              AND (expires_at IS NULL OR expires_at > ?)
            "#,
        )
        .bind(&access_token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        Ok(token)
    }

    /// Revoke OAuth token
    pub async fn revoke_oauth_token(&self, access_token: &str) -> Result<bool, AppError> {
        let access_token_hash = hash_oauth_access_token(access_token);
        let refresh_token_hash = hash_oauth_token_secret(access_token);
        let result = sqlx::query(
            "UPDATE oauth_tokens SET revoked = 1 WHERE access_token = ? OR refresh_token = ?",
        )
        .bind(&access_token_hash)
        .bind(&refresh_token_hash)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Revoke OAuth token for a specific OAuth app only.
    pub async fn revoke_oauth_token_for_app(
        &self,
        app_id: &str,
        token: &str,
    ) -> Result<bool, AppError> {
        let access_token_hash = hash_oauth_access_token(token);
        let refresh_token_hash = hash_oauth_token_secret(token);
        let result = sqlx::query(
            "UPDATE oauth_tokens SET revoked = 1 WHERE app_id = ? AND (access_token = ? OR refresh_token = ?)",
        )
        .bind(app_id)
        .bind(&access_token_hash)
        .bind(&refresh_token_hash)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Look up the owning OAuth app for an access or refresh token, including
    /// revoked tokens so revocation remains idempotent.
    pub async fn lookup_oauth_token_owner(&self, token: &str) -> Result<Option<String>, AppError> {
        let access_token_hash = hash_oauth_access_token(token);
        let refresh_token_hash = hash_oauth_token_secret(token);
        sqlx::query_scalar::<_, String>(
            r#"
            SELECT app_id
            FROM oauth_tokens
            WHERE access_token = ? OR refresh_token = ?
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(&access_token_hash)
        .bind(&refresh_token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Get OAuth token by refresh token.
    pub async fn get_oauth_token_by_refresh_token(
        &self,
        refresh_token: &str,
    ) -> Result<Option<OAuthToken>, AppError> {
        let refresh_token_hash = hash_oauth_token_secret(refresh_token);
        let now = Utc::now();
        let token = sqlx::query_as::<_, OAuthToken>(
            r#"
            SELECT *
            FROM oauth_tokens
            WHERE refresh_token = ?
              AND revoked = 0
              AND (refresh_expires_at IS NULL OR refresh_expires_at > ?)
            "#,
        )
        .bind(&refresh_token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        Ok(token)
    }
}
