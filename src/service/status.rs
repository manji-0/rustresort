//! Status service
//!
//! Handles status (post/toot) operations including
//! create, delete, favourite, boost, bookmark.

use std::{collections::HashSet, sync::Arc};

use super::{StreamEvent, StreamTarget, StreamingEventBus};
#[cfg(test)]
use crate::data::Database;
use crate::data::{
    EntityId, MediaAttachment, PersistedReason, ScheduledStatusInsert, Status, StatusRepository,
    StatusVisibility, TimelineCache,
};
use crate::error::AppError;
#[cfg(test)]
use crate::storage::MediaStorage;
use crate::storage::MediaStorageRepository;

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

fn normalize_remote_status_uri(status_uri: &str) -> Result<url::Url, AppError> {
    let parsed = url::Url::parse(status_uri).map_err(|_| {
        AppError::Validation("status URI must be a valid absolute http(s) URL".to_string())
    })?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(AppError::Validation(
            "status URI must use http or https".to_string(),
        ));
    }
    if parsed.host_str().is_none() {
        return Err(AppError::Validation(
            "status URI must include a host".to_string(),
        ));
    }
    Ok(parsed)
}

fn derive_remote_account_address(status_uri: &url::Url) -> String {
    let host = status_uri
        .host_str()
        .unwrap_or("unknown.invalid")
        .to_ascii_lowercase();
    let authority_host = if host.contains(':') {
        format!("[{}]", host)
    } else {
        host.clone()
    };
    let domain = match status_uri.port() {
        Some(port) => format!("{}:{}", authority_host, port),
        None => authority_host,
    };

    let username = status_uri
        .path_segments()
        .and_then(|segments| {
            let path_segments: Vec<&str> = segments.filter(|segment| !segment.is_empty()).collect();
            path_segments
                .windows(2)
                .find_map(|window| {
                    if matches!(window[0], "users" | "accounts") && !window[1].is_empty() {
                        Some(window[1])
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    path_segments.iter().find_map(|segment| {
                        segment
                            .strip_prefix('@')
                            .and_then(|value| (!value.is_empty()).then_some(value))
                    })
                })
        })
        .unwrap_or("unknown")
        .to_ascii_lowercase();

    format!("{}@{}", username, domain)
}

fn default_port_for_scheme(scheme: &str) -> Option<u16> {
    match scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

fn format_authority_host(host: &str) -> String {
    if host.contains(':') {
        format!("[{}]", host)
    } else {
        host.to_string()
    }
}

fn local_account_address_candidates(base_url: &str, local_username: &str) -> Vec<String> {
    let Ok(parsed) = url::Url::parse(base_url) else {
        return Vec::new();
    };
    let Some(host) = parsed.host_str() else {
        return Vec::new();
    };

    let authority_host = format_authority_host(host);
    let mut candidates = vec![format!("{}@{}", local_username, authority_host)];
    if let Some(port) = parsed.port() {
        candidates.push(format!("{}@{}:{}", local_username, authority_host, port));
    }
    candidates
}

/// Status service
pub struct StatusService {
    db: Arc<dyn StatusRepository>,
    cache: Arc<TimelineCache>,
    storage: Arc<dyn MediaStorageRepository>,
    streaming_event_bus: Arc<dyn StreamingEventBus>,
    base_url: String,
    local_username: String,
    local_default_port: Option<u16>,
    local_account_address_candidates: Vec<String>,
}

impl StatusService {
    /// Create new status service
    pub fn new<R>(
        db: Arc<R>,
        cache: Arc<TimelineCache>,
        storage: Arc<dyn MediaStorageRepository>,
        streaming_event_bus: Arc<dyn StreamingEventBus>,
        base_url: String,
        local_username: String,
    ) -> Self
    where
        R: StatusRepository + 'static,
    {
        let (local_default_port, local_account_address_candidates) = url::Url::parse(&base_url)
            .ok()
            .map(|parsed| {
                (
                    default_port_for_scheme(parsed.scheme()),
                    local_account_address_candidates(&base_url, &local_username),
                )
            })
            .unwrap_or_else(|| (None, Vec::new()));

        Self {
            db,
            cache,
            storage,
            streaming_event_bus,
            base_url,
            local_username,
            local_default_port,
            local_account_address_candidates,
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
        let normalized_visibility =
            StatusVisibility::parse(visibility.trim()).ok_or_else(|| {
                AppError::Validation(
                    "visibility must be one of: public, unlisted, private, direct".to_string(),
                )
            })?;

        let content = content.trim().to_string();
        if content.is_empty() && media_ids.is_empty() {
            return Err(AppError::Validation(
                "status content or media is required".to_string(),
            ));
        }

        let status_id = EntityId::new_string();
        let uri = format!(
            "{}/users/{}/statuses/{}",
            self.base_url.trim_end_matches('/'),
            self.local_username,
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
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
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
            .await?;
        self.invalidate_cached_status(status).await;
        self.publish_status_update(status).await?;
        Ok(())
    }

    /// Get status by ID
    pub async fn get(&self, id: &str) -> Result<Status, AppError> {
        self.find(id).await?.ok_or(AppError::NotFound)
    }

    /// Try to get status by ID.
    pub async fn find(&self, id: &str) -> Result<Option<Status>, AppError> {
        if let Some(status) = self.db.get_status(id).await? {
            return Ok(Some(status));
        }
        if id.starts_with("http://") || id.starts_with("https://") {
            return self.db.get_status_by_uri(id).await;
        }
        Ok(None)
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

    /// Ensure a remote status identified by URI is persisted.
    ///
    /// First checks the database, then cache, and finally inserts a placeholder
    /// row derived from the URI when no cached payload exists.
    pub async fn ensure_remote_status_persisted(
        &self,
        status_uri: &str,
        reason: PersistedReason,
    ) -> Result<Status, AppError> {
        if let Some(status) = self.db.get_status_by_uri(status_uri).await? {
            return Ok(status);
        }
        self.persist_remote_status(status_uri, reason).await
    }

    /// Update an existing status record.
    pub async fn update_loaded(&self, status: &Status) -> Result<(), AppError> {
        self.db.update_status(status).await?;
        self.invalidate_cached_status(status).await;
        self.publish_status_update(status).await?;
        Ok(())
    }

    /// Persist status update with atomic edit-history snapshot.
    pub async fn update_with_edit_snapshot(
        &self,
        previous: &Status,
        updated: &Status,
    ) -> Result<(), AppError> {
        self.db
            .update_status_with_edit_snapshot(previous, updated)
            .await?;
        self.invalidate_cached_status(previous).await;
        self.invalidate_cached_status(updated).await;
        self.publish_status_update(updated).await?;
        Ok(())
    }

    /// Persist status update with atomic edit snapshot and optional media replacement.
    pub async fn update_with_edit_snapshot_and_media(
        &self,
        previous: &Status,
        updated: &Status,
        media_ids: Option<&[String]>,
    ) -> Result<(), AppError> {
        self.db
            .update_status_with_edit_snapshot_and_media(previous, updated, media_ids)
            .await?;
        self.invalidate_cached_status(previous).await;
        self.invalidate_cached_status(updated).await;
        self.publish_status_update(updated).await?;
        Ok(())
    }

    /// Get media attachments linked to a status.
    pub async fn get_media_by_status(
        &self,
        status_id: &str,
    ) -> Result<Vec<MediaAttachment>, AppError> {
        self.db.get_media_by_status(status_id).await
    }

    /// Replace all media attachments associated with a status.
    pub async fn replace_media_for_status(
        &self,
        status_id: &str,
        media_ids: &[String],
    ) -> Result<(), AppError> {
        self.db.replace_status_media(status_id, media_ids).await
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
        request: &ScheduledStatusInsert,
    ) -> Result<String, AppError> {
        self.db.create_scheduled_status(request).await
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
        self.invalidate_cached_status(status).await;
        self.publish_status_delete(status).await?;
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
        let status = self
            .ensure_remote_status_persisted(status_uri, PersistedReason::Favourited)
            .await?;

        let favourite_id = self.db.insert_favourite(&status.id).await?;
        self.publish_status_update(&status).await?;
        Ok((status, favourite_id))
    }

    /// Unfavourite a status
    pub async fn unfavourite(&self, status_uri: &str) -> Result<(), AppError> {
        let status = self
            .ensure_remote_status_persisted(status_uri, PersistedReason::Favourited)
            .await?;
        self.unfavourite_loaded(&status).await
    }

    /// Bookmark a status
    ///
    /// Local-only, no federation.
    pub async fn bookmark(&self, status_uri: &str) -> Result<Status, AppError> {
        let status = self
            .ensure_remote_status_persisted(status_uri, PersistedReason::Bookmarked)
            .await?;

        self.db.insert_bookmark(&status.id).await?;
        self.publish_status_update(&status).await?;
        Ok(status)
    }

    /// Remove bookmark
    pub async fn unbookmark(&self, status_uri: &str) -> Result<(), AppError> {
        let status = self
            .ensure_remote_status_persisted(status_uri, PersistedReason::Bookmarked)
            .await?;
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
        let repost_id = EntityId::new_string();
        let repost_uri = format!(
            "{}/users/{}/statuses/{}/activity",
            self.base_url.trim_end_matches('/'),
            self.local_username,
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

        let media_id = EntityId::new_string();
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
            focus_x: None,
            focus_y: None,
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
                visibility: StatusVisibility::parse(&cached.visibility)
                    .unwrap_or(StatusVisibility::Private),
                language: None,
                account_address: cached.account_address.clone(),
                is_local: false,
                in_reply_to_uri: cached.reply_to_uri.clone(),
                boost_of_uri: cached.boost_of_uri.clone(),
                quote_of_uri: cached.quote_of_uri.clone(),
                persisted_reason: reason,
                created_at: cached.created_at,
                fetched_at: Some(chrono::Utc::now()),
            };
            self.db.insert_status(&status).await?;
            self.invalidate_cached_status(&status).await;
            return Ok(status);
        }

        let normalized = normalize_remote_status_uri(status_uri)?;
        let normalized_uri = normalized.to_string();
        if normalized_uri != status_uri {
            if let Some(existing) = self.db.get_status_by_uri(&normalized_uri).await? {
                return Ok(existing);
            }
            if let Some(cached) = self.cache.get_by_uri(&normalized_uri).await {
                let status = Status {
                    id: cached.id.clone(),
                    uri: cached.uri.clone(),
                    content: cached.content.clone(),
                    content_warning: None,
                    visibility: StatusVisibility::parse(&cached.visibility)
                        .unwrap_or(StatusVisibility::Private),
                    language: None,
                    account_address: cached.account_address.clone(),
                    is_local: false,
                    in_reply_to_uri: cached.reply_to_uri.clone(),
                    boost_of_uri: cached.boost_of_uri.clone(),
                    quote_of_uri: cached.quote_of_uri.clone(),
                    persisted_reason: reason,
                    created_at: cached.created_at,
                    fetched_at: Some(chrono::Utc::now()),
                };
                self.db.insert_status(&status).await?;
                self.invalidate_cached_status(&status).await;
                return Ok(status);
            }
        }

        let now = chrono::Utc::now();
        let placeholder = Status {
            // Keep placeholder IDs aligned with the canonical status URI so API
            // fields that surface in_reply_to_id can resolve through /statuses/:id.
            id: normalized_uri.clone(),
            uri: normalized_uri,
            content: String::new(),
            content_warning: None,
            visibility: StatusVisibility::Private,
            language: None,
            account_address: derive_remote_account_address(&normalized),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: reason,
            created_at: now,
            fetched_at: Some(now),
        };
        self.db.insert_status(&placeholder).await?;
        self.invalidate_cached_status(&placeholder).await;
        Ok(placeholder)
    }

    async fn stream_targets_for_status(
        &self,
        status: &Status,
    ) -> Result<Vec<StreamTarget>, AppError> {
        let mut targets: HashSet<StreamTarget> = HashSet::new();
        if self.should_publish_to_user_stream(status) {
            targets.insert(StreamTarget::User {
                account_id: self.local_username.clone(),
            });
        }

        match status.visibility {
            StatusVisibility::Public => {
                targets.insert(StreamTarget::Public);
                if status.is_local {
                    targets.insert(StreamTarget::PublicLocal);
                }
            }
            StatusVisibility::Unlisted => {}
            StatusVisibility::Direct => {
                targets.insert(StreamTarget::Direct {
                    account_id: self.local_username.clone(),
                });
            }
            StatusVisibility::Private => {}
        }

        if matches!(status.visibility, StatusVisibility::Public) {
            for hashtag in crate::data::extract_hashtags_from_content(status.content.as_str()) {
                targets.insert(StreamTarget::Hashtag { hashtag });
            }
        }

        if !matches!(status.visibility, StatusVisibility::Direct)
            && status.is_local
            && status.account_address.trim().is_empty()
        {
            let mut local_keys = self.local_account_address_candidates.clone();
            if let Some(account) = self.db.get_account().await? {
                local_keys.push(account.id);
            }
            for local_key in local_keys {
                for list_id in self
                    .db
                    .get_list_ids_for_account(local_key.as_str(), self.local_default_port)
                    .await?
                {
                    targets.insert(StreamTarget::List { list_id });
                }
            }
        } else if !matches!(status.visibility, StatusVisibility::Direct) {
            let account_address = status.account_address.trim();
            if !account_address.is_empty() {
                for list_id in self
                    .db
                    .get_list_ids_for_account(account_address, self.local_default_port)
                    .await?
                {
                    targets.insert(StreamTarget::List { list_id });
                }
            }
        }

        Ok(targets.into_iter().collect())
    }

    fn should_publish_to_user_stream(&self, status: &Status) -> bool {
        if status.is_local {
            return true;
        }

        matches!(
            status.persisted_reason,
            PersistedReason::Timeline
                | PersistedReason::CacheOnly
                | PersistedReason::Reposted
                | PersistedReason::Favourited
                | PersistedReason::Bookmarked
                | PersistedReason::ReplyToOwn
        )
    }

    async fn publish_status_update(&self, status: &Status) -> Result<(), AppError> {
        let event = StreamEvent::Update {
            payload: serde_json::json!({
                "id": status.id.as_str(),
                "uri": status.uri.as_str(),
                "visibility": status.visibility.as_str(),
                "created_at": status.created_at.to_rfc3339(),
            }),
            targets: self.stream_targets_for_status(status).await?,
        };
        self.streaming_event_bus.publish(event).await
    }

    async fn publish_status_delete(&self, status: &Status) -> Result<(), AppError> {
        let event = StreamEvent::Delete {
            payload: serde_json::json!({
                "id": status.id.as_str(),
                "uri": status.uri.as_str(),
            }),
            targets: self.stream_targets_for_status(status).await?,
        };
        self.streaming_event_bus.publish(event).await
    }
    async fn invalidate_cached_status(&self, status: &Status) {
        self.cache.remove(&status.id).await;
        self.cache.remove_by_uri(&status.uri).await;
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
        let favourite_id = self.db.insert_favourite(&status.id).await?;
        self.publish_status_update(&status).await?;
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
        self.db.insert_bookmark(&status.id).await?;
        self.publish_status_update(&status).await?;
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
        self.db.insert_repost(&status.id, repost_uri).await?;
        self.publish_status_update(&status).await?;
        Ok(status)
    }

    /// Repost a status by ActivityPub URI
    pub async fn repost_by_uri(
        &self,
        status_uri: &str,
        repost_uri: &str,
    ) -> Result<Status, AppError> {
        let status = self
            .ensure_remote_status_persisted(status_uri, PersistedReason::Reposted)
            .await?;
        self.db.insert_repost(&status.id, repost_uri).await?;
        self.publish_status_update(&status).await?;
        Ok(status)
    }

    /// Unfavourite preloaded status.
    pub async fn unfavourite_loaded(&self, status: &Status) -> Result<(), AppError> {
        self.db.delete_favourite(&status.id).await?;
        self.publish_status_update(status).await?;
        Ok(())
    }

    /// Unbookmark preloaded status.
    pub async fn unbookmark_loaded(&self, status: &Status) -> Result<(), AppError> {
        self.db.delete_bookmark(&status.id).await?;
        self.publish_status_update(status).await?;
        Ok(())
    }

    /// Undo repost for a status by its persisted database ID
    pub async fn unrepost_by_id(&self, status_id: &str) -> Result<Status, AppError> {
        let status = self.get(status_id).await?;
        self.db.delete_repost(&status.id).await?;
        self.publish_status_update(&status).await?;
        Ok(status)
    }

    /// Undo repost for a status by URI
    pub async fn unrepost_by_uri(&self, status_uri: &str) -> Result<Status, AppError> {
        let status = self
            .ensure_remote_status_persisted(status_uri, PersistedReason::Reposted)
            .await?;
        self.db.delete_repost(&status.id).await?;
        self.publish_status_update(&status).await?;
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
        self.db.insert_status_pin(&status.id).await?;
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
        self.db.delete_status_pin(&status.id).await?;
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
    use tokio::time::{Duration, timeout};

    use crate::data::{Account, CachedStatus, EntityId};
    use crate::service::{BroadcastEventBus, StreamEvent, StreamingEventBus};

    async fn create_test_db() -> (Arc<Database>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("service-status.db");
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

    async fn seed_account(db: &Database, username: &str) {
        let account = Account {
            id: EntityId::new_string(),
            username: username.to_string(),
            display_name: Some(username.to_string()),
            note: None,
            also_known_as: None,
            moved_to_uri: None,
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
        create_service_with_bus(db, Arc::new(BroadcastEventBus::new(64))).await
    }

    async fn create_service_with_bus(
        db: Arc<Database>,
        bus: Arc<BroadcastEventBus>,
    ) -> StatusService {
        let cache = Arc::new(TimelineCache::new(64).await.unwrap());
        let storage = create_test_storage().await;
        StatusService::new(
            db,
            cache,
            storage,
            bus,
            "https://test.example.com".to_string(),
            "testuser".to_string(),
        )
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
        assert_eq!(status.visibility, crate::data::StatusVisibility::Public);
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
    async fn get_and_find_fallback_to_uri_lookup_for_legacy_remote_rows() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let service = create_service(db.clone()).await;

        let status = Status {
            id: EntityId::new_string(),
            uri: "https://remote.example/users/alice/statuses/legacy".to_string(),
            content: "<p>legacy remote row</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Private,
            language: Some("en".to_string()),
            account_address: "alice@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Favourited,
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        };
        db.insert_status(&status).await.unwrap();

        let fetched = service.get(&status.uri).await.unwrap();
        assert_eq!(fetched.id, status.id);
        assert_eq!(fetched.uri, status.uri);

        let found = service.find(&status.uri).await.unwrap();
        assert_eq!(
            found.as_ref().map(|value| value.id.as_str()),
            Some(status.id.as_str())
        );
    }

    #[tokio::test]
    async fn by_id_mutations_use_loaded_status_id_when_input_is_uri() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let service = create_service(db.clone()).await;

        let remote_status = Status {
            id: EntityId::new_string(),
            uri: "https://remote.example/users/alice/statuses/uri-input".to_string(),
            content: "<p>remote</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "alice@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Favourited,
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        };
        db.insert_status(&remote_status).await.unwrap();

        service
            .favourite_by_id_with_id(&remote_status.uri)
            .await
            .unwrap();
        assert!(db.is_favourited(&remote_status.id).await.unwrap());

        service.bookmark_by_id(&remote_status.uri).await.unwrap();
        assert!(db.is_bookmarked(&remote_status.id).await.unwrap());

        service
            .repost_by_id(
                &remote_status.uri,
                "https://test.example.com/activities/repost/1",
            )
            .await
            .unwrap();
        assert!(db.is_reposted(&remote_status.id).await.unwrap());
        service.unrepost_by_id(&remote_status.uri).await.unwrap();
        assert!(!db.is_reposted(&remote_status.id).await.unwrap());

        let local_status = Status {
            id: EntityId::new_string(),
            uri: "https://test.example.com/users/testuser/statuses/local-uri-input".to_string(),
            content: "<p>local</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };
        db.insert_status(&local_status).await.unwrap();

        service.pin_by_id(&local_status.uri).await.unwrap();
        assert!(db.is_status_pinned(&local_status.id).await.unwrap());
        service.unpin_by_id(&local_status.uri).await.unwrap();
        assert!(!db.is_status_pinned(&local_status.id).await.unwrap());
    }

    #[tokio::test]
    async fn repost_and_unrepost_roundtrip_by_uri() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let service = create_service(db.clone()).await;

        let status = Status {
            id: EntityId::new_string(),
            uri: "https://remote.example/users/alice/statuses/1".to_string(),
            content: "<p>remote</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "alice@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Favourited,
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
    async fn favourite_persists_placeholder_status_when_cache_misses() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let service = create_service(db.clone()).await;

        let remote_uri = "https://remote.example/users/alice/statuses/42";
        let status = service.favourite(remote_uri).await.unwrap();
        assert_eq!(status.id, remote_uri);
        assert_eq!(status.uri, remote_uri);
        assert!(!status.is_local);
        assert_eq!(status.account_address, "alice@remote.example");
        assert!(db.is_favourited(&status.id).await.unwrap());

        let persisted = db.get_status_by_uri(remote_uri).await.unwrap();
        assert!(persisted.is_some());
    }

    #[tokio::test]
    async fn favourite_uses_cached_non_http_uri_before_validation() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let service = create_service(db.clone()).await;

        let remote_uri = "tag:remote.example,2026:status-42";
        service
            .cache
            .insert(CachedStatus {
                id: EntityId::new_string(),
                uri: remote_uri.to_string(),
                content: "<p>cached non-http status</p>".to_string(),
                account_address: "alice@remote.example".to_string(),
                created_at: Utc::now(),
                visibility: "private".to_string(),
                attachments: vec![],
                reply_to_uri: None,
                boost_of_uri: None,
                quote_of_uri: None,
            })
            .await;

        let status = service.favourite(remote_uri).await.unwrap();
        assert_eq!(status.uri, remote_uri);
        assert!(!status.is_local);
        assert_eq!(status.account_address, "alice@remote.example");

        let persisted = db.get_status_by_uri(remote_uri).await.unwrap();
        assert!(persisted.is_some());
        assert!(db.is_favourited(&status.id).await.unwrap());
    }

    #[tokio::test]
    async fn placeholder_status_is_non_public_when_cache_misses() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let service = create_service(db.clone()).await;

        let remote_uri = "https://remote.example/users/alice/statuses/43";
        let status = service.favourite(remote_uri).await.unwrap();
        assert_eq!(status.id, remote_uri);
        assert_eq!(status.visibility, crate::data::StatusVisibility::Private);

        let persisted = db.get_status_by_uri(remote_uri).await.unwrap().unwrap();
        assert_eq!(persisted.visibility, crate::data::StatusVisibility::Private);
    }

    #[tokio::test]
    async fn favourite_preserves_fragment_in_remote_status_uri() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let service = create_service(db.clone()).await;

        let remote_uri = "https://remote.example/users/alice/statuses/44#activity";
        let status = service.favourite(remote_uri).await.unwrap();
        assert_eq!(status.uri, remote_uri);

        let persisted = db.get_status_by_uri(remote_uri).await.unwrap();
        assert!(persisted.is_some());
        let without_fragment = "https://remote.example/users/alice/statuses/44";
        assert!(
            db.get_status_by_uri(without_fragment)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn favourite_rejects_invalid_remote_status_uri() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let service = create_service(db).await;

        let error = service.favourite("not-a-valid-uri").await.unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn create_publishes_update_to_hashtag_stream() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let bus = Arc::new(BroadcastEventBus::new(64));
        let mut receiver = bus.subscribe_hashtag("rust").await.unwrap();
        let service = create_service_with_bus(db, bus).await;

        let status = service
            .create(
                "hello #Rust".to_string(),
                None,
                "public".to_string(),
                Some("en".to_string()),
                None,
                vec![],
            )
            .await
            .unwrap();

        let event = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        match event {
            StreamEvent::Update { payload, .. } => {
                assert_eq!(
                    payload.get("id").and_then(serde_json::Value::as_str),
                    Some(status.id.as_str())
                );
            }
            other => panic!("expected update event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn favourite_by_id_publishes_update_to_user_stream() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let bus = Arc::new(BroadcastEventBus::new(64));
        let service = create_service_with_bus(db, bus.clone()).await;

        let status = service
            .create(
                "interaction target".to_string(),
                None,
                "public".to_string(),
                Some("en".to_string()),
                None,
                vec![],
            )
            .await
            .unwrap();

        let mut receiver = bus.subscribe_user("testuser").await.unwrap();
        let _ = service.favourite_by_id(&status.id).await.unwrap();

        let event = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            StreamEvent::Update { payload, .. } => {
                assert_eq!(
                    payload.get("id").and_then(serde_json::Value::as_str),
                    Some(status.id.as_str())
                );
            }
            other => panic!("expected update event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bookmark_by_id_publishes_update_to_user_stream() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let bus = Arc::new(BroadcastEventBus::new(64));
        let service = create_service_with_bus(db, bus.clone()).await;

        let status = service
            .create(
                "bookmark target".to_string(),
                None,
                "public".to_string(),
                Some("en".to_string()),
                None,
                vec![],
            )
            .await
            .unwrap();

        let mut receiver = bus.subscribe_user("testuser").await.unwrap();
        let _ = service.bookmark_by_id(&status.id).await.unwrap();

        let event = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            StreamEvent::Update { payload, .. } => {
                assert_eq!(
                    payload.get("id").and_then(serde_json::Value::as_str),
                    Some(status.id.as_str())
                );
            }
            other => panic!("expected update event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn repost_by_id_publishes_update_to_user_stream() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let bus = Arc::new(BroadcastEventBus::new(64));
        let service = create_service_with_bus(db, bus.clone()).await;

        let status = service
            .create(
                "repost target".to_string(),
                None,
                "public".to_string(),
                Some("en".to_string()),
                None,
                vec![],
            )
            .await
            .unwrap();

        let repost_uri = format!(
            "https://test.example.com/users/testuser/statuses/{}/activity",
            EntityId::new_string()
        );
        let mut receiver = bus.subscribe_user("testuser").await.unwrap();
        let _ = service
            .repost_by_id(&status.id, repost_uri.as_str())
            .await
            .unwrap();

        let event = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            StreamEvent::Update { payload, .. } => {
                assert_eq!(
                    payload.get("id").and_then(serde_json::Value::as_str),
                    Some(status.id.as_str())
                );
            }
            other => panic!("expected update event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_publishes_update_to_matching_list_stream_with_equivalent_default_port_address()
    {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let list_id = db.create_list("friends", "list").await.unwrap();
        db.add_account_to_list(&list_id, "alice@example.com:443")
            .await
            .unwrap();

        let bus = Arc::new(BroadcastEventBus::new(64));
        let mut receiver = bus.subscribe_list(&list_id).await.unwrap();
        let service = create_service_with_bus(db.clone(), bus).await;

        let status = Status {
            id: EntityId::new_string(),
            uri: "https://remote.example/users/alice/statuses/list-target".to_string(),
            content: "<p>remote #tag</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "alice@example.com".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Favourited,
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        };
        db.insert_status(&status).await.unwrap();

        service.update_loaded(&status).await.unwrap();

        let event = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event, StreamEvent::Update { .. }));
    }

    #[tokio::test]
    async fn create_local_status_publishes_to_list_stream_when_list_contains_local_account_id() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let account = db.get_account().await.unwrap().unwrap();
        let list_id = db.create_list("local-posts", "list").await.unwrap();
        db.add_account_to_list(&list_id, &account.id).await.unwrap();

        let bus = Arc::new(BroadcastEventBus::new(64));
        let mut receiver = bus.subscribe_list(&list_id).await.unwrap();
        let service = create_service_with_bus(db, bus).await;

        let _status = service
            .create(
                "hello local list stream".to_string(),
                None,
                "public".to_string(),
                Some("en".to_string()),
                None,
                vec![],
            )
            .await
            .unwrap();

        let event = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event, StreamEvent::Update { .. }));
    }

    #[tokio::test]
    async fn create_unlisted_does_not_publish_to_public_stream() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let bus = Arc::new(BroadcastEventBus::new(64));
        let mut receiver = bus.subscribe_public().await.unwrap();
        let service = create_service_with_bus(db, bus).await;

        let _status = service
            .create(
                "hello #Rust".to_string(),
                None,
                "unlisted".to_string(),
                Some("en".to_string()),
                None,
                vec![],
            )
            .await
            .unwrap();

        let maybe_event = timeout(Duration::from_millis(200), receiver.recv()).await;
        assert!(
            maybe_event.is_err(),
            "unlisted status must not be delivered to public stream"
        );
    }

    #[tokio::test]
    async fn create_unlisted_does_not_publish_to_hashtag_stream() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let bus = Arc::new(BroadcastEventBus::new(64));
        let mut receiver = bus.subscribe_hashtag("rust").await.unwrap();
        let service = create_service_with_bus(db, bus).await;

        let _status = service
            .create(
                "hello #Rust".to_string(),
                None,
                "unlisted".to_string(),
                Some("en".to_string()),
                None,
                vec![],
            )
            .await
            .unwrap();

        let maybe_event = timeout(Duration::from_millis(200), receiver.recv()).await;
        assert!(
            maybe_event.is_err(),
            "unlisted status must not be delivered to hashtag stream"
        );
    }

    #[tokio::test]
    async fn create_public_with_cw_only_hashtag_does_not_publish_to_hashtag_stream() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let bus = Arc::new(BroadcastEventBus::new(64));
        let mut receiver = bus.subscribe_hashtag("rust").await.unwrap();
        let service = create_service_with_bus(db, bus).await;

        let _status = service
            .create(
                "hello world".to_string(),
                Some("#Rust".to_string()),
                "public".to_string(),
                Some("en".to_string()),
                None,
                vec![],
            )
            .await
            .unwrap();

        let maybe_event = timeout(Duration::from_millis(200), receiver.recv()).await;
        assert!(
            maybe_event.is_err(),
            "CW-only hashtag must not be delivered to hashtag stream"
        );
    }

    #[tokio::test]
    async fn remote_timeline_update_publishes_to_user_stream() {
        let (db, _temp_dir) = create_test_db().await;
        seed_account(db.as_ref(), "testuser").await;
        let bus = Arc::new(BroadcastEventBus::new(64));
        let mut receiver = bus.subscribe_user("testuser").await.unwrap();
        let service = create_service_with_bus(db.clone(), bus).await;

        let status = Status {
            id: EntityId::new_string(),
            uri: "https://remote.example/users/alice/statuses/timeline".to_string(),
            content: "<p>remote timeline update</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "alice@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Timeline,
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        };
        db.insert_status(&status).await.unwrap();

        service.update_loaded(&status).await.unwrap();

        let event = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event, StreamEvent::Update { .. }));
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
