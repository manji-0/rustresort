//! Rate Limiting for Federation
//!
//! Prevents abuse by limiting incoming requests per actor/domain.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::error::AppError;

const DEFAULT_MAX_TRACKED_KEYS: usize = 10_000;

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
    /// Maximum number of tracked keys in memory
    max_tracked_keys: usize,
}

impl RateLimiter {
    /// Create new rate limiter
    ///
    /// # Arguments
    /// * `max_requests` - Maximum requests per window (default: 100)
    /// * `window_duration` - Window duration (default: 1 minute)
    pub fn new(max_requests: Option<u32>, window_duration: Option<Duration>) -> Self {
        Self::with_max_tracked_keys(max_requests, window_duration, DEFAULT_MAX_TRACKED_KEYS)
    }

    /// Create new rate limiter with explicit in-memory key cap.
    pub fn with_max_tracked_keys(
        max_requests: Option<u32>,
        window_duration: Option<Duration>,
        max_tracked_keys: usize,
    ) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            max_requests: max_requests.unwrap_or(100),
            window_duration: window_duration.unwrap_or(Duration::from_secs(60)),
            max_tracked_keys: max_tracked_keys.max(1),
        }
    }

    fn prune_expired_locked(
        entries: &mut HashMap<String, RateLimitEntry>,
        window_duration: Duration,
    ) -> usize {
        let before = entries.len();
        entries.retain(|_, value| !value.is_new_window(window_duration));
        before - entries.len()
    }

    fn evict_oldest_locked(entries: &mut HashMap<String, RateLimitEntry>) -> bool {
        let Some(oldest_key) = entries
            .iter()
            .min_by_key(|(_, value)| value.window_start)
            .map(|(key, _)| key.clone())
        else {
            return false;
        };
        entries.remove(&oldest_key);
        true
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

        if !entries.contains_key(key) && entries.len() >= self.max_tracked_keys {
            Self::prune_expired_locked(&mut entries, self.window_duration);
            if entries.len() >= self.max_tracked_keys {
                let _ = Self::evict_oldest_locked(&mut entries);
            }
        }

        let entry = entries
            .entry(key.to_string())
            .or_insert_with(|| RateLimitEntry {
                count: 0,
                window_start: Instant::now(),
            });

        if !entry.is_new_window(self.window_duration) && entry.count >= self.max_requests {
            // Rate limited
            Err(AppError::RateLimited)
        } else {
            entry.increment(self.window_duration);
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
        let removed = Self::prune_expired_locked(&mut entries, self.window_duration);

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
            max_tracked_keys: self.max_tracked_keys,
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
    /// Maximum number of keys tracked in memory
    pub max_tracked_keys: usize,
}

fn default_port_for_scheme(scheme: &str) -> Option<u16> {
    if scheme.eq_ignore_ascii_case("http") {
        Some(80)
    } else if scheme.eq_ignore_ascii_case("https") {
        Some(443)
    } else {
        None
    }
}

fn format_domain_key(host: &str, port: Option<u16>, scheme: &str) -> String {
    let normalized_host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let normalized_port = port.filter(|p| Some(*p) != default_port_for_scheme(scheme));

    match normalized_port {
        Some(port) if normalized_host.contains(':') => format!("[{}]:{}", normalized_host, port),
        Some(port) => format!("{}:{}", normalized_host, port),
        None => normalized_host,
    }
}

/// Extract domain from actor URI or URL
pub fn extract_domain(uri: &str) -> String {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Ok(parsed) = url::Url::parse(trimmed) {
        if let Some(host) = parsed.host_str() {
            return format_domain_key(host, parsed.port(), parsed.scheme());
        }
    }

    let fallback = trimmed.split("://").nth(1).unwrap_or(trimmed);
    let authority = fallback
        .split('/')
        .next()
        .unwrap_or(fallback)
        .split('?')
        .next()
        .unwrap_or(fallback)
        .split('#')
        .next()
        .unwrap_or(fallback)
        .trim();
    if authority.is_empty() {
        return String::new();
    }

    if let Ok(parsed_authority) = url::Url::parse(&format!("https://{}", authority)) {
        if let Some(host) = parsed_authority.host_str() {
            return format_domain_key(host, parsed_authority.port(), "https");
        }
    }

    authority.trim_end_matches('.').to_ascii_lowercase()
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

    #[tokio::test]
    async fn test_max_tracked_keys_evicts_oldest_entry() {
        let limiter =
            RateLimiter::with_max_tracked_keys(Some(10), Some(Duration::from_secs(60)), 2);

        assert!(limiter.check_and_increment("actor1").await.is_ok());
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert!(limiter.check_and_increment("actor2").await.is_ok());
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert!(limiter.check_and_increment("actor3").await.is_ok());

        let stats = limiter.stats().await;
        assert_eq!(stats.total_entries, 2);
        assert_eq!(limiter.get_count("actor1").await, 0);
        assert_eq!(limiter.get_count("actor2").await, 1);
        assert_eq!(limiter.get_count("actor3").await, 1);
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
        assert_eq!(
            extract_domain("https://EXAMPLE.COM:443/users/alice"),
            "example.com"
        );
        assert_eq!(
            extract_domain("http://Example.com:80/users/alice"),
            "example.com"
        );
        assert_eq!(
            extract_domain("https://example.com:8443/users/alice"),
            "example.com:8443"
        );
        assert_eq!(
            extract_domain("https://example.com./users/alice"),
            "example.com"
        );
        assert_eq!(extract_domain("example.com:443"), "example.com");
        assert_eq!(
            extract_domain("https://[2001:db8::1]:8443/users/alice"),
            "[2001:db8::1]:8443"
        );
        assert_eq!(
            extract_domain("https://[2001:db8::1]/users/alice"),
            "2001:db8::1"
        );
        assert_eq!(extract_domain("invalid"), "invalid");
    }
}
