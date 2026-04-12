use super::super::*;
use crate::data::PasskeyCredential;

impl Database {
    pub async fn list_passkeys(&self) -> Result<Vec<PasskeyCredential>, AppError> {
        let passkeys = sqlx::query_as::<_, PasskeyCredential>(
            "SELECT * FROM passkeys ORDER BY created_at DESC, id DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(passkeys)
    }

    pub async fn get_passkey_by_id(&self, id: &str) -> Result<Option<PasskeyCredential>, AppError> {
        let passkey = sqlx::query_as::<_, PasskeyCredential>("SELECT * FROM passkeys WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(passkey)
    }

    pub async fn insert_passkey(&self, passkey: &PasskeyCredential) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO passkeys (
                id, credential_id, name, passkey_json, created_at, updated_at, last_used_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&passkey.id)
        .bind(&passkey.credential_id)
        .bind(&passkey.name)
        .bind(&passkey.passkey_json)
        .bind(passkey.created_at)
        .bind(passkey.updated_at)
        .bind(passkey.last_used_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn update_passkey(&self, passkey: &PasskeyCredential) -> Result<(), AppError> {
        sqlx::query(
            r#"
            UPDATE passkeys
            SET credential_id = ?, name = ?, passkey_json = ?, updated_at = ?, last_used_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&passkey.credential_id)
        .bind(&passkey.name)
        .bind(&passkey.passkey_json)
        .bind(passkey.updated_at)
        .bind(passkey.last_used_at)
        .bind(&passkey.id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn delete_passkey(&self, id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM passkeys WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}
