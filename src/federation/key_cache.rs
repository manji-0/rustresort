//! Public Key Caching
//!
//! Caches fetched public keys to reduce remote requests.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::error::AppError;

/// Cached public key entry
#[derive(Debug, Clone)]
struct CachedKey {
    /// PEM-encoded public key
    pem: String,
    /// When this entry was cached
    cached_at: Instant,
    /// TTL for this entry (default: 1 hour)
    ttl: Duration,
}

impl CachedKey {
    /// Check if this cache entry is still valid
    fn is_valid(&self) -> bool {
        self.cached_at.elapsed() < self.ttl
    }
}

/// Public key cache
///
/// Thread-safe cache for remote actor public keys.
/// Reduces network requests by caching fetched keys.
pub struct PublicKeyCache {
    /// Cache storage: key_id -> cached key
    cache: Arc<RwLock<HashMap<String, CachedKey>>>,
    /// HTTP client for fetching keys
    http_client: Arc<reqwest::Client>,
    /// Default TTL for cached keys
    default_ttl: Duration,
}

impl PublicKeyCache {
    /// Create new public key cache
    ///
    /// # Arguments
    /// * `http_client` - HTTP client for fetching keys
    /// * `default_ttl` - Default TTL for cached keys (default: 1 hour)
    pub fn new(http_client: Arc<reqwest::Client>, default_ttl: Option<Duration>) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            http_client,
            default_ttl: default_ttl.unwrap_or(Duration::from_secs(3600)), // 1 hour
        }
    }

    /// Get public key for a key ID
    ///
    /// Checks cache first, fetches from remote if not cached or expired.
    ///
    /// # Arguments
    /// * `key_id` - Full URL to the key (e.g., actor#main-key)
    ///
    /// # Returns
    /// PEM-encoded public key
    pub async fn get(&self, key_id: &str) -> Result<String, AppError> {
        // 1. Check cache (read lock)
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(key_id) {
                if cached.is_valid() {
                    tracing::debug!("Public key cache hit for {}", key_id);
                    return Ok(cached.pem.clone());
                }
                tracing::debug!("Public key cache expired for {}", key_id);
            }
        }

        // 2. Cache miss or expired - fetch from remote
        tracing::debug!("Public key cache miss for {}, fetching...", key_id);
        let pem = super::signature::fetch_public_key(key_id, &self.http_client).await?;

        // 3. Update cache (write lock)
        {
            let mut cache = self.cache.write().await;
            cache.insert(
                key_id.to_string(),
                CachedKey {
                    pem: pem.clone(),
                    cached_at: Instant::now(),
                    ttl: self.default_ttl,
                },
            );
        }

        Ok(pem)
    }

    /// Invalidate a cached key
    ///
    /// Useful when a key is known to be invalid or changed.
    pub async fn invalidate(&self, key_id: &str) {
        let mut cache = self.cache.write().await;
        cache.remove(key_id);
        tracing::debug!("Invalidated public key cache for {}", key_id);
    }

    /// Clear all cached keys
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        tracing::debug!("Cleared all public key cache entries");
    }

    /// Get cache statistics
    pub async fn stats(&self) -> CacheStats {
        let cache = self.cache.read().await;
        let total = cache.len();
        let valid = cache.values().filter(|v| v.is_valid()).count();
        let expired = total - valid;

        CacheStats {
            total_entries: total,
            valid_entries: valid,
            expired_entries: expired,
        }
    }

    /// Prune expired entries
    ///
    /// Should be called periodically to clean up expired entries.
    pub async fn prune_expired(&self) {
        let mut cache = self.cache.write().await;
        let before = cache.len();
        cache.retain(|_, v| v.is_valid());
        let after = cache.len();
        let removed = before - after;

        if removed > 0 {
            tracing::info!("Pruned {} expired public key cache entries", removed);
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Total number of entries
    pub total_entries: usize,
    /// Number of valid (non-expired) entries
    pub valid_entries: usize,
    /// Number of expired entries
    pub expired_entries: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_expiry() {
        let client = Arc::new(reqwest::Client::new());
        let cache = PublicKeyCache::new(client, Some(Duration::from_millis(100)));

        // Manually insert a key
        {
            let mut c = cache.cache.write().await;
            c.insert(
                "test-key".to_string(),
                CachedKey {
                    pem: "test-pem".to_string(),
                    cached_at: Instant::now(),
                    ttl: Duration::from_millis(100),
                },
            );
        }

        // Should be valid immediately
        let stats = cache.stats().await;
        assert_eq!(stats.valid_entries, 1);

        // Wait for expiry
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should be expired
        let stats = cache.stats().await;
        assert_eq!(stats.expired_entries, 1);

        // Prune should remove it
        cache.prune_expired().await;
        let stats = cache.stats().await;
        assert_eq!(stats.total_entries, 0);
    }
}
