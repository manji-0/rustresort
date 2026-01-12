//! Account service
//!
//! Handles account-related operations for the single admin user.

use std::sync::Arc;

use crate::data::{Account, Database};
use crate::error::AppError;
use crate::storage::MediaStorage;

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
        // TODO: Get from DB, return NotFound if missing
        todo!()
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
    pub async fn initialize_account(&self, _username: &str) -> Result<Account, AppError> {
        // TODO:
        // 1. Check if account exists
        // 2. Generate RSA keypair
        // 3. Create account record
        // 4. Insert into DB
        todo!()
    }

    /// Update account profile
    ///
    /// # Arguments
    /// * `display_name` - New display name
    /// * `note` - New bio/note (can contain HTML)
    pub async fn update_profile(
        &self,
        _display_name: Option<String>,
        _note: Option<String>,
    ) -> Result<Account, AppError> {
        // TODO: Update account in DB
        todo!()
    }

    /// Update avatar image
    ///
    /// # Arguments
    /// * `image_data` - Image data (will be converted to WebP)
    ///
    /// # Returns
    /// Public URL of the new avatar
    pub async fn update_avatar(&self, _image_data: Vec<u8>) -> Result<String, AppError> {
        // TODO:
        // 1. Process image (resize, convert to WebP)
        // 2. Upload to R2
        // 3. Delete old avatar if exists
        // 4. Update account record
        // 5. Return new URL
        todo!()
    }

    /// Update header image
    pub async fn update_header(&self, _image_data: Vec<u8>) -> Result<String, AppError> {
        // TODO: Similar to update_avatar
        todo!()
    }

    /// Get RSA private key for signing
    ///
    /// Used by federation module for HTTP Signatures.
    pub async fn get_private_key(&self) -> Result<String, AppError> {
        // TODO: Get account and return private_key_pem
        todo!()
    }

    /// Get RSA public key
    ///
    /// Used for ActivityPub actor endpoint.
    pub async fn get_public_key(&self) -> Result<String, AppError> {
        // TODO: Get account and return public_key_pem
        todo!()
    }
}
