//! Rate Limiting for Federation
//!
//! Prevents abuse by limiting incoming requests per actor/domain.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::error::AppError;

/// Rate limiter entry
#[derive(Debug, Clone)]
struct RateLimitEntry {
    /// Number of requests in current window
    count: u32,
    /// Window start time
    window_start: Instant,
}

impl RateLimitEntry {
    /// Check if this entry is in a new window
    fn is_new_window(&self, window_duration: Duration) -> bool {
        self.window_start.elapsed() >= window_duration
    }

    /// Increment count or reset if new window
    fn increment(&mut self, window_duration: Duration) {
        if self.is_new_window(window_duration) {
            // New window - reset
            self.count = 1;
            self.window_start = Instant::now();
        } else {
            // Same window - increment
            self.count += 1;
        }
    }
}

/// Rate limiter for federation requests
///
/// Implements a sliding window rate limiter per actor/domain.
pub struct RateLimiter {
    /// Rate limit entries: key -> entry
    entries: Arc<RwLock<HashMap<String, RateLimitEntry>>>,
    /// Maximum requests per window
    max_requests: u32,
    /// Window duration
    window_duration: Duration,
}

impl RateLimiter {
    /// Create new rate limiter
    ///
    /// # Arguments
    /// * `max_requests` - Maximum requests per window (default: 100)
    /// * `window_duration` - Window duration (default: 1 minute)
    pub fn new(max_requests: Option<u32>, window_duration: Option<Duration>) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            max_requests: max_requests.unwrap_or(100),
            window_duration: window_duration.unwrap_or(Duration::from_secs(60)),
        }
    }

    /// Check if a request should be allowed
    ///
    /// # Arguments
    /// * `key` - Rate limit key (actor URI or domain)
    ///
    /// # Returns
    /// Ok if allowed, Err if rate limited
    pub async fn check_and_increment(&self, key: &str) -> Result<(), AppError> {
        let mut entries = self.entries.write().await;

        let entry = entries
            .entry(key.to_string())
            .or_insert_with(|| RateLimitEntry {
                count: 0,
                window_start: Instant::now(),
            });

        // Check if we're in a new window
        if entry.is_new_window(self.window_duration) {
            // Reset for new window
            entry.count = 1;
            entry.window_start = Instant::now();
            Ok(())
        } else if entry.count >= self.max_requests {
            // Rate limited
            Err(AppError::RateLimited)
        } else {
            // Increment and allow
            entry.count += 1;
            Ok(())
        }
    }

    /// Get current count for a key
    pub async fn get_count(&self, key: &str) -> u32 {
        let entries = self.entries.read().await;
        entries
            .get(key)
            .filter(|e| !e.is_new_window(self.window_duration))
            .map(|e| e.count)
            .unwrap_or(0)
    }

    /// Reset rate limit for a key
    pub async fn reset(&self, key: &str) {
        let mut entries = self.entries.write().await;
        entries.remove(key);
    }

    /// Clear all rate limit entries
    pub async fn clear(&self) {
        let mut entries = self.entries.write().await;
        entries.clear();
    }

    /// Prune old entries
    ///
    /// Should be called periodically to clean up expired entries.
    pub async fn prune_old(&self) {
        let mut entries = self.entries.write().await;
        let before = entries.len();
        entries.retain(|_, v| !v.is_new_window(self.window_duration));
        let after = entries.len();
        let removed = before - after;

        if removed > 0 {
            tracing::debug!("Pruned {} old rate limit entries", removed);
        }
    }

    /// Get rate limiter statistics
    pub async fn stats(&self) -> RateLimitStats {
        let entries = self.entries.read().await;
        let total = entries.len();
        let active = entries
            .values()
            .filter(|e| !e.is_new_window(self.window_duration))
            .count();

        RateLimitStats {
            total_entries: total,
            active_entries: active,
            max_requests: self.max_requests,
            window_seconds: self.window_duration.as_secs(),
        }
    }
}

/// Rate limiter statistics
#[derive(Debug, Clone)]
pub struct RateLimitStats {
    /// Total number of entries
    pub total_entries: usize,
    /// Number of active (non-expired) entries
    pub active_entries: usize,
    /// Maximum requests per window
    pub max_requests: u32,
    /// Window duration in seconds
    pub window_seconds: u64,
}

/// Extract domain from actor URI or URL
pub fn extract_domain(uri: &str) -> String {
    uri.split("://")
        .nth(1)
        .and_then(|s| s.split('/').next())
        .unwrap_or(uri)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limit() {
        let limiter = RateLimiter::new(Some(3), Some(Duration::from_secs(1)));

        // First 3 requests should succeed
        assert!(limiter.check_and_increment("test-actor").await.is_ok());
        assert!(limiter.check_and_increment("test-actor").await.is_ok());
        assert!(limiter.check_and_increment("test-actor").await.is_ok());

        // 4th request should be rate limited
        assert!(limiter.check_and_increment("test-actor").await.is_err());

        // Wait for window to reset
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Should succeed again
        assert!(limiter.check_and_increment("test-actor").await.is_ok());
    }

    #[tokio::test]
    async fn test_different_keys() {
        let limiter = RateLimiter::new(Some(2), Some(Duration::from_secs(1)));

        // Different keys should have separate limits
        assert!(limiter.check_and_increment("actor1").await.is_ok());
        assert!(limiter.check_and_increment("actor1").await.is_ok());
        assert!(limiter.check_and_increment("actor2").await.is_ok());
        assert!(limiter.check_and_increment("actor2").await.is_ok());

        // Both should be rate limited now
        assert!(limiter.check_and_increment("actor1").await.is_err());
        assert!(limiter.check_and_increment("actor2").await.is_err());
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://example.com/users/alice"),
            "example.com"
        );
        assert_eq!(
            extract_domain("https://mastodon.social/users/bob"),
            "mastodon.social"
        );
        assert_eq!(extract_domain("invalid"), "invalid");
    }
}
