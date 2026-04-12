# TODO: Unimplemented Features – Trait Definitions & Implementation Tasks

Each section lists:
1. **Trait** – where to define it and what methods it needs
2. **Impl** – what concrete type(s) should implement it and what side work is required

Conventions follow the existing codebase:
- Traits live in `src/data/repository.rs` (data layer) or a dedicated module
- All traits derive `Send + Sync`
- Async methods use `#[async_trait]`
- Errors use `AppError`

---

## 1. Streaming Event Bus

Implemented with broadcast-backed SSE streams and exercised by E2E coverage.

**Define trait** in `src/service/streaming.rs` (new file):

```rust
#[async_trait]
pub trait StreamingEventBus: Send + Sync {
    /// Subscribe to events for the authenticated user (notifications, home timeline updates).
    async fn subscribe_user(&self, account_id: &str) -> Result<EventReceiver, AppError>;
    /// Subscribe to the public (federated) timeline.
    async fn subscribe_public(&self) -> Result<EventReceiver, AppError>;
    /// Subscribe to the local-only public timeline.
    async fn subscribe_public_local(&self) -> Result<EventReceiver, AppError>;
    /// Subscribe to a hashtag timeline.
    async fn subscribe_hashtag(&self, hashtag: &str) -> Result<EventReceiver, AppError>;
    /// Subscribe to a user-defined list timeline.
    async fn subscribe_list(&self, list_id: &str) -> Result<EventReceiver, AppError>;
    /// Subscribe to the direct-message (conversation) stream.
    async fn subscribe_direct(&self, account_id: &str) -> Result<EventReceiver, AppError>;
    /// Publish an event to all matching subscribers.
    async fn publish(&self, event: StreamEvent) -> Result<(), AppError>;
}
```

**Impl tasks:**
- [x] Define `StreamEvent` enum (`Update`, `Delete`, `Notification`, `FiltersChanged`, `Announcement`, `AnnouncementReaction`, `AnnouncementDelete`, `Conversation`)
- [x] Define `EventReceiver` as a `tokio::sync::broadcast::Receiver<StreamEvent>` wrapper
- [x] Implement `BroadcastEventBus` using `tokio::sync::broadcast` channels keyed by stream type
- [x] Wire `BroadcastEventBus` into `AppState` and inject into status/notification services so they call `publish()` after writes
- [x] Replace placeholder handlers in `src/api/mastodon/streaming.rs` to use `subscribe_*` methods

---

## 2. Web Push Notifications

Implemented for the single-user deployment model, including VAPID key generation and E2E delivery tests.

**Define trait** in `src/data/repository.rs`:

```rust
#[async_trait]
pub trait PushSubscriptionRepository: Send + Sync {
    /// Store a new Web Push subscription for the current user.
    async fn upsert_subscription(
        &self,
        endpoint: &str,
        keys_p256dh: &str,
        keys_auth: &str,
        alerts: &PushAlerts,
    ) -> Result<PushSubscription, AppError>;
    /// Retrieve the current subscription (single-user instance: at most one).
    async fn get_subscription(&self) -> Result<Option<PushSubscription>, AppError>;
    /// Delete the subscription.
    async fn delete_subscription(&self) -> Result<(), AppError>;
}
```

**Define trait** in `src/service/push.rs` (new file):

```rust
#[async_trait]
pub trait WebPushSender: Send + Sync {
    /// Encrypt and deliver a push payload to the given subscription endpoint.
    async fn send(
        &self,
        subscription: &PushSubscription,
        payload: &PushPayload,
    ) -> Result<(), AppError>;
}
```

**Impl tasks:**
- [x] Add `push_subscriptions` migration (endpoint, p256dh, auth, alerts JSON, created_at)
- [x] Define `PushSubscription`, `PushAlerts`, `PushPayload` structs in `src/data/models.rs`
- [x] Implement `PushSubscriptionRepository` on `Database`
- [x] Implement `WebPushSender` using the `web-push` crate (VAPID + AES-GCM-128 encryption)
- [x] Add VAPID key generation/storage to config
- [x] Add API handlers in `src/api/mastodon/` for `GET/POST/DELETE /api/v1/push/subscription`
- [x] Call `WebPushSender::send` from notification insertion flow after creating a notification

---

## 3. Persistent Activity Delivery Queue

Outbound federation delivery uses an in-memory queue; failures are lost on restart.

**Define trait** in `src/federation/delivery.rs` (extend existing):

```rust
#[async_trait]
pub trait DeliveryQueue: Send + Sync {
    /// Enqueue an activity for delivery to a specific inbox URL.
    async fn enqueue(
        &self,
        inbox_url: &str,
        activity_json: &str,
        actor_key_id: &str,
    ) -> Result<(), AppError>;
    /// Claim the next batch of pending jobs (called by worker).
    async fn claim_pending(&self, limit: usize) -> Result<Vec<DeliveryJob>, AppError>;
    /// Mark a job as successfully delivered.
    async fn mark_delivered(&self, job_id: &str) -> Result<(), AppError>;
    /// Record a failure; increment attempt counter and set next_attempt_at with backoff.
    async fn mark_failed(&self, job_id: &str, error: &str) -> Result<(), AppError>;
    /// Delete jobs that have exceeded max retries.
    async fn reap_dead_jobs(&self, max_attempts: u32) -> Result<u64, AppError>;
}
```

**Impl tasks:**
- [x] Add `delivery_jobs` migration (id, inbox_url, activity_json, actor_key_id, attempts, last_error, next_attempt_at, delivered_at)
- [x] Define `DeliveryJob` struct in `src/data/models.rs`
- [x] Implement `DeliveryQueue` on `Database`
- [x] Implement a `DeliveryWorker` that polls `claim_pending`, attempts delivery, and calls `mark_delivered`/`mark_failed` with exponential backoff
- [x] Spawn `DeliveryWorker` as a background `tokio::task` at startup
- [x] Replace fire-and-forget delivery calls with `DeliveryQueue::enqueue`

---

## 4. Account Migration (Move Activity)

Sending and receiving `Move` activities is not implemented.

**Define trait** in `src/federation/` (new file `migration.rs`):

```rust
#[async_trait]
pub trait AccountMigrationHandler: Send + Sync {
    /// Handle an incoming `Move` activity (remote actor migrating to a new account).
    async fn handle_move(
        &self,
        actor_uri: &str,
        target_uri: &str,
    ) -> Result<(), AppError>;
    /// Build and send a `Move` activity when the local user initiates migration.
    async fn initiate_move(
        &self,
        new_account_uri: &str,
    ) -> Result<(), AppError>;
}
```

**Define trait** in `src/data/repository.rs`:

```rust
#[async_trait]
pub trait MigrationRepository: Send + Sync {
    /// Persist the `alsoKnownAs` alias on the local actor.
    async fn set_also_known_as(&self, uri: &str) -> Result<(), AppError>;
    /// Return the current `alsoKnownAs` value.
    async fn get_also_known_as(&self) -> Result<Option<String>, AppError>;
    /// Redirect all local followers to the new account (update follows table).
    async fn migrate_followers_to(&self, new_account_uri: &str) -> Result<u64, AppError>;
}
```

**Impl tasks:**
- [x] Add `also_known_as` column to `account` table migration
- [x] Implement `MigrationRepository` on `Database`
- [x] Implement `AccountMigrationHandler`; call `DeliveryQueue::enqueue` for `Move` activity
- [x] Expose `PUT /api/v1/accounts/update_credentials` `moved_to_account_id` field
- [x] Handle incoming `Move` in `src/federation/activity.rs` `process_inbox`
- [x] Include `alsoKnownAs` and `movedTo` in the actor JSON in `src/api/activitypub.rs`

---

## 5. Scheduled Status Executor

Scheduled statuses are stored but never automatically published.

**Define trait** in `src/service/` (new file `scheduler.rs`):

```rust
#[async_trait]
pub trait ScheduledStatusRepository: Send + Sync {
    /// Return all scheduled statuses whose `scheduled_at` is now or in the past.
    async fn get_due_statuses(&self) -> Result<Vec<ScheduledStatus>, AppError>;
    /// Mark a scheduled status as published (delete or archive the row).
    async fn mark_published(&self, id: &str) -> Result<(), AppError>;
    /// Mark a scheduled status as failed with an error message.
    async fn mark_failed(&self, id: &str, error: &str) -> Result<(), AppError>;
}

#[async_trait]
pub trait SchedulerRunner: Send + Sync {
    /// Poll for due statuses and publish them. Called on a periodic timer.
    async fn run_once(&self) -> Result<(), AppError>;
}
```

**Impl tasks:**
- [x] Add `error` and `published_at` columns to `scheduled_statuses` migration
- [x] Implement `ScheduledStatusRepository` on `Database`
- [x] Implement `SchedulerRunner` that calls `get_due_statuses`, creates each via `StatusService`, calls `mark_published`/`mark_failed`
- [x] Spawn `SchedulerRunner` as a background `tokio::task` with `tokio::time::interval` (e.g., every 60 s)

---

## 6. Two-Factor Authentication (TOTP)

No 2FA implementation exists.

**Define trait** in `src/auth/` (new file `totp.rs`):

```rust
#[async_trait]
pub trait TotpRepository: Send + Sync {
    /// Persist an encrypted TOTP secret for the user (disabled until confirmed).
    async fn store_secret(&self, encrypted_secret: &str) -> Result<(), AppError>;
    /// Retrieve the stored TOTP secret.
    async fn get_secret(&self) -> Result<Option<String>, AppError>;
    /// Enable TOTP after the user confirms a valid code.
    async fn enable(&self) -> Result<(), AppError>;
    /// Disable TOTP (requires valid code or recovery code).
    async fn disable(&self) -> Result<(), AppError>;
    /// Return whether TOTP is currently enabled.
    async fn is_enabled(&self) -> Result<bool, AppError>;
    /// Store hashed backup codes (replace existing set).
    async fn store_backup_codes(&self, hashed_codes: &[String]) -> Result<(), AppError>;
    /// Consume one backup code (returns false if not found or already used).
    async fn consume_backup_code(&self, hashed_code: &str) -> Result<bool, AppError>;
}
```

**Impl tasks:**
- [ ] Add `totp_secret`, `totp_enabled`, `totp_backup_codes` columns to `account` migration
- [ ] Implement `TotpRepository` on `Database`
- [ ] Implement TOTP verification using the `totp-rs` crate
- [ ] Add `POST /api/v1/totp/enroll`, `POST /api/v1/totp/confirm`, `DELETE /api/v1/totp` endpoints
- [ ] Enforce TOTP check in built-in local login and passkey management flows when enabled

---

## 7. Instance Announcements

No announcement API exists.

**Define trait** in `src/data/repository.rs`:

```rust
#[async_trait]
pub trait AnnouncementRepository: Send + Sync {
    /// List all active announcements (most recent first).
    async fn list_announcements(&self) -> Result<Vec<Announcement>, AppError>;
    /// Get a single announcement.
    async fn get_announcement(&self, id: &str) -> Result<Option<Announcement>, AppError>;
    /// Create a new announcement (admin only).
    async fn create_announcement(&self, text: &str, ends_at: Option<DateTime<Utc>>) -> Result<Announcement, AppError>;
    /// Update announcement text / schedule.
    async fn update_announcement(&self, id: &str, text: &str, ends_at: Option<DateTime<Utc>>) -> Result<(), AppError>;
    /// Delete an announcement.
    async fn delete_announcement(&self, id: &str) -> Result<(), AppError>;
    /// Record that the user has dismissed an announcement.
    async fn dismiss_announcement(&self, id: &str) -> Result<(), AppError>;
    /// Add a reaction emoji to an announcement.
    async fn add_reaction(&self, id: &str, emoji: &str) -> Result<(), AppError>;
    /// Remove a reaction emoji from an announcement.
    async fn remove_reaction(&self, id: &str, emoji: &str) -> Result<(), AppError>;
}
```

**Impl tasks:**
- [ ] Add `announcements` and `announcement_reactions` migrations
- [ ] Define `Announcement` struct in `src/data/models.rs`
- [ ] Implement `AnnouncementRepository` on `Database`
- [ ] Add API handlers for `GET/POST/PUT/DELETE /api/v1/announcements` and `PUT/DELETE /api/v1/announcements/:id/reactions/:name`
- [ ] Publish `StreamEvent::Announcement` via `StreamingEventBus` on create/delete

---

## 8. Trends

No trends API exists (`/api/v1/trends`).

**Define trait** in `src/data/repository.rs`:

```rust
#[async_trait]
pub trait TrendRepository: Send + Sync {
    /// Return trending hashtags sorted by recent usage.
    async fn trending_hashtags(&self, limit: usize) -> Result<Vec<TrendingHashtag>, AppError>;
    /// Return trending statuses (high interaction in last 24 h).
    async fn trending_statuses(&self, limit: usize) -> Result<Vec<Status>, AppError>;
    /// Return trending links (most-shared URLs).
    async fn trending_links(&self, limit: usize) -> Result<Vec<TrendingLink>, AppError>;
    /// Increment usage count for a hashtag (called on status create).
    async fn record_hashtag_use(&self, hashtag: &str) -> Result<(), AppError>;
}
```

**Impl tasks:**
- [ ] Add `hashtag_daily_uses` migration (hashtag, date, count)
- [ ] Define `TrendingHashtag`, `TrendingLink` structs in `src/data/models.rs`
- [ ] Implement `TrendRepository` on `Database`
- [ ] Add API handlers for `GET /api/v1/trends/tags`, `GET /api/v1/trends/statuses`, `GET /api/v1/trends/links`
- [ ] Call `record_hashtag_use` from `StatusService` when a status with hashtags is created

---

## 9. Block Activity Enforcement

`Block` activities from remote actors are parsed but not enforced: blocked actors can still reach the inbox.

**Define trait** in `src/data/repository.rs`:

```rust
#[async_trait]
pub trait FederationBlockRepository: Send + Sync {
    /// Record that a remote actor has blocked the local user.
    async fn record_remote_block(&self, actor_uri: &str) -> Result<(), AppError>;
    /// Remove a remote block (on Undo Block).
    async fn remove_remote_block(&self, actor_uri: &str) -> Result<(), AppError>;
    /// Return true if the remote actor has blocked the local user.
    async fn is_blocked_by_remote(&self, actor_uri: &str) -> Result<bool, AppError>;
}
```

**Impl tasks:**
- [ ] Add `remote_blocks` migration (actor_uri, created_at)
- [ ] Implement `FederationBlockRepository` on `Database`
- [ ] Call `record_remote_block` / `remove_remote_block` in `handle_block` / `handle_undo` in `src/federation/activity.rs`
- [ ] Guard inbox processing: reject activities from actors that the local user has blocked, and skip delivery to actors that have blocked the local user

---

## 10. Public Key Cache Persistence

The public key cache is in-memory only; it is lost on restart, causing unnecessary re-fetches.

**Define trait** in `src/federation/key_cache.rs` (extend existing):

```rust
#[async_trait]
pub trait PersistentKeyCache: Send + Sync {
    /// Store a public key PEM with an expiry timestamp.
    async fn store(&self, key_id: &str, pem: &str, expires_at: DateTime<Utc>) -> Result<(), AppError>;
    /// Look up a key by ID; return `None` if absent or expired.
    async fn load(&self, key_id: &str) -> Result<Option<String>, AppError>;
    /// Remove expired entries (for periodic pruning).
    async fn prune_expired(&self) -> Result<u64, AppError>;
}
```

**Impl tasks:**
- [x] Add `public_key_cache` migration (key_id PRIMARY KEY, pem TEXT, expires_at TIMESTAMP)
- [x] Implement `PersistentKeyCache` on `Database`
- [x] Modify `PublicKeyCache` in `key_cache.rs` to fall back to `PersistentKeyCache` on L1 miss and write through on fetch

---

## 11. Grouped Notifications (v2)

`GET /api/v2/notifications` with server-side grouping is not implemented.

**Define trait** in `src/data/repository.rs`:

```rust
#[async_trait]
pub trait GroupedNotificationRepository: Send + Sync {
    /// Return notifications grouped by type + status, with aggregated accounts.
    async fn get_grouped_notifications(
        &self,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
        types: Option<&[&str]>,
        exclude_types: Option<&[&str]>,
    ) -> Result<Vec<NotificationGroup>, AppError>;
}
```

**Impl tasks:**
- [ ] Define `NotificationGroup` struct (`group_key`, `notifications_count`, `type`, `sample_account_ids`, `status_id`, `latest_page_notification_at`)
- [ ] Implement `GroupedNotificationRepository` on `Database` using a `GROUP BY` query
- [ ] Add handler for `GET /api/v2/notifications` and `GET /api/v2/notifications/:group_key/accounts`

---

## Cross-cutting Implementation Notes

- All new traits must be added to `AppState` (in `src/lib.rs` or `src/config.rs`) via `Arc<dyn Trait>`
- All new migrations go in `migrations/` with ascending numeric prefix
- New API route modules are registered in `src/api/mastodon/mod.rs`
- New federation handlers are dispatched from the `match activity.activity_type` block in `src/federation/activity.rs`
