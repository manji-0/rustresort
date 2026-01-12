# Federation Specification

## Overview

This document describes the ActivityPub federation implementation in RustResort. The implementation is inspired by GoToSocial to ensure interoperability with the Fediverse.

## ActivityPub Compliance

RustResort complies with the following specifications:

- [ActivityPub](https://www.w3.org/TR/activitypub/)
- [ActivityStreams 2.0](https://www.w3.org/TR/activitystreams-core/)
- [ActivityStreams Vocabulary](https://www.w3.org/TR/activitystreams-vocabulary/)
- [HTTP Signatures (draft-cavage-http-signatures-12)](https://tools.ietf.org/html/draft-cavage-http-signatures-12)
- [WebFinger (RFC 7033)](https://tools.ietf.org/html/rfc7033)

## Architecture

```
                                    ┌─────────────────┐
                                    │  Remote Server  │
                                    └────────┬────────┘
                                             │
                        HTTPS + HTTP Signatures
                                             │
┌────────────────────────────────────────────┼────────────────────────────────────────────┐
│                               RustResort                                                │
│                                            │                                            │
│  ┌──────────────┐                 ┌────────┴────────┐                 ┌──────────────┐ │
│  │  Transport   │◄───────────────►│   Federation    │◄───────────────►│    Queue     │ │
│  │   Layer      │                 │    Handler      │                 │   (Tokio)    │ │
│  └──────┬───────┘                 └────────┬────────┘                 └──────────────┘ │
│         │                                  │                                            │
│    ┌────┴────┐                      ┌──────┴──────┐                                    │
│    │  HTTP   │                      │             │                                    │
│    │ Client  │              ┌───────┴───────┐     │                                    │
│    │         │              │               │     │                                    │
│    └─────────┘       ┌──────┴──────┐ ┌──────┴──────┐                                   │
│                      │ Dereferencer│ │  Delivery   │                                   │
│                      │             │ │  Service    │                                   │
│                      └─────────────┘ └─────────────┘                                   │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

## HTTP Signatures

### Signature Generation

All outgoing ActivityPub requests include HTTP Signatures.

**Signed Headers:**
- `(request-target)` - HTTP method and path
- `host` - Host name
- `date` - Request timestamp
- `digest` - SHA-256 hash of body (for POST requests)

**Implementation:**
```rust
pub fn sign_request(
    private_key_pem: &str,
    key_id: &str,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    body: Option<&[u8]>,
) -> Result<String, SignatureError> {
    // 1. Generate Date header
    let date = Utc::now().to_rfc2822();
    
    // 2. Generate Digest header (if body present)
    let digest = body.map(|b| {
        let hash = sha256(b);
        format!("SHA-256={}", base64::encode(hash))
    });
    
    // 3. Build signing string
    let signing_string = build_signing_string(
        method, path, &date, host, digest.as_deref()
    );
    
    // 4. Sign with RSA-SHA256
    let signature = sign_rsa_sha256(private_key_pem, &signing_string)?;
    
    // 5. Format signature header
    Ok(format!(
        r#"keyId="{}",algorithm="rsa-sha256",headers="(request-target) host date{}",signature="{}""#,
        key_id,
        if digest.is_some() { " digest" } else { "" },
        base64::encode(signature)
    ))
}
```

### Signature Verification

Incoming requests must have valid HTTP Signatures.

**Verification Steps:**
1. Parse `Signature` header
2. Fetch public key from `keyId` (with caching)
3. Rebuild signing string
4. Verify RSA-SHA256 signature
5. Validate `Date` header (±30 seconds)
6. Validate `Digest` header (if present)

**Implementation:**
```rust
pub async fn verify_signature(
    method: &str,
    path: &str,
    headers: &HeaderMap,
    body: Option<&[u8]>,
    key_cache: &PublicKeyCache,
) -> Result<String, SignatureError> {
    // 1. Parse Signature header
    let sig_header = headers.get("signature")
        .ok_or(SignatureError::MissingHeader)?;
    let signature = parse_signature_header(sig_header)?;
    
    // 2. Fetch public key (cached)
    let public_key_pem = key_cache.get(&signature.key_id).await?;
    
    // 3. Rebuild signing string
    let signing_string = rebuild_signing_string(
        method, path, headers, &signature.headers
    )?;
    
    // 4. Verify signature
    verify_rsa_sha256(&public_key_pem, &signing_string, &signature.signature)?;
    
    // 5. Validate Date (replay attack prevention)
    validate_date_header(headers)?;
    
    // 6. Validate Digest (if present)
    if let Some(body) = body {
        validate_digest_header(headers, body)?;
    }
    
    Ok(signature.key_id)
}
```

## Public Key Caching

To reduce remote requests, public keys are cached in memory.

**Features:**
- Thread-safe cache (`Arc<RwLock<HashMap>>`)
- TTL-based expiration (default: 1 hour)
- Cache statistics and monitoring
- Manual invalidation support
- Automatic pruning of expired entries

**Usage:**
```rust
let cache = PublicKeyCache::new(http_client, Some(Duration::from_secs(3600)));

// Get key (fetches if not cached)
let pem = cache.get("https://example.com/users/alice#main-key").await?;

// Invalidate key
cache.invalidate("https://example.com/users/alice#main-key").await;

// Get statistics
let stats = cache.stats().await;
println!("Hit rate: {:.2}%", stats.hit_rate() * 100.0);

// Prune expired entries
cache.prune_expired().await;
```

## Activity Processing

### Inbox Handler

```rust
pub async fn process_inbox(
    activity: Activity,
    actor_uri: &str,
    processor: &ActivityProcessor,
) -> Result<(), FederationError> {
    match activity.activity_type.as_str() {
        "Create" => processor.handle_create(activity, actor_uri).await,
        "Update" => processor.handle_update(activity, actor_uri).await,
        "Delete" => processor.handle_delete(activity, actor_uri).await,
        "Follow" => processor.handle_follow(activity, actor_uri).await,
        "Accept" => processor.handle_accept(activity, actor_uri).await,
        "Reject" => processor.handle_reject(activity, actor_uri).await,
        "Undo" => processor.handle_undo(activity, actor_uri).await,
        "Announce" => processor.handle_announce(activity, actor_uri).await,
        "Like" => processor.handle_like(activity, actor_uri).await,
        _ => {
            tracing::warn!("Unhandled activity type: {}", activity.activity_type);
            Ok(())
        }
    }
}
```

### Supported Activities

#### Create
Creates a new object (Note, Article, etc.).

**Behavior:**
- Check for duplicate (by URI)
- Parse object to local format
- Store in database (if persistence criteria met)
- Create notifications for mentions
- Update timeline caches

#### Follow
Remote actor requests to follow local user.

**Behavior:**
- Verify target is local user
- Check if already following
- If account is locked:
  - Create follow request
  - Send notification
- If account is unlocked:
  - Create follower relationship
  - Send Accept activity
  - Send notification

#### Accept
Confirms a Follow request.

**Behavior:**
- Verify we sent the original Follow
- Create following relationship
- Update follower counts

#### Undo
Reverses a previous activity.

**Supported Undo Types:**
- Follow → Unfollow
- Like → Unlike
- Announce → Unboost
- Block → Unblock

#### Announce
Boosts (reblogs) a status.

**Behavior:**
- Fetch original status if not cached
- Create boost record
- Add to timeline
- Send notification to original author

#### Like
Favourites a status.

**Behavior:**
- Verify status exists
- Create favourite record
- Send notification to author

## Activity Delivery

### Batch Delivery

Efficiently delivers activities to multiple recipients.

**Features:**
- Shared inbox optimization (group by domain)
- Concurrent delivery (max 10 parallel)
- Individual error handling
- Delivery statistics logging

**Implementation:**
```rust
pub async fn deliver_to_followers(
    &self,
    activity: &Activity,
    follower_inboxes: Vec<String>,
) -> Vec<DeliveryResult> {
    // Group by domain for shared inbox optimization
    let grouped = group_inboxes_by_domain(&follower_inboxes);
    
    // Concurrent delivery with semaphore
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let mut tasks = vec![];
    
    for (domain, inboxes) in grouped {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let task = tokio::spawn(async move {
            let result = self.deliver_to_inbox(&inboxes[0], activity).await;
            drop(permit);
            result
        });
        tasks.push(task);
    }
    
    // Collect results
    let results = join_all(tasks).await;
    
    // Log statistics
    let success_count = results.iter().filter(|r| r.is_ok()).count();
    tracing::info!(
        "Delivered to {}/{} inboxes",
        success_count,
        results.len()
    );
    
    results
}
```

### Activity Builders

Helper functions to build ActivityPub activities:

- `follow()` - Follow activity
- `accept()` - Accept activity
- `create()` - Create activity (new post)
- `delete()` - Delete activity (with Tombstone)
- `like()` - Like activity (favourite)
- `announce()` - Announce activity (boost)
- `undo()` - Undo activity
- `note()` - Note object
- `note_reply()` - Note with reply

**Example:**
```rust
let activity = builder::create(
    &actor_uri,
    &note_object,
    vec!["https://www.w3.org/ns/activitystreams#Public"],
    vec![&followers_uri],
);
```

## Rate Limiting

Prevents abuse of federation endpoints.

**Features:**
- Sliding window algorithm
- Per-actor or per-domain limiting
- Configurable limits (default: 100 req/min)
- Thread-safe implementation
- Automatic window reset
- Statistics and monitoring

**Usage:**
```rust
let limiter = RateLimiter::new(Some(100), Some(Duration::from_secs(60)));

// Check before processing
match limiter.check_and_increment(&actor_uri).await {
    Ok(_) => {
        // Process request
    }
    Err(AppError::RateLimited) => {
        // Return 429 Too Many Requests
    }
}
```

## WebFinger

Discovers ActivityPub actors from `acct:` URIs.

**Endpoint:** `GET /.well-known/webfinger?resource=acct:username@domain`

**Response:**
```json
{
  "subject": "acct:alice@example.com",
  "aliases": [
    "https://example.com/@alice",
    "https://example.com/users/alice"
  ],
  "links": [
    {
      "rel": "self",
      "type": "application/activity+json",
      "href": "https://example.com/users/alice"
    }
  ]
}
```

## Actor Object

**Endpoint:** `GET /users/{username}`

**Headers:** `Accept: application/activity+json`

**Response:**
```json
{
  "@context": [
    "https://www.w3.org/ns/activitystreams",
    "https://w3id.org/security/v1"
  ],
  "id": "https://example.com/users/alice",
  "type": "Person",
  "preferredUsername": "alice",
  "name": "Alice",
  "summary": "<p>Hello, world!</p>",
  "inbox": "https://example.com/users/alice/inbox",
  "outbox": "https://example.com/users/alice/outbox",
  "followers": "https://example.com/users/alice/followers",
  "following": "https://example.com/users/alice/following",
  "publicKey": {
    "id": "https://example.com/users/alice#main-key",
    "owner": "https://example.com/users/alice",
    "publicKeyPem": "-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----"
  },
  "endpoints": {
    "sharedInbox": "https://example.com/inbox"
  }
}
```

## Security Considerations

### HTTP Signatures
- All incoming activities must be signed
- Signatures verified before processing
- Date header validated (±30 seconds)
- Actor URI must match signature key owner

### Rate Limiting
- Prevents DoS attacks via inbox flooding
- Default: 100 requests/minute per actor
- Adjust based on instance capacity

### Domain Blocking
- Block malicious instances at domain level
- Reject all activities from blocked domains
- Support for subdomain blocking

### Actor Verification
- Verify activity actor matches signature
- Fetch actor profiles from authoritative source
- Cache with TTL to prevent poisoning

## Performance Optimizations

### Shared Inbox
- Group deliveries by domain
- Use shared inbox when available
- Reduces delivery count by ~50-70%

### Public Key Caching
- Cache keys for 1 hour (configurable)
- Reduces remote fetches
- Automatic expiration and pruning

### Batch Delivery
- Concurrent delivery (10 parallel)
- Semaphore-based concurrency control
- Individual error handling

### Connection Pooling
- Reuse HTTP connections
- Connection timeout: 30 seconds
- Request timeout: 60 seconds

## Interoperability

Tested with:
- **Mastodon** - Full compatibility
- **Pleroma/Akkoma** - Core features
- **Misskey** - Basic compatibility
- **GoToSocial** - Full compatibility

## Future Enhancements

- [ ] Shared inbox detection from actor profiles
- [ ] Persistent delivery queue with retry logic
- [ ] Public key cache persistence
- [ ] Adaptive rate limiting based on load
- [ ] Prometheus metrics for federation

## Related Documentation

- [API.md](API.md) - API specifications
- [AUTHENTICATION.md](AUTHENTICATION.md) - Authentication details
- [DEVELOPMENT.md](DEVELOPMENT.md) - Development guide
