//! Status service
//!
//! Handles status (post/toot) operations including
//! create, delete, favourite, boost, bookmark.

use std::sync::Arc;

use crate::data::{Database, EntityId, MediaAttachment, PersistedReason, Status, TimelineCache};
use crate::error::AppError;
use crate::storage::MediaStorage;

const MAX_IMAGE_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
const MAX_VIDEO_UPLOAD_BYTES: usize = 40 * 1024 * 1024;

fn media_file_extension_from_content_type(content_type: &str) -> &'static str {
    match content_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        _ => "bin",
    }
}

/// Status service
pub struct StatusService {
    db: Arc<Database>,
    cache: Arc<TimelineCache>,
    storage: Arc<MediaStorage>,
    base_url: String,
}

impl StatusService {
    /// Create new status service
    pub fn new(
        db: Arc<Database>,
        cache: Arc<TimelineCache>,
        storage: Arc<MediaStorage>,
        base_url: String,
    ) -> Self {
        Self {
            db,
            cache,
            storage,
            base_url,
        }
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
        content: String,
        content_warning: Option<String>,
        visibility: String,
        language: Option<String>,
        in_reply_to_uri: Option<String>,
        media_ids: Vec<String>,
    ) -> Result<Status, AppError> {
        let account = self.db.get_account().await?.ok_or(AppError::NotFound)?;

        let normalized_visibility = visibility.trim().to_ascii_lowercase();
        if !matches!(
            normalized_visibility.as_str(),
            "public" | "unlisted" | "private" | "direct"
        ) {
            return Err(AppError::Validation(
                "visibility must be one of: public, unlisted, private, direct".to_string(),
            ));
        }

        let content = content.trim().to_string();
        if content.is_empty() && media_ids.is_empty() {
            return Err(AppError::Validation(
                "status content or media is required".to_string(),
            ));
        }

        let status_id = EntityId::new().0;
        let uri = format!(
            "{}/users/{}/statuses/{}",
            self.base_url.trim_end_matches('/'),
            account.username,
            status_id
        );
        let status = Status {
            id: status_id,
            uri,
            content: format!("<p>{}</p>", html_escape::encode_text(&content)),
            content_warning,
            visibility: normalized_visibility,
            language: language.or(Some("en".to_string())),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri,
            boost_of_uri: None,
            persisted_reason: "own".to_string(),
            created_at: chrono::Utc::now(),
            fetched_at: None,
        };

        self.persist_local_status_with_media_and_poll(&status, &media_ids, None)
            .await?;

        Ok(status)
    }

    /// Persist a local status with optional media and poll atomically.
    pub async fn persist_local_status_with_media_and_poll(
        &self,
        status: &Status,
        media_ids: &[String],
        poll: Option<(&[String], i64, bool)>,
    ) -> Result<(), AppError> {
        self.db
            .insert_status_with_media_and_poll(status, media_ids, poll)
            .await
    }

    /// Get status by ID
    pub async fn get(&self, id: &str) -> Result<Status, AppError> {
        self.db.get_status(id).await?.ok_or(AppError::NotFound)
    }

    /// Try to get status by ID.
    pub async fn find(&self, id: &str) -> Result<Option<Status>, AppError> {
        self.db.get_status(id).await
    }

    /// Get status by URI
    pub async fn get_by_uri(&self, uri: &str) -> Result<Status, AppError> {
        self.db
            .get_status_by_uri(uri)
            .await?
            .ok_or(AppError::NotFound)
    }

    /// Try to get status by URI.
    pub async fn find_by_uri(&self, uri: &str) -> Result<Option<Status>, AppError> {
        self.db.get_status_by_uri(uri).await
    }

    /// Update an existing status record.
    pub async fn update_loaded(&self, status: &Status) -> Result<(), AppError> {
        self.db.update_status(status).await
    }

    /// Persist status update with atomic edit-history snapshot.
    pub async fn update_with_edit_snapshot(
        &self,
        previous: &Status,
        updated: &Status,
    ) -> Result<(), AppError> {
        self.db
            .update_status_with_edit_snapshot(previous, updated)
            .await
    }

    /// Get media attachments linked to a status.
    pub async fn get_media_by_status(
        &self,
        status_id: &str,
    ) -> Result<Vec<MediaAttachment>, AppError> {
        self.db.get_media_by_status(status_id).await
    }

    /// Get poll metadata for a status if present.
    pub async fn get_poll_by_status_id(
        &self,
        status_id: &str,
    ) -> Result<Option<(String, String, bool, bool, i64, i64)>, AppError> {
        self.db.get_poll_by_status_id(status_id).await
    }

    /// Get poll options for a poll.
    pub async fn get_poll_options(
        &self,
        poll_id: &str,
    ) -> Result<Vec<(String, String, i64)>, AppError> {
        self.db.get_poll_options(poll_id).await
    }

    /// Get favourite activity ID for a status if favourited.
    pub async fn get_favourite_id(&self, status_id: &str) -> Result<Option<String>, AppError> {
        self.db.get_favourite_id(status_id).await
    }

    /// Get repost activity URI for a status if reposted.
    pub async fn get_repost_uri(&self, status_id: &str) -> Result<Option<String>, AppError> {
        self.db.get_repost_uri(status_id).await
    }

    /// Get direct replies for a status URI.
    pub async fn get_replies(&self, in_reply_to_uri: &str) -> Result<Vec<Status>, AppError> {
        self.db.get_status_replies(in_reply_to_uri).await
    }

    /// Get direct replies for a status URI, capped at `limit`.
    pub async fn get_replies_limited(
        &self,
        in_reply_to_uri: &str,
        limit: usize,
    ) -> Result<Vec<Status>, AppError> {
        self.db
            .get_status_replies_limited(in_reply_to_uri, limit)
            .await
    }

    /// Persist an edit-history snapshot for a status.
    pub async fn insert_edit_snapshot(&self, status: &Status) -> Result<(), AppError> {
        self.db
            .insert_status_edit(
                &status.id,
                &status.content,
                status.content_warning.as_deref(),
            )
            .await?;
        Ok(())
    }

    /// Get edit-history snapshots for a status.
    pub async fn get_edit_history(
        &self,
        status_id: &str,
        limit: usize,
    ) -> Result<
        Vec<(
            String,
            String,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
        )>,
        AppError,
    > {
        self.db.get_status_edits(status_id, limit).await
    }

    /// Get a cached idempotency response payload if present.
    pub async fn get_idempotency_response(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<Option<serde_json::Value>, AppError> {
        self.db
            .get_idempotency_response(endpoint, idempotency_key)
            .await
    }

    /// Try to reserve an idempotency key for processing.
    pub async fn reserve_idempotency_key(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<bool, AppError> {
        self.db
            .reserve_idempotency_key(endpoint, idempotency_key)
            .await
    }

    /// Store idempotency response payload.
    pub async fn store_idempotency_response(
        &self,
        endpoint: &str,
        idempotency_key: &str,
        response: &serde_json::Value,
    ) -> Result<(), AppError> {
        self.db
            .store_idempotency_response(endpoint, idempotency_key, response)
            .await
    }

    /// Clear pending idempotency reservation for a key.
    pub async fn clear_pending_idempotency_key(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<(), AppError> {
        self.db
            .clear_pending_idempotency_key(endpoint, idempotency_key)
            .await
    }

    /// Create scheduled status payload.
    pub async fn create_scheduled_status(
        &self,
        scheduled_at: &str,
        status_text: &str,
        visibility: &str,
        content_warning: Option<&str>,
        in_reply_to_id: Option<&str>,
        media_ids: Option<&str>,
        poll_options: Option<&str>,
        poll_expires_in: Option<i64>,
        poll_multiple: bool,
    ) -> Result<String, AppError> {
        self.db
            .create_scheduled_status(
                scheduled_at,
                status_text,
                visibility,
                content_warning,
                in_reply_to_id,
                media_ids,
                poll_options,
                poll_expires_in,
                poll_multiple,
            )
            .await
    }

    /// Get scheduled status response payload by ID.
    pub async fn get_scheduled_status(
        &self,
        id: &str,
    ) -> Result<Option<serde_json::Value>, AppError> {
        self.db.get_scheduled_status(id).await
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
    pub async fn repost(&self, status_uri: &str) -> Result<Status, AppError> {
        let account = self.db.get_account().await?.ok_or(AppError::NotFound)?;
        let repost_id = EntityId::new().0;
        let repost_uri = format!(
            "{}/users/{}/statuses/{}/activity",
            self.base_url.trim_end_matches('/'),
            account.username,
            repost_id
        );
        self.repost_by_uri(status_uri, &repost_uri).await
    }

    /// Undo repost
    pub async fn unrepost(&self, status_uri: &str) -> Result<(), AppError> {
        self.unrepost_by_uri(status_uri).await.map(|_| ())
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
        data: Vec<u8>,
        content_type: String,
        description: Option<String>,
    ) -> Result<MediaAttachment, AppError> {
        if data.is_empty() {
            return Err(AppError::Validation("media data is required".to_string()));
        }

        let normalized_content_type = content_type.trim().to_ascii_lowercase();
        let supported_types = [
            "image/jpeg",
            "image/png",
            "image/gif",
            "image/webp",
            "video/mp4",
        ];
        if !supported_types.contains(&normalized_content_type.as_str()) {
            return Err(AppError::Validation(format!(
                "unsupported media type: {}",
                content_type
            )));
        }

        let max_size = if normalized_content_type.starts_with("image/") {
            MAX_IMAGE_UPLOAD_BYTES
        } else if normalized_content_type.starts_with("video/") {
            MAX_VIDEO_UPLOAD_BYTES
        } else {
            return Err(AppError::Validation(format!(
                "unsupported media type: {}",
                content_type
            )));
        };
        if data.len() > max_size {
            return Err(AppError::Validation(format!(
                "media file too large: exceeds {} bytes",
                max_size
            )));
        }

        let media_id = EntityId::new().0;
        let extension = media_file_extension_from_content_type(&normalized_content_type);
        let s3_key = format!("media/{}.{}", media_id, extension);
        let file_size = data.len() as i64;
        self.storage
            .upload(&s3_key, data, &normalized_content_type)
            .await?;

        let media = MediaAttachment {
            id: media_id,
            status_id: None,
            s3_key: s3_key.clone(),
            thumbnail_s3_key: None,
            content_type: normalized_content_type,
            file_size,
            description,
            blurhash: None,
            width: None,
            height: None,
            created_at: chrono::Utc::now(),
        };

        if let Err(error) = self.db.insert_media(&media).await {
            if let Err(cleanup_error) = self.storage.delete(&s3_key).await {
                tracing::warn!(
                    key = %s3_key,
                    error = %cleanup_error,
                    "failed to cleanup uploaded media after metadata insert error"
                );
            }
            return Err(error);
        }

        Ok(media)
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

    /// Pin status by local status ID.
    pub async fn pin_by_id(&self, status_id: &str) -> Result<Status, AppError> {
        let status = self.get(status_id).await?;
        if !status.is_local {
            return Err(AppError::Validation(
                "Can only pin own statuses".to_string(),
            ));
        }
        self.db.insert_status_pin(status_id).await?;
        Ok(status)
    }

    /// Unpin status by local status ID.
    pub async fn unpin_by_id(&self, status_id: &str) -> Result<Status, AppError> {
        let status = self.get(status_id).await?;
        if !status.is_local {
            return Err(AppError::Validation(
                "Can only pin own statuses".to_string(),
            ));
        }
        self.db.delete_status_pin(status_id).await?;
        Ok(status)
    }

    /// Mute conversation by status ID.
    pub async fn mute_by_id(&self, status_id: &str) -> Result<Status, AppError> {
        let status = self.get(status_id).await?;
        let thread_uri = self.db.resolve_thread_root_uri(&status).await?;
        self.db.insert_muted_thread(&thread_uri).await?;
        Ok(status)
    }

    /// Unmute conversation by status ID.
    pub async fn unmute_by_id(&self, status_id: &str) -> Result<Status, AppError> {
        let status = self.get(status_id).await?;
        let thread_uri = self.db.resolve_thread_root_uri(&status).await?;
        self.db.delete_muted_thread(&thread_uri).await?;
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

    /// Check whether status conversation is muted.
    pub async fn is_muted(&self, status_id: &str) -> Result<bool, AppError> {
        let status = self.get(status_id).await?;
        self.is_muted_loaded(&status).await
    }

    /// Check whether preloaded status conversation is muted.
    pub async fn is_muted_loaded(&self, status: &Status) -> Result<bool, AppError> {
        let thread_uri = self.db.resolve_thread_root_uri(status).await?;
        self.db.is_thread_muted(&thread_uri).await
    }

    /// Check whether status is pinned.
    pub async fn is_pinned(&self, status_id: &str) -> Result<bool, AppError> {
        self.db.is_status_pinned(status_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    use crate::data::{Account, EntityId};

    async fn create_test_db() -> (Arc<Database>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("service-status.db");
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

    async fn seed_account(db: &Database, username: &str) {
        let account = Account {
            id: EntityId::new().0,
            username: username.to_string(),
            display_name: Some(username.to_string()),
            note: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private-key".to_string(),
            public_key_pem: "public-key".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.upsert_account(&account).await.unwrap();
    }

    async fn create_service(db: Arc<Database>) -> StatusService {
        let cache = Arc::new(TimelineCache::new(64).await.unwrap());
        let storage = create_test_storage().await;
        StatusService::new(db, cache, storage, "https://test.example.com".to_string())
    }

    #[tokio::test]
    async fn create_persists_local_status() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let service = create_service(db.clone()).await;

        let status = service
            .create(
                "hello".to_string(),
                Some("cw".to_string()),
                "public".to_string(),
                Some("en".to_string()),
                None,
                vec![],
            )
            .await
            .unwrap();

        assert!(status.uri.ends_with(&format!("/statuses/{}", status.id)));
        assert_eq!(status.visibility, "public");
        assert_eq!(status.content, "<p>hello</p>");

        let persisted = db.get_status(&status.id).await.unwrap().unwrap();
        assert_eq!(persisted.uri, status.uri);
        assert_eq!(persisted.content, "<p>hello</p>");
        assert!(persisted.is_local);
    }

    #[tokio::test]
    async fn create_rejects_invalid_input() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let service = create_service(db).await;

        let invalid_visibility = service
            .create(
                "hello".to_string(),
                None,
                "friends-only".to_string(),
                None,
                None,
                vec![],
            )
            .await
            .unwrap_err();
        assert!(matches!(invalid_visibility, AppError::Validation(_)));

        let empty_content = service
            .create(
                "   ".to_string(),
                None,
                "public".to_string(),
                None,
                None,
                vec![],
            )
            .await
            .unwrap_err();
        assert!(matches!(empty_content, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn repost_and_unrepost_roundtrip_by_uri() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let service = create_service(db.clone()).await;

        let status = Status {
            id: EntityId::new().0,
            uri: "https://remote.example/users/alice/statuses/1".to_string(),
            content: "<p>remote</p>".to_string(),
            content_warning: None,
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: "alice@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: PersistedReason::Favourited.as_str().to_string(),
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        };
        db.insert_status(&status).await.unwrap();

        let reposted = service.repost(&status.uri).await.unwrap();
        assert_eq!(reposted.id, status.id);
        assert!(db.is_reposted(&status.id).await.unwrap());

        service.unrepost(&status.uri).await.unwrap();
        assert!(!db.is_reposted(&status.id).await.unwrap());
    }

    #[tokio::test]
    async fn upload_media_rejects_invalid_payload_before_upload() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let service = create_service(db).await;

        let empty = service
            .upload_media(Vec::new(), "image/png".to_string(), None)
            .await
            .unwrap_err();
        assert!(matches!(empty, AppError::Validation(_)));

        let unsupported = service
            .upload_media(vec![1, 2, 3], "text/plain".to_string(), None)
            .await
            .unwrap_err();
        assert!(matches!(unsupported, AppError::Validation(_)));

        let oversized = service
            .upload_media(
                vec![0_u8; 10 * 1024 * 1024 + 1],
                "image/png".to_string(),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(oversized, AppError::Validation(_)));
    }
}
