//! In-memory caches
//!
//! These caches are volatile and cleared on restart.
//! Uses Moka for high-performance concurrent caching.

use chrono::{DateTime, Utc};
use moka::future::Cache;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

// =============================================================================
// Cached Status (lightweight version for timeline)
// =============================================================================

/// Cached status for timeline display
///
/// This is a lightweight version of Status, only containing
/// fields needed for timeline rendering.
#[derive(Debug, Clone)]
pub struct CachedStatus {
    pub id: String,
    pub uri: String,
    pub content: String,
    /// Account address (user@domain)
    pub account_address: String,
    pub created_at: DateTime<Utc>,
    pub visibility: String,
    pub attachments: Vec<CachedAttachment>,
    pub reply_to_uri: Option<String>,
    pub boost_of_uri: Option<String>,
}

/// Cached media attachment
#[derive(Debug, Clone)]
pub struct CachedAttachment {
    pub url: String,
    pub thumbnail_url: Option<String>,
    pub content_type: String,
    pub description: Option<String>,
    pub blurhash: Option<String>,
}

// =============================================================================
// Timeline Cache
// =============================================================================

/// Timeline cache (volatile, max 2000 items)
///
/// Stores recent statuses from followees.
/// LRU eviction when capacity is reached.
pub struct TimelineCache {
    /// Status ID -> CachedStatus
    statuses: Cache<String, Arc<CachedStatus>>,
    /// Maximum items to keep
    max_items: usize,
}

impl TimelineCache {
    /// Create new timeline cache
    ///
    /// # Arguments
    /// * `max_items` - Maximum number of statuses to cache
    pub fn new(max_items: usize) -> Self {
        let statuses = Cache::builder()
            .max_capacity(max_items as u64)
            .time_to_live(Duration::from_secs(3600 * 24 * 7)) // 7 days TTL
            .build();

        Self {
            statuses,
            max_items,
        }
    }

    /// Insert status into cache
    ///
    /// Automatically evicts oldest items when capacity is reached.
    pub async fn insert(&self, status: CachedStatus) {
        let id = status.id.clone();
        self.statuses.insert(id, Arc::new(status)).await;

        // Update cache size metric
        use crate::metrics::CACHE_SIZE;
        CACHE_SIZE
            .with_label_values(&["timeline"])
            .set(self.statuses.entry_count() as i64);
    }

    /// Get status by ID
    pub async fn get(&self, id: &str) -> Option<Arc<CachedStatus>> {
        let result = self.statuses.get(id).await;

        // Record cache hit/miss
        use crate::metrics::{CACHE_HITS_TOTAL, CACHE_MISSES_TOTAL};
        if result.is_some() {
            CACHE_HITS_TOTAL.with_label_values(&["timeline"]).inc();
        } else {
            CACHE_MISSES_TOTAL.with_label_values(&["timeline"]).inc();
        }

        result
    }

    /// Get status by URI
    pub async fn get_by_uri(&self, uri: &str) -> Option<Arc<CachedStatus>> {
        // Note: This is inefficient but acceptable for cache size of 2000
        // In production, consider maintaining a secondary URI->ID index
        for entry in self.statuses.iter() {
            if entry.1.uri == uri {
                return Some(entry.1.clone());
            }
        }
        None
    }

    /// Remove status from cache
    pub async fn remove(&self, id: &str) {
        self.statuses.invalidate(id).await;
    }

    /// Remove status from cache by ActivityPub URI.
    pub async fn remove_by_uri(&self, uri: &str) {
        if let Some(status) = self.get_by_uri(uri).await {
            self.remove(&status.id).await;
        }
    }

    /// Get home timeline
    ///
    /// Returns statuses from followees, sorted by created_at desc.
    ///
    /// # Arguments
    /// * `followee_addresses` - Set of addresses the user follows
    /// * `limit` - Maximum results
    /// * `max_id` - Return statuses older than this ID
    pub async fn get_home_timeline(
        &self,
        followee_addresses: &HashSet<String>,
        limit: usize,
        max_id: Option<&str>,
    ) -> Vec<Arc<CachedStatus>> {
        let mut statuses: Vec<Arc<CachedStatus>> = self
            .statuses
            .iter()
            .filter(|entry| {
                // Include if from a followee
                followee_addresses.contains(&entry.1.account_address)
            })
            .filter(|entry| {
                // Apply max_id filter if provided
                if let Some(max_id) = max_id {
                    entry.1.id < max_id.to_string()
                } else {
                    true
                }
            })
            .map(|entry| entry.1.clone())
            .collect();

        // Sort by created_at descending
        statuses.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        // Limit results
        statuses.truncate(limit);

        statuses
    }

    /// Get public timeline
    ///
    /// Returns all public statuses in cache.
    pub async fn get_public_timeline(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Vec<Arc<CachedStatus>> {
        let mut statuses: Vec<Arc<CachedStatus>> = self
            .statuses
            .iter()
            .filter(|entry| entry.1.visibility == "public")
            .filter(|entry| {
                if let Some(max_id) = max_id {
                    entry.1.id < max_id.to_string()
                } else {
                    true
                }
            })
            .map(|entry| entry.1.clone())
            .collect();

        statuses.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        statuses.truncate(limit);

        statuses
    }
}

// =============================================================================
// Cached Profile
// =============================================================================

/// Cached user profile
///
/// Full profile data for followees and followers.
#[derive(Debug, Clone)]
pub struct CachedProfile {
    /// Account address (user@domain)
    pub address: String,
    /// ActivityPub actor URI
    pub uri: String,
    pub display_name: Option<String>,
    pub note: Option<String>,
    pub avatar_url: Option<String>,
    pub header_url: Option<String>,
    /// RSA public key for signature verification
    pub public_key_pem: String,
    /// Inbox URI for activity delivery
    pub inbox_uri: String,
    /// Outbox URI for fetching posts
    pub outbox_uri: Option<String>,
    pub followers_count: Option<u64>,
    pub following_count: Option<u64>,
    /// When this profile was last fetched
    pub fetched_at: DateTime<Utc>,
}

// =============================================================================
// Profile Cache
// =============================================================================

/// Profile cache for followees and followers
///
/// Populated on startup by fetching from follow addresses in DB.
/// Updated when Update activities are received.
pub struct ProfileCache {
    /// Address (user@domain) -> CachedProfile
    profiles: Cache<String, Arc<CachedProfile>>,
}

impl ProfileCache {
    /// Create new profile cache
    pub fn new() -> Self {
        let profiles = Cache::builder()
            .time_to_live(Duration::from_secs(86400)) // 24 hours TTL
            .build();

        Self { profiles }
    }

    /// Initialize cache from follow addresses
    ///
    /// Fetches profiles for all followees and followers in parallel.
    /// Called on application startup.
    ///
    /// # Arguments
    /// * `addresses` - List of addresses (user@domain) to fetch
    /// * `http_client` - HTTP client for fetching
    pub async fn initialize_from_addresses(
        &self,
        addresses: &[String],
        _http_client: &reqwest::Client,
    ) {
        // Fetch profiles in parallel (max 10 concurrent)
        use futures::stream::{self, StreamExt};

        stream::iter(addresses)
            .map(|address| async move {
                // TODO: Implement WebFinger + Actor fetch
                // For now, just log that we would fetch
                tracing::debug!(address = %address, "Would fetch profile");
                // Placeholder: In real implementation, this would:
                // 1. Perform WebFinger lookup
                // 2. Fetch actor JSON
                // 3. Parse and cache profile
            })
            .buffer_unordered(10)
            .collect::<Vec<_>>()
            .await;
    }

    /// Get profile by address
    pub async fn get(&self, address: &str) -> Option<Arc<CachedProfile>> {
        let result = self.profiles.get(address).await;

        // Record cache hit/miss
        use crate::metrics::{CACHE_HITS_TOTAL, CACHE_MISSES_TOTAL};
        if result.is_some() {
            CACHE_HITS_TOTAL.with_label_values(&["profile"]).inc();
        } else {
            CACHE_MISSES_TOTAL.with_label_values(&["profile"]).inc();
        }

        result
    }

    /// Insert or update profile
    pub async fn insert(&self, profile: CachedProfile) {
        let address = profile.address.clone();
        self.profiles.insert(address, Arc::new(profile)).await;

        // Update cache size metric
        use crate::metrics::CACHE_SIZE;
        CACHE_SIZE
            .with_label_values(&["profile"])
            .set(self.profiles.entry_count() as i64);
    }

    /// Update profile from ActivityPub Update activity
    ///
    /// Called when receiving Update activity for a known actor.
    pub async fn update_from_activity(&self, actor_uri: &str, _update_data: serde_json::Value) {
        // Find profile by URI
        for entry in self.profiles.iter() {
            if entry.1.uri == actor_uri {
                // TODO: Parse update_data and update profile fields
                // For now, just log
                tracing::debug!(uri = %actor_uri, "Would update profile from activity");
                break;
            }
        }
    }

    /// Get public key for signature verification
    ///
    /// # Arguments
    /// * `address` - Account address (user@domain)
    ///
    /// # Returns
    /// PEM-encoded public key or None if not cached
    pub async fn get_public_key(&self, address: &str) -> Option<String> {
        self.profiles
            .get(address)
            .await
            .map(|p| p.public_key_pem.clone())
    }

    /// Get inbox URI for activity delivery
    pub async fn get_inbox(&self, address: &str) -> Option<String> {
        self.profiles
            .get(address)
            .await
            .map(|p| p.inbox_uri.clone())
    }
}

impl Default for ProfileCache {
    fn default() -> Self {
        Self::new()
    }
}
