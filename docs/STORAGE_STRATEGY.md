# RustResort Data Persistence Strategy

## Overview

RustResort is designed as a **single-user instance** with the philosophy of "persisting only information from my perspective to the database." This strategy minimizes storage usage while maintaining full interoperability with the Fediverse.

## Design Philosophy

### Core Concept: "My Perspective" Storage

Traditional ActivityPub servers store all data received via federation in the database. While necessary for multi-user instances, this is excessive for single-user instances.

RustResort adopts the following principles:

```
┌─────────────────────────────────────────────────────────────┐
│                Data Persistence Principles                   │
├─────────────────────────────────────────────────────────────┤
│  ✓ Content I created → Persist to DB                        │
│  ✓ Content I acted on → Persist to DB                       │
│     (Repost, Fav, Bookmark)                                 │
│  ✓ Follow relationship addresses → Persist to DB            │
│  ✗ Others' timeline toots → Memory cache only (volatile)    │
│  ✗ Others' full profiles → Memory cache only (volatile)     │
└─────────────────────────────────────────────────────────────┘
```

### Single-User Premise

- Only an **admin user** exists on the instance
- No user registration/addition features implemented
- All operations are from a single user's perspective

## Data Classification and Storage Strategy

### 1. Persistent Data (DB Storage)

The following data is permanently stored in SQLite:

| Data Type | Description | Reason |
|-----------|-------------|--------|
| My Status | Posts I created | Core asset |
| Media Metadata | Reference info to files on S3 | Core asset (actual files on S3) |
| Reposted Status | Boosted content | My action |
| Favourited Status | Favourited content | My action |
| Bookmarked Status | Bookmarked content | My action |
| Follow Addresses | `user@domain` format | Relationship maintenance |
| **Notifications** | Mentions, Likes, Boosts, etc. | History retention |
| Domain Blocks | Blocked domains | Moderation settings |
| Instance Settings | Configuration values | Required for operation |

### 2. Volatile Data (Memory Cache)

The following data is kept only in memory and lost on restart:

| Data Type | Cache Size | Lifecycle |
|-----------|------------|-----------|
| Timeline toots | Latest 2000 | Auto-deleted via LRU |
| Followee/Follower profiles | All | Fetched at startup, updated via Federation |
| Remote actor public keys | LRU 1000 | Fetched during signature verification |

### 3. Object Storage (Cloudflare R2)

Media files are stored in Cloudflare R2 and served via Custom Domain:

| Data Type | Storage | Public URL Example |
|-----------|---------|-------------------|
| Avatar images | R2 | `https://media.example.com/avatars/{id}.webp` |
| Header images | R2 | `https://media.example.com/headers/{id}.webp` |
| Post attachments | R2 | `https://media.example.com/attachments/{id}.webp` |
| Thumbnails | R2 | `https://media.example.com/thumbnails/{id}.webp` |

**Media Delivery Flow:**
1. User uploads media → RustResort saves to R2
2. Media URL is `https://media.example.com/...` (R2 Custom Domain)
3. Client fetches directly from R2 via CDN (bypasses RustResort)

See [CLOUDFLARE.md](./CLOUDFLARE.md) for details.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Federation                               │
│                    (Incoming Activities)                         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Activity Router                             │
│         Create / Announce / Like / Follow / Update ...          │
└─────────────────────────────────────────────────────────────────┘
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
│  Persist Path   │ │   Cache Path    │ │  Ignore Path    │
│  (Related to me)│ │ (Reference only)│ │ (Not related)   │
└────────┬────────┘ └────────┬────────┘ └─────────────────┘
         │                   │
         ▼                   ▼
┌─────────────────┐ ┌─────────────────┐
│     SQLite      │ │  Memory Cache   │
│   (Permanent)   │ │   (Volatile)    │
└─────────────────┘ └─────────────────┘
```

## Detailed Design

### Timeline Cache

```rust
use moka::future::Cache;
use std::sync::Arc;

/// Timeline cache (max 2000 items, LRU)
pub struct TimelineCache {
    /// Status ID -> CachedStatus
    statuses: Cache<String, Arc<CachedStatus>>,
    /// Maximum items to keep
    max_items: usize,
}

/// Lightweight Status for caching
#[derive(Debug, Clone)]
pub struct CachedStatus {
    pub id: String,
    pub uri: String,
    pub content: String,
    pub account_address: String,  // user@domain
    pub created_at: DateTime<Utc>,
    pub visibility: Visibility,
    pub attachments: Vec<CachedAttachment>,
    pub reply_to_uri: Option<String>,
    pub boost_of_uri: Option<String>,
    // Note: Account details not included (separate cache reference)
}

impl TimelineCache {
    pub fn new(max_items: usize) -> Self {
        Self {
            statuses: Cache::builder()
                .max_capacity(max_items as u64)
                .time_to_live(Duration::from_secs(3600 * 24 * 7)) // 7 days
                .build(),
            max_items,
        }
    }
    
    /// Add to timeline (auto LRU deletion)
    pub async fn insert(&self, status: CachedStatus) {
        self.statuses.insert(status.id.clone(), Arc::new(status)).await;
    }
    
    /// Get home timeline
    pub async fn get_home_timeline(
        &self,
        followee_addresses: &HashSet<String>,
        limit: usize,
        max_id: Option<&str>,
    ) -> Vec<Arc<CachedStatus>> {
        // Filter and return only followee's statuses
        // ...
    }
}
```

### Profile Cache

```rust
/// Profile cache for followees/followers
pub struct ProfileCache {
    /// user@domain -> CachedProfile
    profiles: Cache<String, Arc<CachedProfile>>,
}

#[derive(Debug, Clone)]
pub struct CachedProfile {
    pub address: String,           // user@domain
    pub uri: String,               // ActivityPub URI
    pub display_name: Option<String>,
    pub note: Option<String>,
    pub avatar_url: Option<String>,
    pub header_url: Option<String>,
    pub public_key_pem: String,
    pub inbox_uri: String,
    pub outbox_uri: Option<String>,
    pub followers_count: Option<u64>,
    pub following_count: Option<u64>,
    pub fetched_at: DateTime<Utc>,
}

impl ProfileCache {
    /// Bulk fetch profiles from DB-stored follow relationships at startup
    pub async fn initialize_from_follows(
        &self,
        follow_addresses: Vec<String>,
        http_client: &HttpClient,
    ) {
        for address in follow_addresses {
            match self.fetch_profile(&address, http_client).await {
                Ok(profile) => {
                    self.profiles.insert(address, Arc::new(profile)).await;
                }
                Err(e) => {
                    tracing::warn!(%address, error = %e, "Failed to fetch profile at startup");
                }
            }
        }
    }
    
    /// Update from Federation Update Activity
    pub async fn update_from_activity(&self, actor: &ActivityActor) {
        let address = format!("{}@{}", actor.preferred_username, actor.domain);
        if let Some(existing) = self.profiles.get(&address).await {
            let updated = CachedProfile {
                display_name: actor.name.clone(),
                note: actor.summary.clone(),
                avatar_url: actor.icon_url(),
                header_url: actor.image_url(),
                fetched_at: Utc::now(),
                ..(*existing).clone()
            };
            self.profiles.insert(address, Arc::new(updated)).await;
        }
    }
}
```

### Persistence Decision Logic

```rust
/// Activity persistence decision
pub enum PersistenceDecision {
    /// Persist to DB
    Persist,
    /// Memory cache only
    CacheOnly,
    /// Don't save
    Ignore,
}

impl ActivityProcessor {
    /// Determine persistence for received Activity
    pub fn decide_persistence(&self, activity: &Activity) -> PersistenceDecision {
        match &activity.activity_type {
            // Mention to me → Save notification to DB + Cache original Status
            ActivityType::Create if self.mentions_me(&activity) => {
                // Notification is persisted to DB, Status itself is cached
                PersistenceDecision::Persist  // notification part
            }
            
            // Followee's post → Timeline cache only
            ActivityType::Create if self.is_followee(&activity.actor) => {
                PersistenceDecision::CacheOnly
            }
            
            // Someone follows me → Save follower address + notification to DB
            ActivityType::Follow if self.targets_me(&activity) => {
                PersistenceDecision::Persist
            }
            
            // Like on my post → Save notification to DB
            ActivityType::Like if self.is_my_status(&activity.object) => {
                PersistenceDecision::Persist  // persist as notification
            }
            
            // Boost of my post → Save notification to DB
            ActivityType::Announce if self.is_my_status(&activity.object) => {
                PersistenceDecision::Persist  // persist as notification
            }
            
            // Others → Ignore
            _ => PersistenceDecision::Ignore,
        }
    }
}
```

### User Action Persistence

```rust
/// Persist others' Status through user action
impl StatusService {
    /// Repost (boost)
    pub async fn repost(&self, status_uri: &str) -> Result<Status, Error> {
        // 1. Get Status from cache
        let cached = self.timeline_cache.get_by_uri(status_uri).await
            .ok_or(Error::NotFound)?;
        
        // 2. Persist others' Status to DB (if not already saved)
        let persisted = self.persist_remote_status(&cached).await?;
        
        // 3. Save Repost relationship to DB
        self.db.insert_repost(&self.my_account_id, &persisted.id).await?;
        
        // 4. Deliver Announce Activity
        self.federation.send_announce(&persisted).await?;
        
        Ok(persisted)
    }
    
    /// Favourite
    pub async fn favourite(&self, status_uri: &str) -> Result<(), Error> {
        let cached = self.timeline_cache.get_by_uri(status_uri).await
            .ok_or(Error::NotFound)?;
        
        // Persist others' Status to DB
        let persisted = self.persist_remote_status(&cached).await?;
        
        // Save Favourite relationship to DB
        self.db.insert_favourite(&self.my_account_id, &persisted.id).await?;
        
        // Deliver Like Activity
        self.federation.send_like(&persisted).await?;
        
        Ok(())
    }
    
    /// Bookmark (local only, no Federation)
    pub async fn bookmark(&self, status_uri: &str) -> Result<(), Error> {
        let cached = self.timeline_cache.get_by_uri(status_uri).await
            .ok_or(Error::NotFound)?;
        
        // Persist others' Status to DB
        let persisted = self.persist_remote_status(&cached).await?;
        
        // Save Bookmark relationship to DB
        self.db.insert_bookmark(&self.my_account_id, &persisted.id).await?;
        
        Ok(())
    }
    
    /// Persist cached Status to DB
    async fn persist_remote_status(&self, cached: &CachedStatus) -> Result<Status, Error> {
        // Return if already in DB
        if let Some(existing) = self.db.get_status_by_uri(&cached.uri).await? {
            return Ok(existing);
        }
        
        // Save new
        let status = Status {
            id: EntityId::new(),
            uri: cached.uri.clone(),
            content: cached.content.clone(),
            account_address: cached.account_address.clone(),
            // ...
            persisted_reason: PersistedReason::UserAction,
        };
        
        self.db.insert_status(&status).await?;
        Ok(status)
    }
}

/// Reason for Status persistence
#[derive(Debug, Clone, PartialEq)]
pub enum PersistedReason {
    /// I created it
    OwnContent,
    /// Repost target
    Reposted,
    /// Favourite target
    Favourited,
    /// Bookmark target
    Bookmarked,
    /// Reply to my post (for context retention)
    ReplyToOwn,
}
```

### Follow Relationship DB Design

```rust
/// Follow relationship (address only)
#[derive(Debug, Clone)]
pub struct Follow {
    pub id: EntityId,
    pub created_at: DateTime<Utc>,
    /// Target address (user@domain)
    pub target_address: String,
    /// ActivityPub URI (for Accept/Undo)
    pub uri: String,
}

/// Follower (address only)
#[derive(Debug, Clone)]
pub struct Follower {
    pub id: EntityId,
    pub created_at: DateTime<Utc>,
    /// Follower's address (user@domain)
    pub follower_address: String,
    /// ActivityPub URI
    pub uri: String,
}
```

SQL Migration:
```sql
-- Follow relationships (accounts I follow)
CREATE TABLE follows (
    id TEXT PRIMARY KEY,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    target_address TEXT NOT NULL UNIQUE,  -- user@domain
    uri TEXT NOT NULL UNIQUE
);

-- Followers (accounts following me)
CREATE TABLE followers (
    id TEXT PRIMARY KEY,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    follower_address TEXT NOT NULL UNIQUE,  -- user@domain
    inbox_uri TEXT NOT NULL,  -- delivery target (also in profile cache, kept here for delivery reliability)
    uri TEXT NOT NULL UNIQUE
);
```

### Startup Initialization Flow

```rust
impl AppState {
    pub async fn initialize() -> Result<Self, Error> {
        // 1. DB connection
        let db = Database::connect(&config.database_url).await?;
        
        // 2. Initialize caches
        let timeline_cache = TimelineCache::new(2000);
        let profile_cache = ProfileCache::new();
        
        // 3. Load follow relationships from DB
        let follow_addresses = db.get_all_follow_addresses().await?;
        let follower_addresses = db.get_all_follower_addresses().await?;
        
        // 4. Fetch followee/follower profiles in parallel
        let http_client = HttpClient::new();
        
        tokio::join!(
            profile_cache.initialize_from_addresses(&follow_addresses, &http_client),
            profile_cache.initialize_from_addresses(&follower_addresses, &http_client),
        );
        
        tracing::info!(
            follows = follow_addresses.len(),
            followers = follower_addresses.len(),
            "Initialized profile cache"
        );
        
        // 5. Timeline starts empty
        // → Populated in real-time via Federation,
        //   or optionally fetch latest from followee's Outbox
        
        Ok(Self {
            db: Arc::new(db),
            timeline_cache: Arc::new(timeline_cache),
            profile_cache: Arc::new(profile_cache),
            http_client: Arc::new(http_client),
            // ...
        })
    }
}
```

## DB Schema (Minimal Configuration)

```sql
-- My account info (single record only)
CREATE TABLE account (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT,
    note TEXT,
    avatar_s3_key TEXT,     -- S3 key
    header_s3_key TEXT,     -- S3 key
    private_key_pem TEXT NOT NULL,
    public_key_pem TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- My posts + persisted others' posts
CREATE TABLE statuses (
    id TEXT PRIMARY KEY,
    uri TEXT NOT NULL UNIQUE,
    content TEXT NOT NULL,
    content_warning TEXT,
    visibility TEXT NOT NULL,
    language TEXT,
    account_address TEXT NOT NULL,  -- empty string for my posts
    is_local INTEGER NOT NULL DEFAULT 0,
    in_reply_to_uri TEXT,
    boost_of_uri TEXT,
    persisted_reason TEXT NOT NULL,  -- own/reposted/favourited/bookmarked/reply_to_own
    created_at TIMESTAMP NOT NULL,
    fetched_at TIMESTAMP
);

-- Media attachments (S3 keys stored, actual files on S3)
CREATE TABLE media_attachments (
    id TEXT PRIMARY KEY,
    status_id TEXT,
    s3_key TEXT NOT NULL,           -- S3 object key
    thumbnail_s3_key TEXT,          -- Thumbnail S3 key
    content_type TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    description TEXT,
    blurhash TEXT,
    width INTEGER,
    height INTEGER,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (status_id) REFERENCES statuses(id)
);

-- Notifications (persisted)
CREATE TABLE notifications (
    id TEXT PRIMARY KEY,
    notification_type TEXT NOT NULL,  -- mention/favourite/reblog/follow/follow_request
    origin_account_address TEXT NOT NULL,  -- user@domain
    status_uri TEXT,                  -- Related Status URI (if any)
    read INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_notifications_created_at ON notifications(created_at DESC);
CREATE INDEX idx_notifications_read ON notifications(read);

-- Follow relationships
CREATE TABLE follows (
    id TEXT PRIMARY KEY,
    target_address TEXT NOT NULL UNIQUE,
    uri TEXT NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Followers
CREATE TABLE followers (
    id TEXT PRIMARY KEY,
    follower_address TEXT NOT NULL UNIQUE,
    inbox_uri TEXT NOT NULL,
    uri TEXT NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Favourites
CREATE TABLE favourites (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (status_id) REFERENCES statuses(id),
    UNIQUE (status_id)
);

-- Bookmarks
CREATE TABLE bookmarks (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (status_id) REFERENCES statuses(id),
    UNIQUE (status_id)
);

-- Repost relationships
CREATE TABLE reposts (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    uri TEXT NOT NULL UNIQUE,  -- Announce Activity URI
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (status_id) REFERENCES statuses(id),
    UNIQUE (status_id)
);

-- Domain blocks
CREATE TABLE domain_blocks (
    id TEXT PRIMARY KEY,
    domain TEXT NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Instance settings
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Indexes
CREATE INDEX idx_statuses_created_at ON statuses(created_at DESC);
CREATE INDEX idx_statuses_account_address ON statuses(account_address);
CREATE INDEX idx_statuses_persisted_reason ON statuses(persisted_reason);
```

## Benefits and Constraints

### Benefits

| Benefit | Description |
|---------|-------------|
| 💾 Storage Savings | Dramatically smaller DB size |
| ⚡ Fast Startup | Minimal DB reads |
| 🔒 Privacy | No retention of others' data |
| 🧹 No Maintenance | No auto-cleanup needed |
| 🎯 Simple | Reduced complexity with single-user focus |

### Constraints

| Constraint | Description | Mitigation |
|------------|-------------|------------|
| Timeline History | Lost on restart | Bookmark important items |
| Search | Cache only | Full-text search for own posts |
| Offline Startup | Empty timeline | Optional Outbox fetch at startup |
| S3 Required | S3 needed for media | Self-hosted MinIO also works |

## Configuration Options

```toml
[cache]
# Maximum timeline cache items
timeline_max_items = 2000

# Profile cache TTL (seconds)
profile_ttl = 86400  # 24 hours

[storage]
# S3-compatible storage (required)
endpoint = "https://s3.amazonaws.com"
bucket = "my-rustresort-media"
region = "ap-northeast-1"
# access_key and secret_key from environment variables

[startup]
# Fetch latest posts from followee's Outbox at startup
fetch_followee_outbox = true

# Maximum items to fetch from Outbox
outbox_fetch_limit = 50
```

## Next Steps

- [DATA_MODEL.md](./DATA_MODEL.md) - Detailed data model
- [FEDERATION.md](./FEDERATION.md) - Federation processing
