//! Example: Complete Federation Setup
//!
//! This example demonstrates how to set up and use all Phase 2 federation features.

use rustresort::data::{ProfileCache, TimelineCache};
use rustresort::federation::{ActivityDelivery, PublicKeyCache, RateLimiter};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // 1. Set up dependencies
    let http_client = Arc::new(reqwest::Client::new());

    // Note: In a real application, you would use Database::from_pool()
    // This example is for demonstration purposes only
    println!("Note: This example demonstrates the API usage.");
    println!("In production, initialize Database with a real connection pool.\n");

    let timeline_cache = Arc::new(TimelineCache::new(1000));
    let profile_cache = Arc::new(ProfileCache::new(3600).await?);

    // 2. Set up public key cache
    let key_cache = Arc::new(PublicKeyCache::new(
        http_client.clone(),
        Some(Duration::from_secs(3600)), // 1 hour TTL
    ));

    // 3. Set up rate limiter
    let rate_limiter = Arc::new(RateLimiter::new(
        Some(100),                     // 100 requests
        Some(Duration::from_secs(60)), // per minute
    ));

    // 4. Set up activity delivery
    let actor_uri = "https://myserver.com/users/me".to_string();
    let key_id = format!("{}#main-key", actor_uri);

    // In production, load from secure storage
    let private_key_pem = "-----BEGIN PRIVATE KEY-----\n...\n-----END PRIVATE KEY-----".to_string();

    let delivery = Arc::new(ActivityDelivery::new(
        http_client.clone(),
        actor_uri.clone(),
        key_id,
        private_key_pem,
    ));

    // 5. Set up activity processor with delivery (without database for this example)
    // In production, you would pass a real database instance
    println!("✓ Federation setup complete!");

    // Example 1: Send a Follow activity
    println!("\n=== Example 1: Sending Follow Activity ===");
    println!("Simulating: delivery.send_follow(...)");
    println!("This would send a Follow activity to a remote actor");
    println!("Returns: Activity URI for tracking");

    // Example 2: Rate limiting check
    println!("\n=== Example 2: Rate Limiting ===");

    // Check rate limit
    match rate_limiter
        .check_and_increment("https://remote.com/users/alice")
        .await
    {
        Ok(_) => {
            println!("✓ Rate limit check passed (1/100)");
        }
        Err(_) => {
            println!("✗ Rate limited!");
        }
    }

    // Check again
    let count = rate_limiter
        .get_count("https://remote.com/users/alice")
        .await;
    println!("✓ Current count for alice: {}", count);

    // Example 3: Batch delivery simulation
    println!("\n=== Example 3: Batch Delivery ===");

    let follower_inboxes = vec![
        "https://server1.com/inbox".to_string(),
        "https://server2.com/users/alice/inbox".to_string(),
        "https://server2.com/users/bob/inbox".to_string(),
    ];

    println!("Would deliver to {} inboxes:", follower_inboxes.len());
    for inbox in &follower_inboxes {
        println!("  - {}", inbox);
    }
    println!("Optimization: Groups by domain, delivers to 2 unique servers");

    // Example 4: Public key caching
    println!("\n=== Example 4: Public Key Caching ===");

    println!("Public key cache reduces remote requests:");
    println!("  - First fetch: Retrieves from remote server");
    println!("  - Subsequent fetches: Returns from cache (much faster)");
    println!("  - TTL: 1 hour (configurable)");
    println!("  - Automatic expiration and pruning");

    // Example 5: Monitor cache and rate limiter
    println!("\n=== Example 5: Monitoring ===");

    let cache_stats = key_cache.stats().await;
    println!("Public Key Cache:");
    println!("  - Total entries: {}", cache_stats.total_entries);
    println!("  - Valid entries: {}", cache_stats.valid_entries);
    println!("  - Expired entries: {}", cache_stats.expired_entries);

    let rate_stats = rate_limiter.stats().await;
    println!("\nRate Limiter:");
    println!("  - Total entries: {}", rate_stats.total_entries);
    println!("  - Active entries: {}", rate_stats.active_entries);
    println!("  - Max requests: {}", rate_stats.max_requests);
    println!("  - Window: {} seconds", rate_stats.window_seconds);

    // Example 6: Periodic maintenance
    println!("\n=== Example 6: Periodic Maintenance ===");

    // Prune expired cache entries
    key_cache.prune_expired().await;
    println!("✓ Pruned expired public key cache entries");

    // Prune old rate limit entries
    rate_limiter.prune_old().await;
    println!("✓ Pruned old rate limit entries");

    println!("\n✓ All examples completed successfully!");

    Ok(())
}

// Example: Background task for periodic maintenance
async fn maintenance_task(key_cache: Arc<PublicKeyCache>, rate_limiter: Arc<RateLimiter>) {
    let mut interval = tokio::time::interval(Duration::from_secs(300)); // 5 minutes

    loop {
        interval.tick().await;

        // Prune caches
        key_cache.prune_expired().await;
        rate_limiter.prune_old().await;

        // Log statistics
        let cache_stats = key_cache.stats().await;
        let rate_stats = rate_limiter.stats().await;

        tracing::info!(
            "Maintenance: key_cache={} entries, rate_limiter={} entries",
            cache_stats.valid_entries,
            rate_stats.active_entries
        );
    }
}
