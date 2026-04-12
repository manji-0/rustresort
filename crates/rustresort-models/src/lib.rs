//! Data models
//!
//! Rust structs representing database entities and cache items.
//! All models use ULID for IDs and chrono for timestamps.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// =============================================================================
// ID Types
// =============================================================================

/// Entity ID wrapper (ULID format, 26 characters)
///
/// Example: "01ARZ3NDEKTSV4RRFFQ69G5FAV"
#[derive(Debug, Clone, Copy, Default)]
pub struct EntityId;

impl EntityId {
    /// Generate a new ULID as a plain string.
    pub fn new_string() -> String {
        ulid::Ulid::new().to_string()
    }
}

// =============================================================================
// Account (Single user only)
// =============================================================================

/// The single admin account for this instance
///
/// Only one account exists in the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub note: Option<String>,
    /// S3 key for avatar image
    pub avatar_s3_key: Option<String>,
    /// S3 key for header image
    pub header_s3_key: Option<String>,
    /// RSA private key (PEM format)
    pub private_key_pem: String,
    /// RSA public key (PEM format)
    pub public_key_pem: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// =============================================================================
// Status
// =============================================================================

/// A post/toot
///
/// Can be:
/// - User's own post (is_local = true)
/// - Remote post that user interacted with (repost/fav/bookmark)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Status {
    pub id: String,
    /// ActivityPub URI (globally unique)
    pub uri: String,
    /// HTML content
    pub content: String,
    /// Content warning text
    pub content_warning: Option<String>,
    /// Visibility: public, unlisted, private, direct
    pub visibility: StatusVisibility,
    /// Language code (ISO 639-1)
    pub language: Option<String>,
    /// Account address for remote posts (user@domain), empty for local
    pub account_address: String,
    /// true if this is user's own post
    pub is_local: bool,
    /// URI of the post this replies to
    pub in_reply_to_uri: Option<String>,
    /// URI of the post this boosts
    pub boost_of_uri: Option<String>,
    /// Why this remote status was persisted
    /// Values: own, reposted, favourited, bookmarked, reply_to_own
    pub persisted_reason: PersistedReason,
    pub created_at: DateTime<Utc>,
    /// When this remote status was fetched
    pub fetched_at: Option<DateTime<Utc>>,
}

/// Status visibility
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum StatusVisibility {
    Public,
    Unlisted,
    Private,
    Direct,
}

impl StatusVisibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Unlisted => "unlisted",
            Self::Private => "private",
            Self::Direct => "direct",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "public" => Some(Self::Public),
            "unlisted" => Some(Self::Unlisted),
            "private" => Some(Self::Private),
            "direct" => Some(Self::Direct),
            _ => None,
        }
    }
}

impl std::fmt::Display for StatusVisibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Reason for persisting a remote status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PersistedReason {
    /// User's own post
    Own,
    /// User reposted (boosted) this
    Reposted,
    /// User favourited this
    Favourited,
    /// User bookmarked this
    Bookmarked,
    /// Reply to user's own post
    ReplyToOwn,
    /// Ephemeral cache-only status placeholder
    CacheOnly,
    /// Timeline fixture placeholder status
    Timeline,
}

impl PersistedReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Own => "own",
            Self::Reposted => "reposted",
            Self::Favourited => "favourited",
            Self::Bookmarked => "bookmarked",
            Self::ReplyToOwn => "reply_to_own",
            Self::CacheOnly => "cache_only",
            Self::Timeline => "timeline",
        }
    }
}

impl std::fmt::Display for PersistedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// =============================================================================
// Media Attachment
// =============================================================================

/// Media file attached to a status
///
/// Actual files are stored in Cloudflare R2.
/// This record holds metadata and S3 keys.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MediaAttachment {
    pub id: String,
    /// Associated status ID (null if not yet attached)
    pub status_id: Option<String>,
    /// S3 key for the media file
    pub s3_key: String,
    /// S3 key for thumbnail
    pub thumbnail_s3_key: Option<String>,
    /// MIME type (e.g., "image/webp")
    pub content_type: String,
    /// File size in bytes
    pub file_size: i64,
    /// Alt text description
    pub description: Option<String>,
    /// Blurhash for placeholder
    pub blurhash: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Focal point X coordinate between -1.0 and 1.0
    pub focus_x: Option<f64>,
    /// Focal point Y coordinate between -1.0 and 1.0
    pub focus_y: Option<f64>,
    pub created_at: DateTime<Utc>,
}

// =============================================================================
// Follow relationships
// =============================================================================

/// A user this instance follows
///
/// Only the address is stored, full profile is cached in memory.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Follow {
    pub id: String,
    /// Target address (user@domain format)
    pub target_address: String,
    /// Canonical ActivityPub actor URI when known
    pub actor_uri: Option<String>,
    /// ActivityPub Follow activity URI
    pub uri: String,
    pub created_at: DateTime<Utc>,
}

/// A user following this instance
///
/// Inbox URI is stored for activity delivery.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Follower {
    pub id: String,
    /// Follower address (user@domain format)
    pub follower_address: String,
    /// Canonical ActivityPub actor URI when known
    pub actor_uri: Option<String>,
    /// Follower's inbox URI for delivery
    pub inbox_uri: String,
    /// ActivityPub Follow activity URI
    pub uri: String,
    pub created_at: DateTime<Utc>,
}

// =============================================================================
// Notifications
// =============================================================================

/// Notification for user interactions
///
/// Persisted to database (not volatile).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Notification {
    pub id: String,
    /// Type: mention, favourite, reblog, follow, follow_request
    pub notification_type: NotificationType,
    /// Who triggered this notification (user@domain)
    pub origin_account_address: String,
    /// Related status URI (if applicable)
    pub status_uri: Option<String>,
    /// Whether user has seen this
    pub read: bool,
    pub created_at: DateTime<Utc>,
}

/// Notification types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum NotificationType {
    Mention,
    Favourite,
    Reblog,
    Follow,
    FollowRequest,
}

impl NotificationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mention => "mention",
            Self::Favourite => "favourite",
            Self::Reblog => "reblog",
            Self::Follow => "follow",
            Self::FollowRequest => "follow_request",
        }
    }
}

impl std::fmt::Display for NotificationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// =============================================================================
// Other entities
// =============================================================================

/// Favourite (like) relationship
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Favourite {
    pub id: String,
    pub status_id: String,
    pub created_at: DateTime<Utc>,
}

/// Bookmark relationship
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Bookmark {
    pub id: String,
    pub status_id: String,
    pub created_at: DateTime<Utc>,
}

/// Repost (boost) relationship
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Repost {
    pub id: String,
    pub status_id: String,
    /// Announce activity URI
    pub uri: String,
    pub created_at: DateTime<Utc>,
}

/// Blocked domain
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DomainBlock {
    pub id: String,
    pub domain: String,
    pub created_at: DateTime<Utc>,
}

/// Key-value settings
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Setting {
    pub key: String,
    pub value: String,
}

// =============================================================================
// OAuth Apps and Tokens
// =============================================================================

/// OAuth application registration
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OAuthApp {
    pub id: String,
    pub name: String,
    pub website: Option<String>,
    pub redirect_uri: String,
    pub client_id: String,
    pub client_secret: String,
    pub vapid_key: Option<String>,
    pub scopes: String,
    pub created_at: DateTime<Utc>,
}

/// OAuth access token
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OAuthToken {
    pub id: String,
    pub app_id: String,
    pub access_token: String,
    pub grant_type: String,
    pub scopes: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
}

/// OAuth authorization code (short-lived, single-use)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OAuthAuthorizationCode {
    pub id: String,
    pub app_id: String,
    pub code: String,
    pub redirect_uri: String,
    pub scopes: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}
