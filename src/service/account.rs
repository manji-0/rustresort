//! Account service
//!
//! Handles account-related operations for the single admin user.

use std::sync::Arc;

#[cfg(test)]
use crate::data::Database;
use crate::data::{Account, AccountCredentialsPatch, AccountRepository, EntityId};
use crate::error::AppError;
#[cfg(test)]
use crate::storage::MediaStorage;
use crate::storage::MediaStorageRepository;

#[cfg(test)]
const ACCOUNT_KEY_BITS: usize = 2048;
#[cfg(not(test))]
const ACCOUNT_KEY_BITS: usize = 4096;

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
    db: Arc<dyn AccountRepository>,
    storage: Arc<dyn MediaStorageRepository>,
}

impl AccountService {
    /// Create new account service
    pub fn new<R>(db: Arc<R>, storage: Arc<dyn MediaStorageRepository>) -> Self
    where
        R: AccountRepository + 'static,
    {
        Self { db, storage }
    }

    /// Get the admin account
    ///
    /// # Returns
    /// The single account or error if not initialized
    pub async fn get_account(&self) -> Result<Account, AppError> {
        self.db.get_account().await?.ok_or(AppError::NotFound)
    }

    /// Get follower inbox URIs for outbound federation fan-out.
    pub async fn get_follower_inboxes(&self) -> Result<Vec<String>, AppError> {
        self.db.get_follower_inboxes().await
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
        let username = username.trim();
        if username.is_empty() {
            return Err(AppError::Validation("username cannot be empty".to_string()));
        }

        // Fast-path guard before expensive key generation.
        if self.db.get_account().await?.is_some() {
            return Err(AppError::Validation(
                "account is already initialized".to_string(),
            ));
        }

        let (private_key_pem, public_key_pem) =
            tokio::task::spawn_blocking(|| -> Result<(String, String), String> {
                use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
                use rsa::{RsaPrivateKey, RsaPublicKey};

                let mut rng = rand::thread_rng();
                let private_key = RsaPrivateKey::new(&mut rng, ACCOUNT_KEY_BITS)
                    .map_err(|error| error.to_string())?;
                let public_key = RsaPublicKey::from(&private_key);
                let private_key_pem = private_key
                    .to_pkcs8_pem(LineEnding::LF)
                    .map_err(|error| error.to_string())?
                    .to_string();
                let public_key_pem = public_key
                    .to_public_key_pem(LineEnding::LF)
                    .map_err(|error| error.to_string())?;
                Ok((private_key_pem, public_key_pem))
            })
            .await
            .map_err(|e| AppError::task_join("account key generation", e))?
            .map_err(AppError::internal)?;

        let account = Account {
            id: EntityId::new_string(),
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

        let display_name_patch = display_name.map(normalize_optional_text);
        let note_patch = note.map(normalize_optional_text);
        if display_name_patch.is_none() && note_patch.is_none() {
            return Ok(account);
        }

        let updated_at = chrono::Utc::now();

        let updated = self
            .db
            .patch_account_profile(
                account.id.as_str(),
                display_name_patch.as_ref().map(|value| value.as_deref()),
                note_patch.as_ref().map(|value| value.as_deref()),
                updated_at,
            )
            .await?;
        if !updated {
            return Err(AppError::NotFound);
        }

        if let Some(display_name) = display_name_patch {
            account.display_name = display_name;
        }
        if let Some(note) = note_patch {
            account.note = note;
        }
        account.updated_at = updated_at;
        Ok(account)
    }

    async fn delete_storage_key_best_effort(&self, key: &str, reason: &str) {
        if let Err(error) = self.storage.delete(key).await {
            tracing::warn!(key = %key, error = %error, %reason, "failed to delete media key");
        }
    }

    /// Atomically update account credentials.
    ///
    /// When avatar/header updates are requested, media uploads are performed
    /// first and account fields are patched in one database statement. Any
    /// failure after uploads triggers cleanup of newly uploaded keys.
    pub async fn update_credentials(
        &self,
        display_name: Option<String>,
        note: Option<String>,
        avatar_image_data: Option<Vec<u8>>,
        header_image_data: Option<Vec<u8>>,
    ) -> Result<Account, AppError> {
        if avatar_image_data.is_none() && header_image_data.is_none() {
            return self.update_profile(display_name, note).await;
        }

        if avatar_image_data
            .as_ref()
            .map(|image_data| image_data.is_empty())
            .unwrap_or(false)
        {
            return Err(AppError::Validation(
                "avatar image data is empty".to_string(),
            ));
        }
        if header_image_data
            .as_ref()
            .map(|image_data| image_data.is_empty())
            .unwrap_or(false)
        {
            return Err(AppError::Validation(
                "header image data is empty".to_string(),
            ));
        }

        let account = self.get_account().await?;
        let previous_avatar_key = account.avatar_s3_key.clone();
        let previous_header_key = account.header_s3_key.clone();

        let display_name_patch = display_name.map(normalize_optional_text);
        let note_patch = note.map(normalize_optional_text);

        let mut new_avatar_key = None;
        if let Some(image_data) = avatar_image_data {
            let image_id = EntityId::new_string();
            let (avatar_s3_key, _avatar_url) =
                self.storage.upload_avatar(&image_id, image_data).await?;
            new_avatar_key = Some(avatar_s3_key);
        }

        let mut new_header_key = None;
        if let Some(image_data) = header_image_data {
            let image_id = EntityId::new_string();
            match self.storage.upload_header(&image_id, image_data).await {
                Ok((header_s3_key, _header_url)) => {
                    new_header_key = Some(header_s3_key);
                }
                Err(error) => {
                    if let Some(avatar_s3_key) = new_avatar_key.as_deref() {
                        self.delete_storage_key_best_effort(
                            avatar_s3_key,
                            "rollback uploaded avatar after header upload error",
                        )
                        .await;
                    }
                    return Err(error);
                }
            }
        }

        let updated_at = chrono::Utc::now();
        let updated = match self
            .db
            .patch_account_credentials_if_matches(&AccountCredentialsPatch {
                account_id: account.id.clone(),
                expected_current_avatar_s3_key: previous_avatar_key.clone(),
                expected_current_header_s3_key: previous_header_key.clone(),
                avatar_s3_key: new_avatar_key
                    .clone()
                    .or_else(|| previous_avatar_key.clone()),
                header_s3_key: new_header_key
                    .clone()
                    .or_else(|| previous_header_key.clone()),
                display_name: display_name_patch.clone(),
                note: note_patch.clone(),
                updated_at,
            })
            .await
        {
            Ok(updated) => updated,
            Err(error) => {
                if let Some(avatar_s3_key) = new_avatar_key.as_deref() {
                    self.delete_storage_key_best_effort(
                        avatar_s3_key,
                        "rollback uploaded avatar after database update error",
                    )
                    .await;
                }
                if let Some(header_s3_key) = new_header_key.as_deref() {
                    self.delete_storage_key_best_effort(
                        header_s3_key,
                        "rollback uploaded header after database update error",
                    )
                    .await;
                }
                return Err(error);
            }
        };
        if !updated {
            if let Some(avatar_s3_key) = new_avatar_key.as_deref() {
                self.delete_storage_key_best_effort(
                    avatar_s3_key,
                    "rollback uploaded avatar after concurrent update/not found",
                )
                .await;
            }
            if let Some(header_s3_key) = new_header_key.as_deref() {
                self.delete_storage_key_best_effort(
                    header_s3_key,
                    "rollback uploaded header after concurrent update/not found",
                )
                .await;
            }
            let not_found = match self.db.get_account().await? {
                Some(current) => current.id != account.id,
                None => true,
            };
            if not_found {
                return Err(AppError::NotFound);
            }
            return Err(AppError::Validation(
                "credentials changed concurrently; retry".to_string(),
            ));
        }

        // Re-read persisted row so response reflects committed state even if
        // another request updated non-patched fields concurrently.
        let account = self.get_account().await?;

        if let (Some(old_key), Some(new_key)) = (
            previous_avatar_key.as_deref(),
            account.avatar_s3_key.as_deref(),
        ) && old_key != new_key
        {
            self.delete_storage_key_best_effort(
                old_key,
                "delete previous avatar after credential update",
            )
            .await;
        }
        if let (Some(old_key), Some(new_key)) = (
            previous_header_key.as_deref(),
            account.header_s3_key.as_deref(),
        ) && old_key != new_key
        {
            self.delete_storage_key_best_effort(
                old_key,
                "delete previous header after credential update",
            )
            .await;
        }

        Ok(account)
    }

    /// Update avatar image
    ///
    /// # Arguments
    /// * `image_data` - WebP image bytes (conversion is not performed here)
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

        let image_id = EntityId::new_string();
        let (avatar_s3_key, avatar_url) = self.storage.upload_avatar(&image_id, image_data).await?;

        let updated_at = chrono::Utc::now();
        let updated = match self
            .db
            .update_account_avatar_key_if_matches(
                account.id.as_str(),
                previous_key.as_deref(),
                Some(&avatar_s3_key),
                updated_at,
            )
            .await
        {
            Ok(updated) => updated,
            Err(error) => {
                if let Err(cleanup_error) = self.storage.delete(&avatar_s3_key).await {
                    tracing::warn!(
                        key = %avatar_s3_key,
                        error = %cleanup_error,
                        "failed to rollback uploaded avatar after database update error"
                    );
                }
                return Err(error);
            }
        };
        if !updated {
            if let Err(cleanup_error) = self.storage.delete(&avatar_s3_key).await {
                tracing::warn!(
                    key = %avatar_s3_key,
                    error = %cleanup_error,
                    "failed to rollback uploaded avatar after concurrent update/not found"
                );
            }
            let not_found = match self.db.get_account().await? {
                Some(current) => current.id != account.id,
                None => true,
            };
            if not_found {
                return Err(AppError::NotFound);
            }
            return Err(AppError::Validation(
                "avatar changed concurrently; retry".to_string(),
            ));
        }

        account.avatar_s3_key = Some(avatar_s3_key.clone());
        account.updated_at = updated_at;

        if let Some(old_key) = previous_key.as_deref().filter(|old| *old != avatar_s3_key)
            && let Err(error) = self.storage.delete(old_key).await
        {
            tracing::warn!(
                key = %old_key,
                error = %error,
                "failed to delete previous avatar from storage"
            );
        }

        Ok(avatar_url)
    }

    /// Update header image
    ///
    /// # Arguments
    /// * `image_data` - WebP header image bytes (conversion is not performed here)
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

        let image_id = EntityId::new_string();
        let (header_s3_key, header_url) = self.storage.upload_header(&image_id, image_data).await?;

        let updated_at = chrono::Utc::now();
        let updated = match self
            .db
            .update_account_header_key_if_matches(
                account.id.as_str(),
                previous_key.as_deref(),
                Some(&header_s3_key),
                updated_at,
            )
            .await
        {
            Ok(updated) => updated,
            Err(error) => {
                if let Err(cleanup_error) = self.storage.delete(&header_s3_key).await {
                    tracing::warn!(
                        key = %header_s3_key,
                        error = %cleanup_error,
                        "failed to rollback uploaded header after database update error"
                    );
                }
                return Err(error);
            }
        };
        if !updated {
            if let Err(cleanup_error) = self.storage.delete(&header_s3_key).await {
                tracing::warn!(
                    key = %header_s3_key,
                    error = %cleanup_error,
                    "failed to rollback uploaded header after concurrent update/not found"
                );
            }
            let not_found = match self.db.get_account().await? {
                Some(current) => current.id != account.id,
                None => true,
            };
            if not_found {
                return Err(AppError::NotFound);
            }
            return Err(AppError::Validation(
                "header changed concurrently; retry".to_string(),
            ));
        }

        account.header_s3_key = Some(header_s3_key.clone());
        account.updated_at = updated_at;

        if let Some(old_key) = previous_key.as_deref().filter(|old| *old != header_s3_key)
            && let Err(error) = self.storage.delete(old_key).await
        {
            tracing::warn!(
                key = %old_key,
                error = %error,
                "failed to delete previous header from storage"
            );
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

    async fn create_test_storage() -> Arc<dyn MediaStorageRepository> {
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
            id: EntityId::new_string(),
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

        let note_only = service
            .update_profile(None, Some("updated-note".to_string()))
            .await
            .unwrap();
        assert_eq!(note_only.display_name, Some("Display".to_string()));
        assert_eq!(note_only.note, Some("updated-note".to_string()));

        let display_only = service
            .update_profile(Some("updated-display".to_string()), None)
            .await
            .unwrap();
        assert_eq!(
            display_only.display_name,
            Some("updated-display".to_string())
        );
        assert_eq!(display_only.note, Some("updated-note".to_string()));

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
