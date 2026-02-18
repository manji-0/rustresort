//! Account service
//!
//! Handles account-related operations for the single admin user.

use std::sync::Arc;

use crate::data::{Account, Database, EntityId};
use crate::error::AppError;
use crate::storage::MediaStorage;

fn normalize_optional_text(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Account service
pub struct AccountService {
    db: Arc<Database>,
    storage: Arc<MediaStorage>,
}

impl AccountService {
    /// Create new account service
    pub fn new(db: Arc<Database>, storage: Arc<MediaStorage>) -> Self {
        Self { db, storage }
    }

    /// Get the admin account
    ///
    /// # Returns
    /// The single account or error if not initialized
    pub async fn get_account(&self) -> Result<Account, AppError> {
        self.db.get_account().await?.ok_or(AppError::NotFound)
    }

    /// Initialize the admin account
    ///
    /// Creates a new account with generated RSA keypair.
    /// Should only be called once during initial setup.
    ///
    /// # Arguments
    /// * `username` - Account username (no @domain)
    ///
    /// # Errors
    /// Returns error if account already exists
    pub async fn initialize_account(&self, username: &str) -> Result<Account, AppError> {
        use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
        use rsa::{RsaPrivateKey, RsaPublicKey};

        let username = username.trim();
        if username.is_empty() {
            return Err(AppError::Validation("username cannot be empty".to_string()));
        }

        let mut rng = rand::thread_rng();
        let private_key =
            RsaPrivateKey::new(&mut rng, 4096).map_err(|e| AppError::Internal(e.into()))?;
        let public_key = RsaPublicKey::from(&private_key);

        let private_key_pem = private_key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|e| AppError::Internal(e.into()))?
            .to_string();
        let public_key_pem = public_key
            .to_public_key_pem(LineEnding::LF)
            .map_err(|e| AppError::Internal(e.into()))?;

        let account = Account {
            id: EntityId::new().0,
            username: username.to_string(),
            display_name: Some(username.to_string()),
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem,
            public_key_pem,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let inserted = self.db.insert_account_if_empty(&account).await?;
        if !inserted {
            return Err(AppError::Validation(
                "account is already initialized".to_string(),
            ));
        }

        Ok(account)
    }

    /// Update account profile
    ///
    /// # Arguments
    /// * `display_name` - New display name
    /// * `note` - New bio/note (can contain HTML)
    pub async fn update_profile(
        &self,
        display_name: Option<String>,
        note: Option<String>,
    ) -> Result<Account, AppError> {
        let mut account = self.get_account().await?;

        let new_display_name = display_name
            .map(normalize_optional_text)
            .unwrap_or_else(|| account.display_name.clone());
        let new_note = note
            .map(normalize_optional_text)
            .unwrap_or_else(|| account.note.clone());
        let updated_at = chrono::Utc::now();

        let updated = self
            .db
            .update_account_profile(
                &account.id,
                new_display_name.as_deref(),
                new_note.as_deref(),
                updated_at,
            )
            .await?;
        if !updated {
            return Err(AppError::NotFound);
        }

        account.display_name = new_display_name;
        account.note = new_note;
        account.updated_at = updated_at;
        Ok(account)
    }

    /// Update avatar image
    ///
    /// # Arguments
    /// * `image_data` - Image data (will be converted to WebP)
    ///
    /// # Returns
    /// Public URL of the new avatar
    pub async fn update_avatar(&self, image_data: Vec<u8>) -> Result<String, AppError> {
        if image_data.is_empty() {
            return Err(AppError::Validation(
                "avatar image data is empty".to_string(),
            ));
        }

        let mut account = self.get_account().await?;
        let previous_key = account.avatar_s3_key.clone();

        let image_id = EntityId::new().0;
        let (avatar_s3_key, avatar_url) = self.storage.upload_avatar(&image_id, image_data).await?;

        let updated_at = chrono::Utc::now();
        let updated = self
            .db
            .update_account_avatar_key(&account.id, Some(&avatar_s3_key), updated_at)
            .await?;
        if !updated {
            return Err(AppError::NotFound);
        }

        account.avatar_s3_key = Some(avatar_s3_key.clone());
        account.updated_at = updated_at;

        if let Some(old_key) = previous_key.as_deref().filter(|old| *old != avatar_s3_key) {
            if let Err(error) = self.storage.delete(old_key).await {
                tracing::warn!(
                    key = %old_key,
                    error = %error,
                    "failed to delete previous avatar from storage"
                );
            }
        }

        Ok(avatar_url)
    }

    /// Update header image
    ///
    /// # Arguments
    /// * `image_data` - Header image data (will be converted to WebP)
    ///
    /// # Returns
    /// Public URL of the new header image
    pub async fn update_header(&self, image_data: Vec<u8>) -> Result<String, AppError> {
        if image_data.is_empty() {
            return Err(AppError::Validation(
                "header image data is empty".to_string(),
            ));
        }

        let mut account = self.get_account().await?;
        let previous_key = account.header_s3_key.clone();

        let image_id = EntityId::new().0;
        let (header_s3_key, header_url) = self.storage.upload_header(&image_id, image_data).await?;

        let updated_at = chrono::Utc::now();
        let updated = self
            .db
            .update_account_header_key(&account.id, Some(&header_s3_key), updated_at)
            .await?;
        if !updated {
            return Err(AppError::NotFound);
        }

        account.header_s3_key = Some(header_s3_key.clone());
        account.updated_at = updated_at;

        if let Some(old_key) = previous_key.as_deref().filter(|old| *old != header_s3_key) {
            if let Err(error) = self.storage.delete(old_key).await {
                tracing::warn!(
                    key = %old_key,
                    error = %error,
                    "failed to delete previous header from storage"
                );
            }
        }

        Ok(header_url)
    }

    /// Get RSA private key for signing
    ///
    /// Used by federation module for HTTP Signatures.
    pub async fn get_private_key(&self) -> Result<String, AppError> {
        Ok(self.get_account().await?.private_key_pem)
    }

    /// Get RSA public key
    ///
    /// Used for ActivityPub actor endpoint.
    pub async fn get_public_key(&self) -> Result<String, AppError> {
        Ok(self.get_account().await?.public_key_pem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    async fn create_test_db() -> (Arc<Database>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("service-account.db");
        let db = Database::connect(&db_path).await.unwrap();
        (Arc::new(db), temp_dir)
    }

    async fn create_test_storage() -> Arc<MediaStorage> {
        let media = crate::config::MediaStorageConfig {
            bucket: "test-media-bucket".to_string(),
            public_url: "https://media.test.example.com".to_string(),
        };
        let cloudflare = crate::config::CloudflareConfig {
            account_id: "test-account".to_string(),
            r2_access_key_id: "test-access-key".to_string(),
            r2_secret_access_key: "test-secret-key".to_string(),
        };

        Arc::new(MediaStorage::new(&media, &cloudflare).await.unwrap())
    }

    #[tokio::test]
    async fn initialize_account_creates_and_rejects_duplicate() {
        let (db, _temp_dir) = create_test_db().await;
        let storage = create_test_storage().await;
        let service = AccountService::new(db.clone(), storage);

        let account = service.initialize_account(" admin ").await.unwrap();
        assert_eq!(account.username, "admin");
        assert_eq!(account.display_name, Some("admin".to_string()));
        assert!(account.private_key_pem.contains("BEGIN PRIVATE KEY"));
        assert!(account.public_key_pem.contains("BEGIN PUBLIC KEY"));

        let error = service.initialize_account("another").await.unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn initialize_account_rejects_empty_username() {
        let (db, _temp_dir) = create_test_db().await;
        let storage = create_test_storage().await;
        let service = AccountService::new(db, storage);

        let empty = service.initialize_account("").await.unwrap_err();
        assert!(matches!(empty, AppError::Validation(_)));

        let whitespace = service.initialize_account("   ").await.unwrap_err();
        assert!(matches!(whitespace, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn update_profile_and_keys_use_persisted_account() {
        let (db, _temp_dir) = create_test_db().await;
        let storage = create_test_storage().await;
        let service = AccountService::new(db.clone(), storage);

        let account = Account {
            id: EntityId::new().0,
            username: "admin".to_string(),
            display_name: Some("Admin".to_string()),
            note: Some("first".to_string()),
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private-key".to_string(),
            public_key_pem: "public-key".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.upsert_account(&account).await.unwrap();

        let updated = service
            .update_profile(Some("  Display  ".to_string()), Some("  bio  ".to_string()))
            .await
            .unwrap();
        assert_eq!(updated.display_name, Some("Display".to_string()));
        assert_eq!(updated.note, Some("bio".to_string()));

        let cleared = service
            .update_profile(Some("   ".to_string()), Some("".to_string()))
            .await
            .unwrap();
        assert_eq!(cleared.display_name, None);
        assert_eq!(cleared.note, None);

        let private_key = service.get_private_key().await.unwrap();
        let public_key = service.get_public_key().await.unwrap();
        assert_eq!(private_key, "private-key");
        assert_eq!(public_key, "public-key");
    }
}
