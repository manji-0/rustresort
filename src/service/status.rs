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
        Err(AppError::NotImplemented(
            "status creation via service is not implemented yet".to_string(),
        ))
    }

    /// Get status by ID
    pub async fn get(&self, id: &str) -> Result<Status, AppError> {
        self.db.get_status(id).await?.ok_or(AppError::NotFound)
    }

    /// Get status by URI
    pub async fn get_by_uri(&self, uri: &str) -> Result<Status, AppError> {
        self.db
            .get_status_by_uri(uri)
            .await?
            .ok_or(AppError::NotFound)
    }

    /// Delete status
    ///
    /// Only allowed for own statuses.
    ///
    /// # Side Effects
    /// - Deletes from database
    /// - Deletes associated media from R2
    /// - Should trigger Delete activity (handled by caller)
    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        let status = self.get(id).await?;
        self.delete_loaded(&status).await
    }

    /// Delete a preloaded status
    ///
    /// Use this to avoid reloading the same status when the caller
    /// has already resolved it (e.g. API handler needs it for response).
    pub async fn delete_loaded(&self, status: &Status) -> Result<(), AppError> {
        if !status.is_local {
            return Err(AppError::Forbidden);
        }

        self.db.delete_status(&status.id).await?;
        Ok(())
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
    pub async fn favourite(&self, status_uri: &str) -> Result<Status, AppError> {
        let (status, _) = self.favourite_with_id(status_uri).await?;
        Ok(status)
    }

    /// Favourite (like) a status and return favourite row ID.
    pub async fn favourite_with_id(&self, status_uri: &str) -> Result<(Status, String), AppError> {
        let status = match self.db.get_status_by_uri(status_uri).await? {
            Some(status) => status,
            None => {
                self.persist_remote_status(status_uri, PersistedReason::Favourited)
                    .await?
            }
        };

        let favourite_id = self.db.insert_favourite(&status.id).await?;
        Ok((status, favourite_id))
    }

    /// Unfavourite a status
    pub async fn unfavourite(&self, status_uri: &str) -> Result<(), AppError> {
        let status = self.get_by_uri(status_uri).await?;
        self.unfavourite_loaded(&status).await
    }

    /// Bookmark a status
    ///
    /// Local-only, no federation.
    pub async fn bookmark(&self, status_uri: &str) -> Result<Status, AppError> {
        let status = match self.db.get_status_by_uri(status_uri).await? {
            Some(status) => status,
            None => {
                self.persist_remote_status(status_uri, PersistedReason::Bookmarked)
                    .await?
            }
        };

        self.db.insert_bookmark(&status.id).await?;
        Ok(status)
    }

    /// Remove bookmark
    pub async fn unbookmark(&self, status_uri: &str) -> Result<(), AppError> {
        let status = self.get_by_uri(status_uri).await?;
        self.unbookmark_loaded(&status).await
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
        Err(AppError::NotImplemented(
            "repost via service is not implemented yet".to_string(),
        ))
    }

    /// Undo repost
    pub async fn unrepost(&self, _status_uri: &str) -> Result<(), AppError> {
        Err(AppError::NotImplemented(
            "unrepost via service is not implemented yet".to_string(),
        ))
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
        Err(AppError::NotImplemented(
            "media upload via service is not implemented yet".to_string(),
        ))
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
        status_uri: &str,
        reason: PersistedReason,
    ) -> Result<Status, AppError> {
        if let Some(existing) = self.db.get_status_by_uri(status_uri).await? {
            return Ok(existing);
        }

        if let Some(cached) = self.cache.get_by_uri(status_uri).await {
            let status = Status {
                id: cached.id.clone(),
                uri: cached.uri.clone(),
                content: cached.content.clone(),
                content_warning: None,
                visibility: cached.visibility.clone(),
                language: None,
                account_address: cached.account_address.clone(),
                is_local: false,
                in_reply_to_uri: cached.reply_to_uri.clone(),
                boost_of_uri: cached.boost_of_uri.clone(),
                persisted_reason: reason.as_str().to_string(),
                created_at: cached.created_at,
                fetched_at: Some(chrono::Utc::now()),
            };
            self.db.insert_status(&status).await?;
            return Ok(status);
        }

        Err(AppError::NotImplemented(
            "remote status persistence requires federation fetch; not implemented yet".to_string(),
        ))
    }

    /// Favourite by local status ID
    pub async fn favourite_by_id(&self, status_id: &str) -> Result<Status, AppError> {
        let (status, _) = self.favourite_by_id_with_id(status_id).await?;
        Ok(status)
    }

    /// Favourite by local status ID and return favourite row ID.
    pub async fn favourite_by_id_with_id(
        &self,
        status_id: &str,
    ) -> Result<(Status, String), AppError> {
        let status = self.get(status_id).await?;
        let favourite_id = self.db.insert_favourite(status_id).await?;
        Ok((status, favourite_id))
    }

    /// Unfavourite by local status ID
    pub async fn unfavourite_by_id(&self, status_id: &str) -> Result<Status, AppError> {
        let status = self.get(status_id).await?;
        self.unfavourite_loaded(&status).await?;
        Ok(status)
    }

    /// Bookmark by local status ID
    pub async fn bookmark_by_id(&self, status_id: &str) -> Result<Status, AppError> {
        let status = self.get(status_id).await?;
        self.db.insert_bookmark(status_id).await?;
        Ok(status)
    }

    /// Unbookmark by local status ID
    pub async fn unbookmark_by_id(&self, status_id: &str) -> Result<Status, AppError> {
        let status = self.get(status_id).await?;
        self.unbookmark_loaded(&status).await?;
        Ok(status)
    }

    /// Repost a status by its persisted database ID
    pub async fn repost_by_id(
        &self,
        status_id: &str,
        repost_uri: &str,
    ) -> Result<Status, AppError> {
        let status = self.get(status_id).await?;
        self.db.insert_repost(status_id, repost_uri).await?;
        Ok(status)
    }

    /// Repost a status by ActivityPub URI
    pub async fn repost_by_uri(
        &self,
        status_uri: &str,
        repost_uri: &str,
    ) -> Result<Status, AppError> {
        let status = self
            .persist_remote_status(status_uri, PersistedReason::Reposted)
            .await?;
        self.db.insert_repost(&status.id, repost_uri).await?;
        Ok(status)
    }

    /// Unfavourite preloaded status.
    pub async fn unfavourite_loaded(&self, status: &Status) -> Result<(), AppError> {
        self.db.delete_favourite(&status.id).await?;
        Ok(())
    }

    /// Unbookmark preloaded status.
    pub async fn unbookmark_loaded(&self, status: &Status) -> Result<(), AppError> {
        self.db.delete_bookmark(&status.id).await?;
        Ok(())
    }

    /// Undo repost for a status by its persisted database ID
    pub async fn unrepost_by_id(&self, status_id: &str) -> Result<Status, AppError> {
        let status = self.get(status_id).await?;
        self.db.delete_repost(status_id).await?;
        Ok(status)
    }

    /// Undo repost for a status by URI
    pub async fn unrepost_by_uri(&self, status_uri: &str) -> Result<Status, AppError> {
        let status = self.get_by_uri(status_uri).await?;
        self.db.delete_repost(&status.id).await?;
        Ok(status)
    }

    /// Check whether status is favourited
    pub async fn is_favourited(&self, status_id: &str) -> Result<bool, AppError> {
        self.db.is_favourited(status_id).await
    }

    /// Check whether status is bookmarked
    pub async fn is_bookmarked(&self, status_id: &str) -> Result<bool, AppError> {
        self.db.is_bookmarked(status_id).await
    }

    /// Check whether status is reposted
    pub async fn is_reposted(&self, status_id: &str) -> Result<bool, AppError> {
        self.db.is_reposted(status_id).await
    }
}
