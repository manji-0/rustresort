# Quick Reference - Federation Phase 2 API

## Public Key Cache

```rust
use rustresort::federation::PublicKeyCache;
use std::time::Duration;

// Initialize
let cache = PublicKeyCache::new(
    http_client.clone(),
    Some(Duration::from_secs(3600)) // 1 hour TTL
);

// Get key (fetches from remote if not cached)
let pem = cache.get("https://example.com/users/alice#main-key").await?;

// Invalidate a key
cache.invalidate("https://example.com/users/alice#main-key").await;

// Get statistics
let stats = cache.stats().await;
println!("Valid: {}, Expired: {}", stats.valid_entries, stats.expired_entries);

// Prune expired entries (call periodically)
cache.prune_expired().await;
```

---

## Rate Limiter

```rust
use rustresort::federation::{RateLimiter, extract_domain};
use std::time::Duration;

// Initialize
let limiter = RateLimiter::new(
    Some(100),                     // max requests
    Some(Duration::from_secs(60))  // per window
);

// Check and increment
match limiter.check_and_increment("https://example.com/users/alice").await {
    Ok(_) => {
        // Request allowed, process it
    }
    Err(AppError::RateLimited) => {
        // Request blocked
        return Err(AppError::RateLimited);
    }
}

// Get current count
let count = limiter.get_count("https://example.com/users/alice").await;

// Reset a key
limiter.reset("https://example.com/users/alice").await;

// Extract domain for domain-level limiting
let domain = extract_domain("https://example.com/users/alice");
limiter.check_and_increment(&domain).await?;

// Prune old entries (call periodically)
limiter.prune_old().await;
```

---

## Activity Builders

```rust
use rustresort::federation::delivery::builder;

// Follow
let follow = builder::follow(
    "https://myserver.com/activities/123",
    "https://myserver.com/users/me",
    "https://remote.com/users/alice"
);

// Accept
let accept = builder::accept(
    "https://myserver.com/activities/124",
    "https://myserver.com/users/me",
    follow_activity  // Original Follow activity
);

// Create (with Note)
let note = builder::note(
    "https://myserver.com/statuses/456",
    "https://myserver.com/users/me",
    "<p>Hello, world!</p>",
    "2026-01-12T10:00:00Z",
    vec!["https://www.w3.org/ns/activitystreams#Public"],
    vec!["https://myserver.com/users/me/followers"]
);

let create = builder::create(
    "https://myserver.com/activities/125",
    "https://myserver.com/users/me",
    note,
    vec!["https://www.w3.org/ns/activitystreams#Public"],
    vec!["https://myserver.com/users/me/followers"]
);

// Reply
let reply = builder::note_reply(
    "https://myserver.com/statuses/457",
    "https://myserver.com/users/me",
    "<p>Great post!</p>",
    "2026-01-12T10:05:00Z",
    "https://remote.com/statuses/789",  // in_reply_to
    vec!["https://www.w3.org/ns/activitystreams#Public"],
    vec!["https://remote.com/users/alice"]
);

// Like
let like = builder::like(
    "https://myserver.com/activities/126",
    "https://myserver.com/users/me",
    "https://remote.com/statuses/789"
);

// Announce (boost)
let announce = builder::announce(
    "https://myserver.com/activities/127",
    "https://myserver.com/users/me",
    "https://remote.com/statuses/789",
    vec!["https://www.w3.org/ns/activitystreams#Public"]
);

// Delete
let delete = builder::delete(
    "https://myserver.com/activities/128",
    "https://myserver.com/users/me",
    "https://myserver.com/statuses/456"
);

// Undo
let undo = builder::undo(
    "https://myserver.com/activities/129",
    "https://myserver.com/users/me",
    like_activity  // Activity to undo
);
```

---

## Activity Delivery

```rust
use rustresort::federation::ActivityDelivery;

// Initialize
let delivery = ActivityDelivery::new(
    http_client.clone(),
    "https://myserver.com/users/me".to_string(),
    "https://myserver.com/users/me#main-key".to_string(),
    private_key_pem
);

// Send Follow
let follow_id = delivery.send_follow(
    "https://remote.com/users/alice",
    "https://remote.com/users/alice/inbox"
).await?;

// Send Accept
delivery.send_accept(
    "https://remote.com/activities/123",  // Follow activity URI
    "https://remote.com/users/alice/inbox"
).await?;

// Send Create (batch delivery)
let results = delivery.send_create(
    &status,
    vec![
        "https://server1.com/inbox".to_string(),
        "https://server2.com/inbox".to_string(),
    ]
).await;

// Check results
for result in results {
    if result.success {
        println!("✓ {}", result.inbox_uri);
    } else {
        println!("✗ {}: {:?}", result.inbox_uri, result.error);
    }
}

// Send Like
let like_id = delivery.send_like(
    "https://remote.com/statuses/789",
    "https://remote.com/users/alice/inbox"
).await?;

// Send Announce (boost)
let announce_id = delivery.send_announce(
    "https://remote.com/statuses/789",
    follower_inboxes
).await?;

// Send Delete
let results = delivery.send_delete(
    "https://myserver.com/statuses/456",
    follower_inboxes
).await;

// Send Undo
let results = delivery.send_undo(
    "https://myserver.com/activities/126",  // Activity to undo
    follower_inboxes
).await;

// Batch delivery to followers
let results = delivery.deliver_to_followers(
    activity,
    follower_inboxes
).await;
```

---

## Activity Processor

```rust
use rustresort::federation::ActivityProcessor;

// Initialize without delivery
let processor = ActivityProcessor::new(
    db.clone(),
    timeline_cache.clone(),
    profile_cache.clone(),
    http_client.clone(),
    "me@myserver.com".to_string()
);

// Initialize with delivery (enables Accept sending)
let processor = ActivityProcessor::new(
    db.clone(),
    timeline_cache.clone(),
    profile_cache.clone(),
    http_client.clone(),
    "me@myserver.com".to_string()
).with_delivery(delivery.clone());

// Process incoming activity
processor.process(activity, "https://remote.com/users/alice").await?;
```

---

## Complete Integration Example

```rust
use rustresort::federation::{
    ActivityDelivery, ActivityProcessor, PublicKeyCache, RateLimiter
};
use std::sync::Arc;
use std::time::Duration;

// 1. Set up caching and rate limiting
let key_cache = Arc::new(PublicKeyCache::new(
    http_client.clone(),
    Some(Duration::from_secs(3600))
));

let rate_limiter = Arc::new(RateLimiter::new(
    Some(100),
    Some(Duration::from_secs(60))
));

// 2. Set up delivery
let delivery = Arc::new(ActivityDelivery::new(
    http_client.clone(),
    actor_uri,
    key_id,
    private_key_pem
));

// 3. Set up processor with delivery
let processor = ActivityProcessor::new(
    db.clone(),
    timeline_cache.clone(),
    profile_cache.clone(),
    http_client.clone(),
    local_address
).with_delivery(delivery.clone());

// 4. Handle incoming request
async fn handle_inbox(
    activity: serde_json::Value,
    actor_uri: String,
    processor: Arc<ActivityProcessor>,
    rate_limiter: Arc<RateLimiter>,
    key_cache: Arc<PublicKeyCache>,
) -> Result<(), AppError> {
    // Check rate limit
    rate_limiter.check_and_increment(&actor_uri).await?;
    
    // Verify signature (with caching)
    let key_id = extract_key_id_from_signature(&headers)?;
    let public_key = key_cache.get(&key_id).await?;
    verify_signature(method, path, &headers, body, &public_key)?;
    
    // Process activity (may send Accept if it's a Follow)
    processor.process(activity, &actor_uri).await?;
    
    Ok(())
}

// 5. Periodic maintenance
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(300));
    loop {
        interval.tick().await;
        key_cache.prune_expired().await;
        rate_limiter.prune_old().await;
    }
});
```

---

## Error Handling

```rust
use rustresort::error::AppError;

match delivery.send_follow(target, inbox).await {
    Ok(activity_uri) => {
        println!("Follow sent: {}", activity_uri);
    }
    Err(AppError::Federation(msg)) => {
        eprintln!("Federation error: {}", msg);
    }
    Err(AppError::RateLimited) => {
        eprintln!("Rate limited");
    }
    Err(e) => {
        eprintln!("Error: {}", e);
    }
}
```

---

## Monitoring

```rust
// Cache statistics
let cache_stats = key_cache.stats().await;
tracing::info!(
    "Key cache: {} valid, {} expired",
    cache_stats.valid_entries,
    cache_stats.expired_entries
);

// Rate limiter statistics
let rate_stats = rate_limiter.stats().await;
tracing::info!(
    "Rate limiter: {} active entries, max {} req/{}s",
    rate_stats.active_entries,
    rate_stats.max_requests,
    rate_stats.window_seconds
);

// Delivery results
let success = results.iter().filter(|r| r.success).count();
let total = results.len();
tracing::info!("Delivery: {}/{} succeeded", success, total);
```
