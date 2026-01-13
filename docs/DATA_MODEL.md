# RustResort Data Model Design

## Overview

This document defines the data model used in RustResort, a single-user ActivityPub instance. The design is inspired by GoToSocial's `gtsmodel` package while leveraging Rust's type system for safety and correctness.

## Design Principles

1. **Type Safety**: Maximize use of Rust's type system
2. **Invariant Enforcement**: Constraints through newtype patterns
3. **NULL Safety**: Explicit nullability via `Option<T>`
4. **Serialization**: Full serde support
5. **SQLx Compatibility**: Direct mapping to SQLite schema

## Common Type Definitions

### ID Type (ULID)

All entities use ULID (Universally Unique Lexicographically Sortable Identifier) for IDs:

```rust
use ulid::Ulid;

/// Entity ID wrapper (ULID format, 26 characters)
/// Example: "01ARZ3NDEKTSV4RRFFQ69G5FAV"
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(String);

impl EntityId {
    /// Generate a new ULID
    pub fn new() -> Self {
        Self(Ulid::new().to_string())
    }
    
    /// Create from existing string
    pub fn from_string(s: String) -> Self {
        Self(s)
    }
    
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
```

### Timestamp Type

```rust
use chrono::{DateTime, Utc};

/// Timestamp type (UTC)
pub type Timestamp = DateTime<Utc>;
```

## Core Models

### Account (Single User)

RustResort is designed as a **single-user instance**. Only one local account exists in the database.

```rust
/// The single admin account for this instance
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Account {
    /// Database ID (ULID)
    pub id: String,
    
    /// Username (unique)
    pub username: String,
    
    /// Display name
    pub display_name: Option<String>,
    
    /// Profile bio/note
    pub note: Option<String>,
    
    /// Avatar S3 key (Cloudflare R2)
    pub avatar_s3_key: Option<String>,
    
    /// Header image S3 key
    pub header_s3_key: Option<String>,
    
    /// RSA private key (PEM format, 4096-bit)
    pub private_key_pem: String,
    
    /// RSA public key (PEM format, 4096-bit)
    pub public_key_pem: String,
    
    /// Creation timestamp
    pub created_at: String,
    
    /// Last update timestamp
    pub updated_at: String,
}
```

**Key Design Decisions:**
- **Single user only**: No multi-user support needed
- **S3-based media**: Avatar and header stored in Cloudflare R2
- **RSA 4096-bit keys**: For ActivityPub HTTP Signatures (see [RSA_KEY_SPEC.md](./RSA_KEY_SPEC.md))

### Status (Post/Toot)

Represents a post, which can be:
- User's own post (`is_local = true`)
- Remote post that the user interacted with (repost/favorite/bookmark)

```rust
/// A post/toot
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Status {
    /// Database ID (ULID)
    pub id: String,
    
    /// ActivityPub URI (unique)
    pub uri: String,
    
    /// HTML content
    pub content: String,
    
    /// Content warning (CW)
    pub content_warning: Option<String>,
    
    /// Visibility: "public", "unlisted", "private", "direct"
    pub visibility: String,
    
    /// Language code (BCP47)
    pub language: Option<String>,
    
    /// Author's account address (@user@domain)
    pub account_address: String,
    
    /// Is this a local post?
    pub is_local: bool,
    
    /// Reply to URI
    pub in_reply_to_uri: Option<String>,
    
    /// Boost of URI
    pub boost_of_uri: Option<String>,
    
    /// Reason for persisting: "own", "reposted", "favourited", "bookmarked", "reply_to_own"
    pub persisted_reason: String,
    
    /// Creation timestamp
    pub created_at: String,
    
    /// Fetch timestamp (for remote posts)
    pub fetched_at: Option<String>,
}
```

**Persisted Reason:**
Remote posts are only stored if there's a reason:
- `own`: User's own post
- `reposted`: User boosted this
- `favourited`: User favorited this
- `bookmarked`: User bookmarked this
- `reply_to_own`: Reply to user's post

### MediaAttachment

Media files are stored in **Cloudflare R2** (S3-compatible storage). The database only stores metadata and S3 keys.

```rust
/// Media file attached to a status
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MediaAttachment {
    /// Database ID (ULID)
    pub id: String,
    
    /// Status ID (optional, set when attached)
    pub status_id: Option<String>,
    
    /// S3 key for the media file
    pub s3_key: String,
    
    /// S3 key for thumbnail
    pub thumbnail_s3_key: Option<String>,
    
    /// MIME type
    pub content_type: String,
    
    /// File size in bytes
    pub file_size: i64,
    
    /// Alt text description
    pub description: Option<String>,
    
    /// Blurhash for preview
    pub blurhash: Option<String>,
    
    /// Image width (pixels)
    pub width: Option<i32>,
    
    /// Image height (pixels)
    pub height: Option<i32>,
    
    /// Creation timestamp
    pub created_at: String,
}
```

### Follow (Users We Follow)

```rust
/// A user this instance follows
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Follow {
    /// Database ID (ULID)
    pub id: String,
    
    /// Target account address (@user@domain)
    pub target_address: String,
    
    /// ActivityPub Follow activity URI
    pub uri: String,
    
    /// Creation timestamp
    pub created_at: String,
}
```

**Note:** Full profile data is cached in memory, not stored in the database.

### Follower (Users Following Us)

```rust
/// A user following this instance
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Follower {
    /// Database ID (ULID)
    pub id: String,
    
    /// Follower account address (@user@domain)
    pub follower_address: String,
    
    /// Follower's inbox URI (for activity delivery)
    pub inbox_uri: String,
    
    /// ActivityPub Follow activity URI
    pub uri: String,
    
    /// Creation timestamp
    pub created_at: String,
}
```

### Notification

```rust
/// Notification for user interactions
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Notification {
    /// Database ID (ULID)
    pub id: String,
    
    /// Type: "mention", "favourite", "reblog", "follow", "follow_request"
    pub notification_type: String,
    
    /// Origin account address
    pub origin_account_address: String,
    
    /// Related status URI (optional)
    pub status_uri: Option<String>,
    
    /// Read flag
    pub read: bool,
    
    /// Creation timestamp
    pub created_at: String,
}
```

### Favourite (Like)

```rust
/// Favourite (like) relationship
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Favourite {
    pub id: String,
    pub status_id: String,
    pub created_at: String,
}
```

### Bookmark

```rust
/// Bookmark relationship
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Bookmark {
    pub id: String,
    pub status_id: String,
    pub created_at: String,
}
```

### Repost (Boost)

```rust
/// Repost (boost) relationship
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Repost {
    pub id: String,
    pub status_id: String,
    pub uri: String,  // ActivityPub Announce activity URI
    pub created_at: String,
}
```

### DomainBlock

```rust
/// Blocked domain
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DomainBlock {
    pub id: String,
    pub domain: String,
    pub created_at: String,
}
```

## OAuth Models

### Application

```rust
/// OAuth application
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub redirect_uris: String,  // JSON array
    pub scopes: String,
    pub client_id: String,
    pub client_secret: String,
    pub website: Option<String>,
    pub created_at: String,
}
```

### Token

```rust
/// OAuth access token
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Token {
    pub id: String,
    pub access_token: String,
    pub token_type: String,  // "Bearer"
    pub scope: String,
    pub application_id: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}
```

## Additional Features

### Polls

```rust
/// Poll attached to a status
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Poll {
    pub id: String,
    pub status_id: String,
    pub expires_at: Option<String>,
    pub multiple: bool,
    pub voters_count: i32,
    pub created_at: String,
}

/// Poll option
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PollOption {
    pub id: String,
    pub poll_id: String,
    pub title: String,
    pub votes_count: i32,
}

/// User's vote on a poll
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PollVote {
    pub id: String,
    pub poll_id: String,
    pub option_id: String,
    pub created_at: String,
}
```

### Blocks and Mutes

```rust
/// Account block
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountBlock {
    pub id: String,
    pub target_address: String,
    pub created_at: String,
}

/// Account mute
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountMute {
    pub id: String,
    pub target_address: String,
    pub notifications: bool,  // Mute notifications too?
    pub expires_at: Option<String>,
    pub created_at: String,
}
```

### Lists

```rust
/// User-defined list
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct List {
    pub id: String,
    pub title: String,
    pub replies_policy: String,  // "followed", "list", "none"
    pub created_at: String,
}

/// Account in a list
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ListAccount {
    pub id: String,
    pub list_id: String,
    pub account_address: String,
    pub created_at: String,
}
```

### Filters

```rust
/// Content filter
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Filter {
    pub id: String,
    pub title: String,
    pub context: String,  // JSON array: ["home", "notifications", "public", "thread"]
    pub expires_at: Option<String>,
    pub filter_action: String,  // "warn", "hide"
    pub created_at: String,
}

/// Filter keyword
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FilterKeyword {
    pub id: String,
    pub filter_id: String,
    pub keyword: String,
    pub whole_word: bool,
}
```

### Hashtags

```rust
/// Hashtag
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Hashtag {
    pub id: String,
    pub name: String,
    pub url: String,
    pub created_at: String,
}

/// Status-Hashtag relationship
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StatusHashtag {
    pub status_id: String,
    pub hashtag_id: String,
}
```

## ER Diagram (Simplified)

```
┌──────────────┐
│   Account    │  (Single user)
└──────┬───────┘
       │
       ├─────────────┐
       │             │
┌──────┴───┐   ┌────┴─────┐
│  Status  │   │  Follow  │
└────┬─────┘   └──────────┘
     │
     ├──────────┬──────────┬──────────┐
     │          │          │          │
┌────┴────┐ ┌──┴───┐ ┌────┴────┐ ┌──┴──────┐
│  Media  │ │ Poll │ │Favourite│ │Bookmark │
└─────────┘ └──────┘ └─────────┘ └─────────┘
```

## Migration Strategy

1. **SQLite Only** (for single-user personal instances)
2. Use SQLx migration feature (`sqlx migrate`)
3. Table creation order respects dependencies
4. Leverage compile-time query verification (`sqlx::query!` macro)
5. Automatic backup to S3-compatible storage (see [BACKUP.md](./BACKUP.md))

### SQLite Benefits (for Personal Instances)

- **Single File**: Only `data/rustresort.db`
- **Zero Configuration**: No external DB server required
- **Portable**: Complete migration via file copy
- **Easy Backup**: File-level upload to S3
- **Lightweight**: Minimal memory usage

### Migration Files

```
migrations/
├── 001_initial.sql
├── 002_oauth.sql
├── 003_blocks_mutes_lists_filters.sql
├── 004_polls_scheduled_conversations.sql
└── 005_hashtags_fts.sql
```

## Index Design

Key indexes:

- `statuses.uri` (UNIQUE)
- `statuses.is_local`
- `statuses.created_at DESC`
- `statuses.account_address`
- `follows.target_address` (UNIQUE)
- `followers.follower_address` (UNIQUE)
- `notifications.created_at DESC`
- `notifications.read`

## Next Steps

- [API.md](./API.md) - API specification details
- [FEDERATION.md](./FEDERATION.md) - ActivityPub federation specification
- [RSA_KEY_SPEC.md](./RSA_KEY_SPEC.md) - RSA key specifications for HTTP Signatures
