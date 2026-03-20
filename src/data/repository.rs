//! Repository traits for data access.
//!
//! These traits decouple service-layer logic from the concrete `Database`
//! implementation and make mocking simpler in unit tests.

use axum::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashSet;

use crate::error::AppError;

use super::{Account, Database, MediaAttachment, Status};

#[derive(Debug, Clone)]
pub struct AccountCredentialsPatch {
    pub account_id: String,
    pub expected_current_avatar_s3_key: Option<String>,
    pub expected_current_header_s3_key: Option<String>,
    pub avatar_s3_key: Option<String>,
    pub header_s3_key: Option<String>,
    pub display_name: Option<Option<String>>,
    pub note: Option<Option<String>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ScheduledStatusInsert {
    pub scheduled_at: String,
    pub status_text: String,
    pub visibility: String,
    pub content_warning: Option<String>,
    pub in_reply_to_id: Option<String>,
    pub media_ids: Option<String>,
    pub poll_options: Option<String>,
    pub poll_expires_in: Option<i64>,
    pub poll_multiple: bool,
}

#[derive(Debug, Clone)]
pub struct ListTimelineQuery {
    pub list_id: String,
    pub local_account_address: String,
    pub local_account_id: String,
    pub default_port: Option<u16>,
    pub limit: usize,
    pub max_id: Option<String>,
    pub min_id: Option<String>,
}

/// Data access for account domain operations.
#[async_trait]
pub trait AccountRepository: Send + Sync {
    async fn get_account(&self) -> Result<Option<Account>, AppError>;
    async fn get_follower_inboxes(&self) -> Result<Vec<String>, AppError>;
    async fn insert_account_if_empty(&self, account: &Account) -> Result<bool, AppError>;
    async fn patch_account_profile(
        &self,
        account_id: &str,
        display_name: Option<Option<&str>>,
        note: Option<Option<&str>>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError>;
    async fn patch_account_credentials_if_matches(
        &self,
        patch: &AccountCredentialsPatch,
    ) -> Result<bool, AppError>;
    async fn update_account_avatar_key_if_matches(
        &self,
        account_id: &str,
        expected_current_avatar_s3_key: Option<&str>,
        avatar_s3_key: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError>;
    async fn update_account_header_key_if_matches(
        &self,
        account_id: &str,
        expected_current_header_s3_key: Option<&str>,
        header_s3_key: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError>;
}

/// Data access used by `StatusService`.
#[async_trait]
pub trait StatusRepository: Send + Sync {
    async fn insert_status_with_media_and_poll(
        &self,
        status: &Status,
        media_ids: &[String],
        poll: Option<(&[String], i64, bool)>,
    ) -> Result<(), AppError>;
    async fn get_status(&self, id: &str) -> Result<Option<Status>, AppError>;
    async fn get_status_by_uri(&self, uri: &str) -> Result<Option<Status>, AppError>;
    async fn update_status(&self, status: &Status) -> Result<(), AppError>;
    async fn update_status_with_edit_snapshot(
        &self,
        previous: &Status,
        updated: &Status,
    ) -> Result<(), AppError>;
    async fn update_status_with_edit_snapshot_and_media(
        &self,
        previous: &Status,
        updated: &Status,
        media_ids: Option<&[String]>,
    ) -> Result<(), AppError>;
    async fn get_media_by_status(&self, status_id: &str) -> Result<Vec<MediaAttachment>, AppError>;
    async fn replace_status_media(
        &self,
        status_id: &str,
        media_ids: &[String],
    ) -> Result<(), AppError>;
    async fn get_poll_by_status_id(
        &self,
        status_id: &str,
    ) -> Result<Option<(String, String, bool, bool, i64, i64)>, AppError>;
    async fn get_poll_options(&self, poll_id: &str)
    -> Result<Vec<(String, String, i64)>, AppError>;
    async fn get_favourite_id(&self, status_id: &str) -> Result<Option<String>, AppError>;
    async fn get_repost_uri(&self, status_id: &str) -> Result<Option<String>, AppError>;
    async fn get_status_replies(&self, in_reply_to_uri: &str) -> Result<Vec<Status>, AppError>;
    async fn get_status_replies_limited(
        &self,
        in_reply_to_uri: &str,
        limit: usize,
    ) -> Result<Vec<Status>, AppError>;
    async fn insert_status_edit(
        &self,
        status_id: &str,
        content: &str,
        content_warning: Option<&str>,
    ) -> Result<String, AppError>;
    async fn get_status_edits(
        &self,
        status_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, String, Option<String>, DateTime<Utc>)>, AppError>;
    async fn get_idempotency_response(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<Option<serde_json::Value>, AppError>;
    async fn reserve_idempotency_key(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<bool, AppError>;
    async fn store_idempotency_response(
        &self,
        endpoint: &str,
        idempotency_key: &str,
        response: &serde_json::Value,
    ) -> Result<(), AppError>;
    async fn clear_pending_idempotency_key(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<(), AppError>;
    async fn create_scheduled_status(
        &self,
        request: &ScheduledStatusInsert,
    ) -> Result<String, AppError>;
    async fn get_scheduled_status(&self, id: &str) -> Result<Option<serde_json::Value>, AppError>;
    async fn delete_status(&self, id: &str) -> Result<(), AppError>;
    async fn insert_favourite(&self, status_id: &str) -> Result<String, AppError>;
    async fn insert_bookmark(&self, status_id: &str) -> Result<String, AppError>;
    async fn insert_media(&self, media: &MediaAttachment) -> Result<(), AppError>;
    async fn insert_status(&self, status: &Status) -> Result<(), AppError>;
    async fn insert_repost(&self, status_id: &str, uri: &str) -> Result<String, AppError>;
    async fn delete_favourite(&self, status_id: &str) -> Result<(), AppError>;
    async fn delete_bookmark(&self, status_id: &str) -> Result<(), AppError>;
    async fn delete_repost(&self, status_id: &str) -> Result<(), AppError>;
    async fn insert_status_pin(&self, status_id: &str) -> Result<(), AppError>;
    async fn delete_status_pin(&self, status_id: &str) -> Result<(), AppError>;
    async fn resolve_thread_root_uri(&self, status: &Status) -> Result<String, AppError>;
    async fn insert_muted_thread(&self, thread_uri: &str) -> Result<(), AppError>;
    async fn delete_muted_thread(&self, thread_uri: &str) -> Result<(), AppError>;
    async fn is_favourited(&self, status_id: &str) -> Result<bool, AppError>;
    async fn is_bookmarked(&self, status_id: &str) -> Result<bool, AppError>;
    async fn is_reposted(&self, status_id: &str) -> Result<bool, AppError>;
    async fn is_thread_muted(&self, thread_uri: &str) -> Result<bool, AppError>;
    async fn is_status_pinned(&self, status_id: &str) -> Result<bool, AppError>;
    async fn get_account(&self) -> Result<Option<Account>, AppError>;
    async fn get_list_ids_for_account(
        &self,
        account_address: &str,
        default_port: Option<u16>,
    ) -> Result<Vec<String>, AppError>;
}

/// Data access used by `TimelineService`.
#[async_trait]
pub trait TimelineRepository: Send + Sync {
    async fn get_local_statuses_in_window(
        &self,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError>;
    async fn get_local_public_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError>;
    async fn get_statuses_by_hashtag_in_window(
        &self,
        hashtag: &str,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError>;
    async fn get_list_timeline_statuses_in_window(
        &self,
        query: &ListTimelineQuery,
    ) -> Result<Vec<Status>, AppError>;
    async fn get_favourited_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError>;
    async fn get_bookmarked_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError>;
    async fn get_bookmarked_status_ids_batch(
        &self,
        status_ids: &[String],
    ) -> Result<HashSet<String>, AppError>;
    async fn get_favourited_status_ids_batch(
        &self,
        status_ids: &[String],
    ) -> Result<HashSet<String>, AppError>;
    async fn get_muted_thread_uris(&self) -> Result<HashSet<String>, AppError>;
    async fn resolve_thread_root_uri(&self, status: &Status) -> Result<String, AppError>;
}

#[async_trait]
impl AccountRepository for Database {
    async fn get_account(&self) -> Result<Option<Account>, AppError> {
        Database::get_account(self).await
    }

    async fn get_follower_inboxes(&self) -> Result<Vec<String>, AppError> {
        Database::get_follower_inboxes(self).await
    }

    async fn insert_account_if_empty(&self, account: &Account) -> Result<bool, AppError> {
        Database::insert_account_if_empty(self, account).await
    }

    async fn patch_account_profile(
        &self,
        account_id: &str,
        display_name: Option<Option<&str>>,
        note: Option<Option<&str>>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError> {
        Database::patch_account_profile(self, account_id, display_name, note, updated_at).await
    }

    async fn patch_account_credentials_if_matches(
        &self,
        patch: &AccountCredentialsPatch,
    ) -> Result<bool, AppError> {
        Database::patch_account_credentials_if_matches(self, patch).await
    }

    async fn update_account_avatar_key_if_matches(
        &self,
        account_id: &str,
        expected_current_avatar_s3_key: Option<&str>,
        avatar_s3_key: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError> {
        Database::update_account_avatar_key_if_matches(
            self,
            account_id,
            expected_current_avatar_s3_key,
            avatar_s3_key,
            updated_at,
        )
        .await
    }

    async fn update_account_header_key_if_matches(
        &self,
        account_id: &str,
        expected_current_header_s3_key: Option<&str>,
        header_s3_key: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool, AppError> {
        Database::update_account_header_key_if_matches(
            self,
            account_id,
            expected_current_header_s3_key,
            header_s3_key,
            updated_at,
        )
        .await
    }
}

#[async_trait]
impl StatusRepository for Database {
    async fn insert_status_with_media_and_poll(
        &self,
        status: &Status,
        media_ids: &[String],
        poll: Option<(&[String], i64, bool)>,
    ) -> Result<(), AppError> {
        Database::insert_status_with_media_and_poll(self, status, media_ids, poll).await
    }

    async fn get_status(&self, id: &str) -> Result<Option<Status>, AppError> {
        Database::get_status(self, id).await
    }

    async fn get_status_by_uri(&self, uri: &str) -> Result<Option<Status>, AppError> {
        Database::get_status_by_uri(self, uri).await
    }

    async fn update_status(&self, status: &Status) -> Result<(), AppError> {
        Database::update_status(self, status).await
    }

    async fn update_status_with_edit_snapshot(
        &self,
        previous: &Status,
        updated: &Status,
    ) -> Result<(), AppError> {
        Database::update_status_with_edit_snapshot(self, previous, updated).await
    }

    async fn update_status_with_edit_snapshot_and_media(
        &self,
        previous: &Status,
        updated: &Status,
        media_ids: Option<&[String]>,
    ) -> Result<(), AppError> {
        Database::update_status_with_edit_snapshot_and_media(self, previous, updated, media_ids)
            .await
    }

    async fn get_media_by_status(&self, status_id: &str) -> Result<Vec<MediaAttachment>, AppError> {
        Database::get_media_by_status(self, status_id).await
    }

    async fn replace_status_media(
        &self,
        status_id: &str,
        media_ids: &[String],
    ) -> Result<(), AppError> {
        Database::replace_status_media(self, status_id, media_ids).await
    }

    async fn get_poll_by_status_id(
        &self,
        status_id: &str,
    ) -> Result<Option<(String, String, bool, bool, i64, i64)>, AppError> {
        Database::get_poll_by_status_id(self, status_id).await
    }

    async fn get_poll_options(
        &self,
        poll_id: &str,
    ) -> Result<Vec<(String, String, i64)>, AppError> {
        Database::get_poll_options(self, poll_id).await
    }

    async fn get_favourite_id(&self, status_id: &str) -> Result<Option<String>, AppError> {
        Database::get_favourite_id(self, status_id).await
    }

    async fn get_repost_uri(&self, status_id: &str) -> Result<Option<String>, AppError> {
        Database::get_repost_uri(self, status_id).await
    }

    async fn get_status_replies(&self, in_reply_to_uri: &str) -> Result<Vec<Status>, AppError> {
        Database::get_status_replies(self, in_reply_to_uri).await
    }

    async fn get_status_replies_limited(
        &self,
        in_reply_to_uri: &str,
        limit: usize,
    ) -> Result<Vec<Status>, AppError> {
        Database::get_status_replies_limited(self, in_reply_to_uri, limit).await
    }

    async fn insert_status_edit(
        &self,
        status_id: &str,
        content: &str,
        content_warning: Option<&str>,
    ) -> Result<String, AppError> {
        Database::insert_status_edit(self, status_id, content, content_warning).await
    }

    async fn get_status_edits(
        &self,
        status_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, String, Option<String>, DateTime<Utc>)>, AppError> {
        Database::get_status_edits(self, status_id, limit).await
    }

    async fn get_idempotency_response(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<Option<serde_json::Value>, AppError> {
        Database::get_idempotency_response(self, endpoint, idempotency_key).await
    }

    async fn reserve_idempotency_key(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<bool, AppError> {
        Database::reserve_idempotency_key(self, endpoint, idempotency_key).await
    }

    async fn store_idempotency_response(
        &self,
        endpoint: &str,
        idempotency_key: &str,
        response: &serde_json::Value,
    ) -> Result<(), AppError> {
        Database::store_idempotency_response(self, endpoint, idempotency_key, response).await
    }

    async fn clear_pending_idempotency_key(
        &self,
        endpoint: &str,
        idempotency_key: &str,
    ) -> Result<(), AppError> {
        Database::clear_pending_idempotency_key(self, endpoint, idempotency_key).await
    }

    async fn create_scheduled_status(
        &self,
        request: &ScheduledStatusInsert,
    ) -> Result<String, AppError> {
        Database::create_scheduled_status(self, request).await
    }

    async fn get_scheduled_status(&self, id: &str) -> Result<Option<serde_json::Value>, AppError> {
        Database::get_scheduled_status(self, id).await
    }

    async fn delete_status(&self, id: &str) -> Result<(), AppError> {
        Database::delete_status(self, id).await
    }

    async fn insert_favourite(&self, status_id: &str) -> Result<String, AppError> {
        Database::insert_favourite(self, status_id).await
    }

    async fn insert_bookmark(&self, status_id: &str) -> Result<String, AppError> {
        Database::insert_bookmark(self, status_id).await
    }

    async fn insert_media(&self, media: &MediaAttachment) -> Result<(), AppError> {
        Database::insert_media(self, media).await
    }

    async fn insert_status(&self, status: &Status) -> Result<(), AppError> {
        Database::insert_status(self, status).await
    }

    async fn insert_repost(&self, status_id: &str, uri: &str) -> Result<String, AppError> {
        Database::insert_repost(self, status_id, uri).await
    }

    async fn delete_favourite(&self, status_id: &str) -> Result<(), AppError> {
        Database::delete_favourite(self, status_id).await
    }

    async fn delete_bookmark(&self, status_id: &str) -> Result<(), AppError> {
        Database::delete_bookmark(self, status_id).await
    }

    async fn delete_repost(&self, status_id: &str) -> Result<(), AppError> {
        Database::delete_repost(self, status_id).await
    }

    async fn insert_status_pin(&self, status_id: &str) -> Result<(), AppError> {
        Database::insert_status_pin(self, status_id).await
    }

    async fn delete_status_pin(&self, status_id: &str) -> Result<(), AppError> {
        Database::delete_status_pin(self, status_id).await
    }

    async fn resolve_thread_root_uri(&self, status: &Status) -> Result<String, AppError> {
        Database::resolve_thread_root_uri(self, status).await
    }

    async fn insert_muted_thread(&self, thread_uri: &str) -> Result<(), AppError> {
        Database::insert_muted_thread(self, thread_uri).await
    }

    async fn delete_muted_thread(&self, thread_uri: &str) -> Result<(), AppError> {
        Database::delete_muted_thread(self, thread_uri).await
    }

    async fn is_favourited(&self, status_id: &str) -> Result<bool, AppError> {
        Database::is_favourited(self, status_id).await
    }

    async fn is_bookmarked(&self, status_id: &str) -> Result<bool, AppError> {
        Database::is_bookmarked(self, status_id).await
    }

    async fn is_reposted(&self, status_id: &str) -> Result<bool, AppError> {
        Database::is_reposted(self, status_id).await
    }

    async fn is_thread_muted(&self, thread_uri: &str) -> Result<bool, AppError> {
        Database::is_thread_muted(self, thread_uri).await
    }

    async fn is_status_pinned(&self, status_id: &str) -> Result<bool, AppError> {
        Database::is_status_pinned(self, status_id).await
    }

    async fn get_account(&self) -> Result<Option<Account>, AppError> {
        Database::get_account(self).await
    }

    async fn get_list_ids_for_account(
        &self,
        account_address: &str,
        default_port: Option<u16>,
    ) -> Result<Vec<String>, AppError> {
        Database::get_list_ids_for_account(self, account_address, default_port).await
    }
}

#[async_trait]
impl TimelineRepository for Database {
    async fn get_local_statuses_in_window(
        &self,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        Database::get_local_statuses_in_window(self, limit, max_id, min_id).await
    }

    async fn get_local_public_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        Database::get_local_public_statuses(self, limit, max_id).await
    }

    async fn get_statuses_by_hashtag_in_window(
        &self,
        hashtag: &str,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        Database::get_statuses_by_hashtag_in_window(self, hashtag, limit, max_id, min_id).await
    }

    async fn get_list_timeline_statuses_in_window(
        &self,
        query: &ListTimelineQuery,
    ) -> Result<Vec<Status>, AppError> {
        Database::get_list_timeline_statuses_in_window(self, query).await
    }

    async fn get_favourited_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        Database::get_favourited_statuses(self, limit, max_id).await
    }

    async fn get_bookmarked_statuses(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        Database::get_bookmarked_statuses(self, limit, max_id).await
    }

    async fn get_bookmarked_status_ids_batch(
        &self,
        status_ids: &[String],
    ) -> Result<HashSet<String>, AppError> {
        Database::get_bookmarked_status_ids_batch(self, status_ids).await
    }

    async fn get_favourited_status_ids_batch(
        &self,
        status_ids: &[String],
    ) -> Result<HashSet<String>, AppError> {
        Database::get_favourited_status_ids_batch(self, status_ids).await
    }

    async fn get_muted_thread_uris(&self) -> Result<HashSet<String>, AppError> {
        Database::get_muted_thread_uris(self).await
    }

    async fn resolve_thread_root_uri(&self, status: &Status) -> Result<String, AppError> {
        Database::resolve_thread_root_uri(self, status).await
    }
}
