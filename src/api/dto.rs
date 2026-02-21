//! Mastodon API response DTOs
//!
//! Data Transfer Objects for Mastodon-compatible API responses.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Account response (Mastodon API compatible)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountResponse {
    pub id: String,
    pub username: String,
    pub acct: String,
    pub display_name: String,
    pub locked: bool,
    pub bot: bool,
    pub discoverable: bool,
    pub group: bool,
    pub created_at: DateTime<Utc>,
    pub note: String,
    pub url: String,
    pub avatar: String,
    pub avatar_static: String,
    pub header: String,
    pub header_static: String,
    pub followers_count: i32,
    pub following_count: i32,
    pub statuses_count: i32,
    pub last_status_at: Option<String>,
    pub emojis: Vec<serde_json::Value>,
    pub fields: Vec<serde_json::Value>,
}

/// Status response (Mastodon API compatible)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub in_reply_to_id: Option<String>,
    pub in_reply_to_account_id: Option<String>,
    pub sensitive: bool,
    pub spoiler_text: String,
    pub visibility: String,
    pub language: Option<String>,
    pub uri: String,
    pub url: String,
    pub replies_count: i32,
    pub reblogs_count: i32,
    pub favourites_count: i32,
    pub edited_at: Option<DateTime<Utc>>,
    pub content: String,
    pub reblog: Option<Box<StatusResponse>>,
    pub account: AccountResponse,
    pub media_attachments: Vec<MediaAttachmentResponse>,
    pub mentions: Vec<serde_json::Value>,
    pub tags: Vec<serde_json::Value>,
    pub emojis: Vec<serde_json::Value>,
    pub card: Option<serde_json::Value>,
    pub poll: Option<serde_json::Value>,
    pub favourited: Option<bool>,
    pub reblogged: Option<bool>,
    pub muted: Option<bool>,
    pub bookmarked: Option<bool>,
    pub pinned: Option<bool>,
}

/// Media attachment response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAttachmentResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub media_type: String,
    pub url: String,
    pub preview_url: String,
    pub remote_url: Option<String>,
    pub text_url: Option<String>,
    pub meta: Option<serde_json::Value>,
    pub description: Option<String>,
    pub blurhash: Option<String>,
}

/// Context response (ancestors and descendants)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextResponse {
    pub ancestors: Vec<StatusResponse>,
    pub descendants: Vec<StatusResponse>,
}

/// Relationship response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipResponse {
    pub id: String,
    pub following: bool,
    pub showing_reblogs: bool,
    pub notifying: bool,
    pub followed_by: bool,
    pub blocking: bool,
    pub blocked_by: bool,
    pub muting: bool,
    pub muting_notifications: bool,
    pub requested: bool,
    pub domain_blocking: bool,
    pub endorsed: bool,
    pub note: String,
}

/// Notification response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub notification_type: String,
    pub created_at: DateTime<Utc>,
    pub account: AccountResponse,
    pub status: Option<StatusResponse>,
}

/// Instance response (Mastodon API compatible)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceResponse {
    pub uri: String,
    pub title: String,
    pub short_description: String,
    pub description: String,
    pub email: String,
    pub version: String,
    pub languages: Vec<String>,
    pub registrations: bool,
    pub approval_required: bool,
    pub invites_enabled: bool,
    pub configuration: InstanceConfiguration,
    pub urls: InstanceUrls,
    pub stats: InstanceStats,
    pub thumbnail: Option<String>,
    pub contact_account: Option<AccountResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfiguration {
    pub statuses: StatusesConfiguration,
    pub media_attachments: MediaConfiguration,
    pub polls: PollsConfiguration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusesConfiguration {
    pub max_characters: i32,
    pub max_media_attachments: i32,
    pub characters_reserved_per_url: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfiguration {
    pub supported_mime_types: Vec<String>,
    pub image_size_limit: i64,
    pub image_matrix_limit: i64,
    pub video_size_limit: i64,
    pub video_frame_rate_limit: i32,
    pub video_matrix_limit: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollsConfiguration {
    pub max_options: i32,
    pub max_characters_per_option: i32,
    pub min_expiration: i32,
    pub max_expiration: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceUrls {
    pub streaming_api: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceStats {
    pub user_count: i64,
    pub status_count: i64,
    pub domain_count: i64,
}
