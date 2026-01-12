# RustResort - Future Implementation Roadmap

**Created**: 2026-01-11 22:19  
**Purpose**: Define interfaces and TODOs for next development session  
**Status**: Planning Document

---

## 📋 Table of Contents

1. [Phase 3 Completion](#phase-3-completion)
2. [Phase 4: Advanced Features](#phase-4-advanced-features)
3. [ActivityPub Federation](#activitypub-federation)
4. [Background Jobs & Automation](#background-jobs--automation)
5. [Performance & Optimization](#performance--optimization)
6. [Testing & Quality](#testing--quality)

---

## Phase 3 Completion

### 1. Search API - Full Implementation

**Current Status**: 50% complete (basic account search only)  
**Remaining Work**: Full-text search, hashtag integration

#### Interface: Full-Text Status Search

```rust
// src/data/database.rs
impl Database {
    /// Search statuses by content
    /// 
    /// TODO: Implement full-text search using SQLite FTS5
    /// - Create FTS5 virtual table for status content
    /// - Index status content and content_warning
    /// - Support phrase queries and boolean operators
    /// - Rank results by relevance
    pub async fn search_statuses(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Status>, AppError> {
        todo!("Implement FTS5 full-text search for statuses")
    }
}
```

#### Interface: Hashtag Search

```rust
// src/data/database.rs
impl Database {
    /// Search hashtags
    /// 
    /// TODO: Implement hashtag search and tracking
    /// - Create hashtags table
    /// - Extract hashtags from status content
    /// - Track usage count and trending
    /// - Support autocomplete
    pub async fn search_hashtags(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, i64)>, AppError> {
        todo!("Implement hashtag search with usage counts")
    }
    
    /// Get trending hashtags
    /// 
    /// TODO: Calculate trending hashtags
    /// - Track hashtag usage over time
    /// - Calculate trend score
    /// - Return top N trending tags
    pub async fn get_trending_hashtags(
        &self,
        limit: usize,
        time_window_hours: i64,
    ) -> Result<Vec<(String, i64, f64)>, AppError> {
        todo!("Implement trending hashtag calculation")
    }
}
```

#### Migration: Search Tables

```sql
-- migrations/005_search_features.sql

-- Full-text search virtual table
CREATE VIRTUAL TABLE IF NOT EXISTS statuses_fts USING fts5(
    status_id UNINDEXED,
    content,
    content_warning,
    tokenize = 'porter unicode61'
);

-- Hashtags table
CREATE TABLE IF NOT EXISTS hashtags (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    usage_count INTEGER NOT NULL DEFAULT 0,
    last_used_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_hashtags_usage ON hashtags(usage_count DESC);
CREATE INDEX IF NOT EXISTS idx_hashtags_last_used ON hashtags(last_used_at DESC);

-- Status hashtags junction table
CREATE TABLE IF NOT EXISTS status_hashtags (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    hashtag_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE,
    FOREIGN KEY (hashtag_id) REFERENCES hashtags(id) ON DELETE CASCADE,
    UNIQUE(status_id, hashtag_id)
);

CREATE INDEX IF NOT EXISTS idx_status_hashtags_status ON status_hashtags(status_id);
CREATE INDEX IF NOT EXISTS idx_status_hashtags_hashtag ON status_hashtags(hashtag_id);
```

---

## Phase 4: Advanced Features

### 1. Trends API

**Status**: Not started  
**Priority**: Medium

#### Interface: Trends Service

```rust
// src/service/trends.rs

#![allow(dead_code)]

use crate::data::Database;
use crate::error::AppError;
use std::sync::Arc;

/// Trends calculation service
pub struct TrendsService {
    db: Arc<Database>,
}

impl TrendsService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
    
    /// Get trending statuses
    /// 
    /// TODO: Implement trending status calculation
    /// - Track favorites, boosts, replies over time
    /// - Calculate engagement score
    /// - Apply time decay
    /// - Filter by language/instance
    pub async fn get_trending_statuses(
        &self,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, AppError> {
        todo!("Calculate and return trending statuses")
    }
    
    /// Get trending links
    /// 
    /// TODO: Implement trending link tracking
    /// - Extract URLs from statuses
    /// - Track share count
    /// - Fetch preview metadata
    /// - Calculate trend score
    pub async fn get_trending_links(
        &self,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, AppError> {
        todo!("Calculate and return trending links")
    }
    
    /// Update trends cache
    /// 
    /// TODO: Background job to update trends
    /// - Run periodically (e.g., every 15 minutes)
    /// - Calculate all trend types
    /// - Update cache
    pub async fn update_trends(&self) -> Result<(), AppError> {
        todo!("Update all trends calculations")
    }
}
```

#### API Endpoints

```rust
// src/api/mastodon/trends.rs

/// GET /api/v1/trends/statuses
pub async fn trending_statuses(
    State(state): State<AppState>,
    Query(params): Query<TrendsParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    todo!("Return trending statuses")
}

/// GET /api/v1/trends/tags
pub async fn trending_tags(
    State(state): State<AppState>,
    Query(params): Query<TrendsParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    todo!("Return trending hashtags")
}

/// GET /api/v1/trends/links
pub async fn trending_links(
    State(state): State<AppState>,
    Query(params): Query<TrendsParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    todo!("Return trending links")
}
```

### 2. Streaming API

**Status**: Not started  
**Priority**: High (for real-time updates)

#### Interface: Streaming Service

```rust
// src/api/mastodon/streaming.rs

use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::response::Response;

/// WebSocket streaming endpoint
/// 
/// TODO: Implement WebSocket streaming
/// - Handle WebSocket upgrade
/// - Authenticate user
/// - Subscribe to event streams
/// - Send real-time updates
/// - Handle disconnections
pub async fn streaming(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    CurrentUser(session): CurrentUser,
) -> Response {
    todo!("Implement WebSocket streaming")
}

/// Handle WebSocket connection
async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    user_id: String,
) {
    todo!("Handle WebSocket messages and events")
}

/// Stream types
#[derive(Debug, Clone)]
pub enum StreamType {
    /// User's home timeline
    User,
    /// Public timeline
    Public,
    /// Local public timeline
    PublicLocal,
    /// Hashtag timeline
    Hashtag(String),
    /// List timeline
    List(String),
    /// Direct messages
    Direct,
}
```

### 3. Push Notifications

**Status**: Not started  
**Priority**: Medium

#### Interface: Push Service

```rust
// src/service/push.rs

#![allow(dead_code)]

use crate::error::AppError;

/// Web Push notification service
pub struct PushService {
    vapid_private_key: String,
    vapid_public_key: String,
}

impl PushService {
    /// Create new push service
    /// 
    /// TODO: Initialize Web Push service
    /// - Load VAPID keys
    /// - Set up push client
    pub fn new(vapid_private_key: String, vapid_public_key: String) -> Self {
        todo!("Initialize push service")
    }
    
    /// Send push notification
    /// 
    /// TODO: Send Web Push notification
    /// - Build notification payload
    /// - Sign with VAPID
    /// - Send to push endpoint
    /// - Handle errors and retries
    pub async fn send_notification(
        &self,
        subscription: &PushSubscription,
        notification: &Notification,
    ) -> Result<(), AppError> {
        todo!("Send Web Push notification")
    }
}

#[derive(Debug, Clone)]
pub struct PushSubscription {
    pub endpoint: String,
    pub p256dh_key: String,
    pub auth_secret: String,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub icon: Option<String>,
    pub badge: Option<String>,
}
```

---

## ActivityPub Federation

### 1. Complete Activity Processing

**Current Status**: Interfaces defined, not implemented  
**Priority**: High (for federation)

#### Implement Activity Handlers

```rust
// src/federation/activity.rs

impl ActivityProcessor {
    /// Process an incoming activity
    /// 
    /// TODO: Complete activity processing pipeline
    /// 1. Parse activity type from JSON
    /// 2. Verify signature (already done by middleware)
    /// 3. Check if domain is blocked
    /// 4. Dispatch to type-specific handler
    /// 5. Update caches and database
    /// 6. Create notifications if needed
    pub async fn process(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        // Parse activity type
        let activity_type = activity["type"]
            .as_str()
            .and_then(ActivityType::from_str)
            .ok_or_else(|| AppError::Validation("Invalid activity type".to_string()))?;
        
        // Check if domain is blocked
        let domain = extract_domain(actor_uri)?;
        if self.db.is_domain_blocked(&domain).await? {
            return Err(AppError::Forbidden);
        }
        
        // Dispatch to handler
        match activity_type {
            ActivityType::Create => self.handle_create(activity, actor_uri).await,
            ActivityType::Update => self.handle_update(activity, actor_uri).await,
            ActivityType::Delete => self.handle_delete(activity, actor_uri).await,
            ActivityType::Follow => self.handle_follow(activity, actor_uri).await,
            ActivityType::Accept => self.handle_accept(activity, actor_uri).await,
            ActivityType::Undo => self.handle_undo(activity, actor_uri).await,
            ActivityType::Like => self.handle_like(activity, actor_uri).await,
            ActivityType::Announce => self.handle_announce(activity, actor_uri).await,
            _ => {
                tracing::warn!("Unhandled activity type: {:?}", activity_type);
                Ok(())
            }
        }
    }
    
    // TODO: Implement each handler method
    // See src/federation/activity.rs for detailed TODOs
}

fn extract_domain(uri: &str) -> Result<String, AppError> {
    todo!("Extract domain from ActivityPub URI")
}
```

### 2. Complete Activity Delivery

```rust
// src/federation/delivery.rs

impl ActivityDelivery {
    /// Deliver activity to a single inbox
    /// 
    /// TODO: Complete delivery implementation
    /// 1. Serialize activity to JSON
    /// 2. Generate HTTP signature
    /// 3. POST to inbox with signed headers
    /// 4. Handle response and retries
    pub async fn deliver_to_inbox(
        &self,
        inbox_uri: &str,
        activity: serde_json::Value,
    ) -> Result<(), AppError> {
        // Serialize activity
        let body = serde_json::to_string(&activity)?;
        
        // Generate signature
        let signature = self.sign_request("POST", inbox_uri, &body)?;
        
        // Send request
        let response = self.http_client
            .post(inbox_uri)
            .header("Content-Type", "application/activity+json")
            .header("Signature", signature)
            .body(body)
            .send()
            .await?;
        
        if !response.status().is_success() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "Delivery failed: {}",
                response.status()
            )));
        }
        
        Ok(())
    }
    
    fn sign_request(&self, method: &str, uri: &str, body: &str) -> Result<String, AppError> {
        todo!("Generate HTTP signature for request")
    }
}
```

### 3. WebFinger Implementation

```rust
// src/federation/webfinger.rs

/// Generate WebFinger response for local account
/// 
/// TODO: Implement WebFinger response generation
/// - Build JRD (JSON Resource Descriptor)
/// - Include profile URL, ActivityPub actor URL
/// - Add rel links for various protocols
pub fn generate_webfinger_response(
    username: &str,
    domain: &str,
) -> serde_json::Value {
    serde_json::json!({
        "subject": format!("acct:{}@{}", username, domain),
        "aliases": [
            format!("https://{}/users/{}", domain, username),
            format!("https://{}/@{}", domain, username),
        ],
        "links": [
            {
                "rel": "self",
                "type": "application/activity+json",
                "href": format!("https://{}/users/{}", domain, username)
            },
            {
                "rel": "http://webfinger.net/rel/profile-page",
                "type": "text/html",
                "href": format!("https://{}/@{}", domain, username)
            }
        ]
    })
}

/// Fetch remote actor via WebFinger
/// 
/// TODO: Implement WebFinger lookup
/// 1. Parse account address (user@domain)
/// 2. Query /.well-known/webfinger
/// 3. Extract ActivityPub actor URL
/// 4. Fetch and cache actor document
pub async fn fetch_actor(
    http_client: &reqwest::Client,
    account_address: &str,
) -> Result<serde_json::Value, AppError> {
    todo!("Implement WebFinger lookup and actor fetching")
}
```

---

## Background Jobs & Automation

### 1. Scheduled Status Publisher

```rust
// src/service/scheduler.rs

#![allow(dead_code)]

use crate::data::Database;
use crate::error::AppError;
use std::sync::Arc;
use tokio::time::{interval, Duration};

/// Scheduled status publisher service
pub struct SchedulerService {
    db: Arc<Database>,
}

impl SchedulerService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
    
    /// Start scheduler background task
    /// 
    /// TODO: Implement scheduler loop
    /// - Run every minute
    /// - Check for due scheduled statuses
    /// - Publish each status
    /// - Handle errors and retries
    pub async fn start(&self) -> Result<(), AppError> {
        let mut ticker = interval(Duration::from_secs(60));
        
        loop {
            ticker.tick().await;
            if let Err(e) = self.process_due_statuses().await {
                tracing::error!("Scheduler error: {}", e);
            }
        }
    }
    
    async fn process_due_statuses(&self) -> Result<(), AppError> {
        todo!("Find and publish due scheduled statuses")
    }
}
```

### 2. Poll Expiration Handler

```rust
// src/service/polls.rs

#![allow(dead_code)]

use crate::data::Database;
use crate::error::AppError;
use std::sync::Arc;

/// Poll management service
pub struct PollService {
    db: Arc<Database>,
}

impl PollService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
    
    /// Mark expired polls
    /// 
    /// TODO: Implement poll expiration
    /// - Find polls past expires_at
    /// - Mark as expired
    /// - Optionally send notifications
    pub async fn expire_polls(&self) -> Result<usize, AppError> {
        todo!("Mark expired polls in database")
    }
    
    /// Calculate final poll results
    /// 
    /// TODO: Finalize poll results
    /// - Calculate percentages
    /// - Determine winner(s)
    /// - Cache results
    pub async fn finalize_poll(&self, poll_id: &str) -> Result<(), AppError> {
        todo!("Calculate and cache final poll results")
    }
}
```

### 3. Cache Cleanup

```rust
// src/service/cleanup.rs

#![allow(dead_code)]

use crate::data::{TimelineCache, ProfileCache};
use crate::error::AppError;
use std::sync::Arc;

/// Cache cleanup service
pub struct CleanupService {
    timeline_cache: Arc<TimelineCache>,
    profile_cache: Arc<ProfileCache>,
}

impl CleanupService {
    pub fn new(
        timeline_cache: Arc<TimelineCache>,
        profile_cache: Arc<ProfileCache>,
    ) -> Self {
        Self {
            timeline_cache,
            profile_cache,
        }
    }
    
    /// Clean expired cache entries
    /// 
    /// TODO: Implement cache cleanup
    /// - Remove entries older than TTL
    /// - Respect cache size limits
    /// - Log cleanup statistics
    pub async fn cleanup(&self) -> Result<(), AppError> {
        todo!("Clean expired cache entries")
    }
}
```

---

## Performance & Optimization

### 1. Database Indexing

```sql
-- migrations/006_performance_indexes.sql

-- Optimize timeline queries
CREATE INDEX IF NOT EXISTS idx_statuses_created_visibility 
    ON statuses(created_at DESC, visibility);

-- Optimize notification queries
CREATE INDEX IF NOT EXISTS idx_notifications_created_read 
    ON notifications(created_at DESC, read);

-- Optimize search queries
CREATE INDEX IF NOT EXISTS idx_statuses_account_created 
    ON statuses(account_address, created_at DESC);

-- Optimize poll queries
CREATE INDEX IF NOT EXISTS idx_polls_expires 
    ON polls(expires_at) WHERE expired = 0;
```

### 2. Query Optimization

```rust
// src/data/database.rs

impl Database {
    /// Get timeline with optimized query
    /// 
    /// TODO: Optimize timeline query
    /// - Use covering indexes
    /// - Batch fetch related data
    /// - Implement query result caching
    pub async fn get_timeline_optimized(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        todo!("Implement optimized timeline query")
    }
}
```

### 3. Connection Pooling

```rust
// src/data/database.rs

impl Database {
    /// Connect with optimized pool settings
    /// 
    /// TODO: Optimize connection pool
    /// - Set appropriate pool size
    /// - Configure connection timeout
    /// - Enable prepared statement cache
    pub async fn connect_optimized(path: &Path) -> Result<Self, AppError> {
        todo!("Create database connection with optimized settings")
    }
}
```

---

## Testing & Quality

### 1. Integration Tests

```rust
// tests/integration/api_tests.rs

#[tokio::test]
async fn test_poll_voting() {
    todo!("Test poll creation and voting flow")
}

#[tokio::test]
async fn test_scheduled_status() {
    todo!("Test scheduled status creation and publishing")
}

#[tokio::test]
async fn test_conversation_flow() {
    todo!("Test conversation creation and management")
}

#[tokio::test]
async fn test_search_functionality() {
    todo!("Test search across accounts, statuses, and hashtags")
}
```

### 2. Federation Tests

```rust
// tests/integration/federation_tests.rs

#[tokio::test]
async fn test_activity_delivery() {
    todo!("Test ActivityPub activity delivery")
}

#[tokio::test]
async fn test_activity_processing() {
    todo!("Test incoming activity processing")
}

#[tokio::test]
async fn test_webfinger_lookup() {
    todo!("Test WebFinger discovery")
}
```

### 3. Performance Tests

```rust
// tests/performance/load_tests.rs

#[tokio::test]
async fn test_timeline_performance() {
    todo!("Benchmark timeline query performance")
}

#[tokio::test]
async fn test_search_performance() {
    todo!("Benchmark search query performance")
}
```

---

## 📋 Priority Matrix

### Immediate (Next Session)
1. ✅ Complete Search API (full-text + hashtags)
2. ✅ Implement Trends API basics
3. ✅ Add FTS5 migration

### Short Term (1-2 sessions)
4. ✅ Streaming API (WebSocket)
5. ✅ Scheduled status publisher
6. ✅ Poll expiration handler

### Medium Term (3-5 sessions)
7. ✅ ActivityPub federation (complete)
8. ✅ WebFinger implementation
9. ✅ Push notifications

### Long Term (Future)
10. ✅ Performance optimization
11. ✅ Comprehensive testing
12. ✅ Production deployment

---

## 📚 Reference Documents

- **Architecture**: `docs/ARCHITECTURE.md`
- **Data Model**: `docs/DATA_MODEL.md`
- **Storage Strategy**: `docs/STORAGE_STRATEGY.md`
- **API Compliance**: `docs/MASTODON_API_COMPLIANCE_PLAN.md`
- **Development Guide**: `docs/DEVELOPMENT.md`

---

**Last Updated**: 2026-01-11 22:19  
**Next Review**: Start of next development session
