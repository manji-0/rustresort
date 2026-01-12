//! Status service
//!
//! Handles status (post/toot) operations including
//! create, delete, favourite, boost, bookmark.

use std::sync::Arc;

use crate::data::{Database, MediaAttachment, PersistedReason, Status, TimelineCache};
use crate::error::AppError;
use crate::storage::MediaStorage;

/// Status service
pub struct StatusService {
    db: Arc<Database>,
    cache: Arc<TimelineCache>,
    storage: Arc<MediaStorage>,
}

impl StatusService {
    /// Create new status service
    pub fn new(db: Arc<Database>, cache: Arc<TimelineCache>, storage: Arc<MediaStorage>) -> Self {
        Self { db, cache, storage }
    }

    // =========================================================================
    // CRUD Operations
    // =========================================================================

    /// Create a new status
    ///
    /// # Arguments
    /// * `content` - HTML content
    /// * `content_warning` - Optional CW text
    /// * `visibility` - public, unlisted, private, direct
    /// * `language` - ISO 639-1 language code
    /// * `in_reply_to_uri` - URI of status being replied to
    /// * `media_ids` - IDs of previously uploaded media
    ///
    /// # Returns
    /// Created status
    ///
    /// # Side Effects
    /// - Inserts into database
    /// - Attaches media
    /// - Triggers federation delivery (via returned status)
    pub async fn create(
        &self,
        _content: String,
        _content_warning: Option<String>,
        _visibility: String,
        _language: Option<String>,
        _in_reply_to_uri: Option<String>,
        _media_ids: Vec<String>,
    ) -> Result<Status, AppError> {
        // TODO:
        // 1. Generate ID and URI
        // 2. Create Status record
        // 3. Insert into DB
        // 4. Attach media if any
        // 5. Return status (federation handled by caller)
        todo!()
    }

    /// Get status by ID
    pub async fn get(&self, _id: &str) -> Result<Status, AppError> {
        // TODO: Get from DB, return NotFound if missing
        todo!()
    }

    /// Get status by URI
    pub async fn get_by_uri(&self, _uri: &str) -> Result<Status, AppError> {
        // TODO: Get from DB, return NotFound if missing
        todo!()
    }

    /// Delete status
    ///
    /// Only allowed for own statuses.
    ///
    /// # Side Effects
    /// - Deletes from database
    /// - Deletes associated media from R2
    /// - Should trigger Delete activity (handled by caller)
    pub async fn delete(&self, _id: &str) -> Result<(), AppError> {
        // TODO:
        // 1. Get status, verify is_local
        // 2. Delete associated media from R2
        // 3. Delete from DB
        todo!()
    }

    // =========================================================================
    // Interactions
    // =========================================================================

    /// Favourite (like) a status
    ///
    /// # Side Effects
    /// - Persists remote status if not already persisted
    /// - Creates favourite record
    /// - Should trigger Like activity (handled by caller)
    pub async fn favourite(&self, _status_uri: &str) -> Result<Status, AppError> {
        // TODO:
        // 1. Get status from cache or fetch
        // 2. Persist if remote (with reason Favourited)
        // 3. Insert favourite record
        // 4. Return status
        todo!()
    }

    /// Unfavourite a status
    pub async fn unfavourite(&self, _status_uri: &str) -> Result<(), AppError> {
        // TODO: Delete favourite record
        // Note: Don't delete the persisted status (might have other reasons)
        todo!()
    }

    /// Bookmark a status
    ///
    /// Local-only, no federation.
    pub async fn bookmark(&self, _status_uri: &str) -> Result<Status, AppError> {
        // TODO:
        // 1. Get status from cache or fetch
        // 2. Persist if remote (with reason Bookmarked)
        // 3. Insert bookmark record
        todo!()
    }

    /// Remove bookmark
    pub async fn unbookmark(&self, _status_uri: &str) -> Result<(), AppError> {
        // TODO: Delete bookmark record
        todo!()
    }

    /// Repost (boost) a status
    ///
    /// # Side Effects
    /// - Persists remote status if not already persisted
    /// - Creates repost record
    /// - Should trigger Announce activity (handled by caller)
    ///
    /// # Returns
    /// The repost status (Announce wrapper)
    pub async fn repost(&self, _status_uri: &str) -> Result<Status, AppError> {
        // TODO:
        // 1. Get status from cache or fetch
        // 2. Persist if remote (with reason Reposted)
        // 3. Create repost record with Announce URI
        // 4. Return original status
        todo!()
    }

    /// Undo repost
    pub async fn unrepost(&self, _status_uri: &str) -> Result<(), AppError> {
        // TODO: Delete repost record
        todo!()
    }

    // =========================================================================
    // Media
    // =========================================================================

    /// Upload media attachment
    ///
    /// # Arguments
    /// * `data` - File data
    /// * `content_type` - MIME type
    /// * `description` - Alt text
    ///
    /// # Returns
    /// Created media attachment (not yet attached to status)
    pub async fn upload_media(
        &self,
        _data: Vec<u8>,
        _content_type: String,
        _description: Option<String>,
    ) -> Result<MediaAttachment, AppError> {
        // TODO:
        // 1. Generate ID
        // 2. Process image/video if needed
        // 3. Generate thumbnail
        // 4. Upload to R2
        // 5. Create media record
        // 6. Insert into DB
        todo!()
    }

    // =========================================================================
    // Internal
    // =========================================================================

    /// Persist a remote status from cache to database
    ///
    /// Called when user interacts with a remote status.
    ///
    /// # Arguments
    /// * `status_uri` - URI of the status
    /// * `reason` - Why we're persisting this
    ///
    /// # Returns
    /// Persisted status
    async fn persist_remote_status(
        &self,
        _status_uri: &str,
        _reason: PersistedReason,
    ) -> Result<Status, AppError> {
        // TODO:
        // 1. Check if already in DB
        // 2. Get from cache
        // 3. Convert to Status with persisted_reason
        // 4. Insert into DB
        todo!()
    }
}
