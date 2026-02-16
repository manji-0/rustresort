//! Activity processing
//!
//! Handles incoming ActivityPub activities.

#![allow(dead_code)]

use std::sync::Arc;

use crate::data::{Database, ProfileCache, TimelineCache};
use crate::error::AppError;

/// Return true when a Follow target references the local actor.
///
/// Accepted forms:
/// - `username@domain[:port]`
/// - `acct:username@domain[:port]`
/// - `<protocol>://domain[:port]/users/username` (with optional trailing slash)
/// - `<protocol>://domain[:port]/@username` (with optional trailing slash)
///
/// `protocol` must match the local instance protocol (`http` or `https`).
fn default_port_for_scheme(scheme: &str) -> Option<u16> {
    match scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

fn parse_host_and_port(authority: &str) -> Option<(String, Option<u16>)> {
    let parsed = url::Url::parse(&format!("http://{}", authority)).ok()?;
    let host = parsed.host_str()?.to_string();
    Some((host, parsed.port()))
}

fn extract_follow_target(activity: &serde_json::Value) -> Result<String, AppError> {
    let object = activity
        .get("object")
        .ok_or_else(|| AppError::Validation("Missing object in Follow".to_string()))?;

    object
        .as_str()
        .or_else(|| object.get("id").and_then(|id| id.as_str()))
        .map(str::to_string)
        .ok_or_else(|| AppError::Validation("Invalid object in Follow".to_string()))
}

fn is_local_follow_target(local_address: &str, local_protocol: &str, object: &str) -> bool {
    let object = object.trim();
    if object.is_empty() {
        return false;
    }

    if object.eq_ignore_ascii_case(local_address) {
        return true;
    }

    if object
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("acct:"))
    {
        let acct = &object[5..];
        return acct.eq_ignore_ascii_case(local_address);
    }

    let Some((local_username, local_domain)) = local_address.split_once('@') else {
        return false;
    };
    let Some((local_host, local_port)) = parse_host_and_port(local_domain) else {
        return false;
    };

    let Ok(parsed) = url::Url::parse(object) else {
        return false;
    };
    let local_scheme = if local_protocol.eq_ignore_ascii_case("http") {
        "http"
    } else if local_protocol.eq_ignore_ascii_case("https") {
        "https"
    } else {
        return false;
    };

    if parsed.scheme() != local_scheme {
        return false;
    }

    let Some(host) = parsed.host_str() else {
        return false;
    };

    if !host.eq_ignore_ascii_case(&local_host) {
        return false;
    }

    let port_matches = match local_port {
        Some(port) => parsed.port_or_known_default() == Some(port),
        None => match parsed.port() {
            Some(explicit_port) => default_port_for_scheme(parsed.scheme()) == Some(explicit_port),
            None => true,
        },
    };
    if !port_matches {
        return false;
    }

    let path = parsed.path().trim_end_matches('/');
    path == format!("/users/{}", local_username) || path == format!("/@{}", local_username)
}

/// ActivityPub Activity types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityType {
    Create,
    Update,
    Delete,
    Follow,
    Accept,
    Reject,
    Undo,
    Like,
    Announce,
    Block,
    // Add more as needed
}

impl ActivityType {
    /// Parse activity type from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Create" => Some(Self::Create),
            "Update" => Some(Self::Update),
            "Delete" => Some(Self::Delete),
            "Follow" => Some(Self::Follow),
            "Accept" => Some(Self::Accept),
            "Reject" => Some(Self::Reject),
            "Undo" => Some(Self::Undo),
            "Like" => Some(Self::Like),
            "Announce" => Some(Self::Announce),
            "Block" => Some(Self::Block),
            _ => None,
        }
    }
}

/// Decision on how to handle an activity
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistenceDecision {
    /// Store in database permanently
    Persist,
    /// Store in cache only (volatile)
    CacheOnly,
    /// Don't store
    Ignore,
}

/// Activity processor
///
/// Processes incoming ActivityPub activities from inbox.
pub struct ActivityProcessor {
    db: Arc<Database>,
    timeline_cache: Arc<TimelineCache>,
    profile_cache: Arc<ProfileCache>,
    http_client: Arc<reqwest::Client>,
    /// Local account address for comparison
    local_address: String,
    /// Local instance protocol
    local_protocol: String,
    /// Activity delivery service for sending responses
    delivery: Option<Arc<super::ActivityDelivery>>,
}

impl ActivityProcessor {
    /// Create new activity processor
    pub fn new(
        db: Arc<Database>,
        timeline_cache: Arc<TimelineCache>,
        profile_cache: Arc<ProfileCache>,
        http_client: Arc<reqwest::Client>,
        local_address: String,
        local_protocol: String,
    ) -> Self {
        Self {
            db,
            timeline_cache,
            profile_cache,
            http_client,
            local_address,
            local_protocol,
            delivery: None,
        }
    }

    /// Set activity delivery service
    ///
    /// This allows the processor to send activities (like Accept) in response to incoming activities.
    pub fn with_delivery(mut self, delivery: Arc<super::ActivityDelivery>) -> Self {
        self.delivery = Some(delivery);
        self
    }

    /// Process an incoming activity
    ///
    /// # Arguments
    /// * `activity` - Raw JSON-LD activity
    /// * `actor_uri` - Verified actor URI (from signature)
    ///
    /// # Returns
    /// Ok if processed, Err if rejected
    ///
    /// # Side Effects
    /// - May persist data to DB
    /// - May update caches
    /// - May create notifications
    pub async fn process(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        // 1. Parse activity type
        let activity_type_str = activity
            .get("type")
            .and_then(|t| t.as_str())
            .ok_or_else(|| AppError::Validation("Missing activity type".to_string()))?;

        let activity_type = ActivityType::from_str(activity_type_str).ok_or_else(|| {
            AppError::Validation(format!("Unknown activity type: {}", activity_type_str))
        })?;

        // 2. Check if domain is blocked
        let actor_domain = actor_uri
            .split("://")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .unwrap_or("");

        if self.db.is_domain_blocked(actor_domain).await? {
            return Err(AppError::Forbidden);
        }

        // 3. Dispatch to type-specific handler
        match activity_type {
            ActivityType::Create => self.handle_create(activity, actor_uri).await,
            ActivityType::Update => self.handle_update(activity, actor_uri).await,
            ActivityType::Delete => self.handle_delete(activity, actor_uri).await,
            ActivityType::Follow => self.handle_follow(activity, actor_uri).await,
            ActivityType::Accept => self.handle_accept(activity, actor_uri).await,
            ActivityType::Reject => Ok(()), // Ignore for now
            ActivityType::Undo => self.handle_undo(activity, actor_uri).await,
            ActivityType::Like => self.handle_like(activity, actor_uri).await,
            ActivityType::Announce => self.handle_announce(activity, actor_uri).await,
            ActivityType::Block => Ok(()), // Ignore blocks from remote
        }
    }

    /// Determine how to handle an activity
    ///
    /// Based on activity type and relevance to local user.
    fn decide_persistence(&self, activity: &serde_json::Value) -> PersistenceDecision {
        // Get activity type
        let activity_type = activity
            .get("type")
            .and_then(|t| t.as_str())
            .and_then(ActivityType::from_str);

        match activity_type {
            Some(ActivityType::Follow) => {
                // Follow targeting us -> Persist (creates notification)
                PersistenceDecision::Persist
            }
            Some(ActivityType::Like) => {
                // Like of our status -> Persist (creates notification)
                // The handler will check if it's actually our status
                PersistenceDecision::Persist
            }
            Some(ActivityType::Announce) => {
                // Check if it's a quote boost (has content) or regular boost
                if let Some(object) = activity.get("object") {
                    // Quote boost: Announce activity with embedded Note/Article
                    if object.is_object() && object.get("type").is_some() {
                        // Check if the quote mentions us
                        if self.mentions_local_user(object) {
                            // Quote boost mentioning us -> Persist
                            return PersistenceDecision::Persist;
                        }
                    } else if let Some(object_uri) = object.as_str() {
                        // Regular boost: just a URI reference
                        // Check if it's our status being boosted
                        if self.is_local_status(object_uri) {
                            // Boost of our status -> Persist (creates notification)
                            return PersistenceDecision::Persist;
                        }
                    }
                }
                // Boost of someone else's status -> Ignore
                PersistenceDecision::Ignore
            }
            Some(ActivityType::Create) => {
                // Check if it mentions us or replies to us
                if let Some(object) = activity.get("object") {
                    if self.mentions_local_user(object) {
                        // Create with mention from others -> Persist (notification)
                        return PersistenceDecision::Persist;
                    }
                    // Check if it's a reply to our post
                    if let Some(in_reply_to) = object.get("inReplyTo").and_then(|r| r.as_str()) {
                        if self.is_local_status(in_reply_to) {
                            // Reply to our post -> Persist (notification)
                            return PersistenceDecision::Persist;
                        }
                    }
                    // Create from followee -> CacheOnly (future enhancement)
                    // For now, we don't cache
                }
                PersistenceDecision::Ignore
            }
            Some(ActivityType::Accept) => {
                // Accept of our Follow -> Persist
                PersistenceDecision::Persist
            }
            Some(ActivityType::Undo) => {
                // Undo Follow -> Persist (removes follower)
                PersistenceDecision::Persist
            }
            _ => {
                // Others -> Ignore
                PersistenceDecision::Ignore
            }
        }
    }

    // =========================================================================
    // Activity type handlers
    // =========================================================================

    /// Handle Create activity (new post)
    async fn handle_create(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        // 1. Extract object (Note, etc.)
        let object = activity
            .get("object")
            .ok_or_else(|| AppError::Validation("Missing object in Create".to_string()))?;

        // Get the object type
        let object_type = object
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown");

        // We mainly care about Note objects (posts)
        if object_type != "Note" && object_type != "Article" {
            return Ok(()); // Ignore other object types for now
        }

        // Extract actor address
        let actor_address = self.extract_actor_address(actor_uri);

        // 3. Check for mentions -> create notification
        if self.mentions_local_user(object) {
            // Get the status URI
            let status_uri = object
                .get("id")
                .and_then(|id| id.as_str())
                .map(|s| s.to_string());

            // Create mention notification
            let notification = crate::data::Notification {
                id: crate::data::EntityId::new().0,
                notification_type: "mention".to_string(),
                origin_account_address: actor_address.clone(),
                status_uri,
                read: false,
                created_at: chrono::Utc::now(),
            };

            self.db.insert_notification(&notification).await?;
        }

        // 4. Check if reply to our post -> create notification
        if let Some(in_reply_to) = object.get("inReplyTo").and_then(|r| r.as_str()) {
            if self.is_local_status(in_reply_to) {
                // Get the status URI
                let status_uri = object
                    .get("id")
                    .and_then(|id| id.as_str())
                    .map(|s| s.to_string());

                // Create reply notification (if not already created as mention)
                if !self.mentions_local_user(object) {
                    let notification = crate::data::Notification {
                        id: crate::data::EntityId::new().0,
                        notification_type: "mention".to_string(), // Replies are also mentions
                        origin_account_address: actor_address,
                        status_uri,
                        read: false,
                        created_at: chrono::Utc::now(),
                    };

                    self.db.insert_notification(&notification).await?;
                }
            }
        }

        // 2. Check if from followee -> add to cache (future enhancement)
        // For now, we don't implement timeline caching

        Ok(())
    }

    /// Handle Update activity (profile update)
    async fn handle_update(
        &self,
        _activity: serde_json::Value,
        _actor_uri: &str,
    ) -> Result<(), AppError> {
        // For single-user instance, we mainly care about updates from followees
        // Profile cache updates would go here in a full implementation
        // For now, just accept and ignore
        Ok(())
    }

    /// Handle Delete activity
    async fn handle_delete(
        &self,
        _activity: serde_json::Value,
        _actor_uri: &str,
    ) -> Result<(), AppError> {
        // For single-user instance with minimal persistence,
        // we don't need to track deletions extensively
        // Cache invalidation would go here in a full implementation
        Ok(())
    }

    /// Handle Follow activity
    async fn handle_follow(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        // 1. Verify target is local user
        let target = extract_follow_target(&activity)?;

        // Check if the object references our local actor.
        if !is_local_follow_target(&self.local_address, &self.local_protocol, &target) {
            return Err(AppError::Validation(
                "Follow target is not local user".to_string(),
            ));
        }

        // 2. Get actor's inbox for later Accept delivery
        let inbox_uri = activity
            .get("actor")
            .and_then(|a| {
                if let Some(actor_str) = a.as_str() {
                    Some(format!("{}/inbox", actor_str))
                } else {
                    a.get("inbox")
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string())
                }
            })
            .unwrap_or_else(|| format!("{}/inbox", actor_uri));

        // Extract actor address from URI
        let actor_address = self.extract_actor_address(actor_uri);

        // Get the Follow activity ID
        let follow_activity_uri = activity
            .get("id")
            .and_then(|id| id.as_str())
            .unwrap_or(actor_uri)
            .to_string();

        // 3. Add to followers table
        let follower = crate::data::Follower {
            id: crate::data::EntityId::new().0,
            follower_address: actor_address.clone(),
            inbox_uri: inbox_uri.clone(),
            uri: follow_activity_uri.clone(),
            created_at: chrono::Utc::now(),
        };

        self.db.insert_follower(&follower).await?;

        // 4. Create notification
        let notification = crate::data::Notification {
            id: crate::data::EntityId::new().0,
            notification_type: "follow".to_string(),
            origin_account_address: actor_address,
            status_uri: None,
            read: false,
            created_at: chrono::Utc::now(),
        };

        self.db.insert_notification(&notification).await?;

        // 5. Send Accept activity
        if let Some(ref delivery) = self.delivery {
            match delivery.send_accept(&follow_activity_uri, &inbox_uri).await {
                Ok(_) => {
                    tracing::info!("Successfully sent Accept to {}", inbox_uri);
                }
                Err(e) => {
                    tracing::error!("Failed to send Accept to {}: {}", inbox_uri, e);
                    // Don't fail the whole operation if Accept sending fails
                    // The follower is already added to the database
                }
            }
        } else {
            tracing::warn!("No delivery service configured, cannot send Accept");
        }

        Ok(())
    }

    /// Handle Accept activity (follow accepted)
    async fn handle_accept(
        &self,
        activity: serde_json::Value,
        _actor_uri: &str,
    ) -> Result<(), AppError> {
        // 1. Verify we sent the original Follow
        let object = activity.get("object");

        // The object should be our Follow activity
        // For now, just log that we received an Accept
        tracing::info!("Received Accept activity: {:?}", object);

        // In a full implementation:
        // 2. Mark follow as accepted in DB
        // 3. Fetch actor's recent posts to cache

        Ok(())
    }

    /// Handle Undo activity
    async fn handle_undo(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        // 1. Get the undone activity
        let object = activity.get("object");

        if let Some(obj) = object {
            // Check the type of the undone activity
            if let Some(obj_type) = obj.get("type").and_then(|t| t.as_str()) {
                match obj_type {
                    "Follow" => {
                        // Remove from followers
                        let actor_address = self.extract_actor_address(actor_uri);
                        // Note: We'd need a delete_follower method in DB
                        tracing::info!("Unfollowed by {}", actor_address);
                        Ok(())
                    }
                    "Like" | "Announce" => {
                        // Could remove notification, but for simplicity just ignore
                        Ok(())
                    }
                    _ => Ok(()),
                }
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    /// Handle Like activity
    async fn handle_like(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        // 1. Check if liking our status
        let object = activity
            .get("object")
            .and_then(|o| o.as_str())
            .ok_or_else(|| AppError::Validation("Missing object in Like".to_string()))?;

        // Check if it's a local status
        if !self.is_local_status(object) {
            return Ok(()); // Not our status, ignore
        }

        // Extract actor address
        let actor_address = self.extract_actor_address(actor_uri);

        // 2. Create notification
        let notification = crate::data::Notification {
            id: crate::data::EntityId::new().0,
            notification_type: "favourite".to_string(),
            origin_account_address: actor_address,
            status_uri: Some(object.to_string()),
            read: false,
            created_at: chrono::Utc::now(),
        };

        self.db.insert_notification(&notification).await?;

        Ok(())
    }

    /// Handle Announce activity (boost)
    async fn handle_announce(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        let object = activity
            .get("object")
            .ok_or_else(|| AppError::Validation("Missing object in Announce".to_string()))?;

        // Extract actor address
        let actor_address = self.extract_actor_address(actor_uri);

        // Check if it's a quote boost (embedded object) or regular boost (URI)
        if object.is_object() {
            // Quote boost: Announce with embedded Note/Article
            // Check if the quote mentions us
            if self.mentions_local_user(object) {
                // Get the quote status URI
                let status_uri = object
                    .get("id")
                    .and_then(|id| id.as_str())
                    .map(|s| s.to_string());

                // Create mention notification for quote boost
                let notification = crate::data::Notification {
                    id: crate::data::EntityId::new().0,
                    notification_type: "mention".to_string(),
                    origin_account_address: actor_address,
                    status_uri,
                    read: false,
                    created_at: chrono::Utc::now(),
                };

                self.db.insert_notification(&notification).await?;
            }
            // If quote doesn't mention us, ignore (future: could cache if from followee)
        } else if let Some(object_uri) = object.as_str() {
            // Regular boost: just a URI reference
            // Check if it's our status being boosted
            if self.is_local_status(object_uri) {
                // Create reblog notification for boost of our status
                let notification = crate::data::Notification {
                    id: crate::data::EntityId::new().0,
                    notification_type: "reblog".to_string(),
                    origin_account_address: actor_address,
                    status_uri: Some(object_uri.to_string()),
                    read: false,
                    created_at: chrono::Utc::now(),
                };

                self.db.insert_notification(&notification).await?;
            }
            // If boosting someone else's status, ignore (future: could cache if from followee)
        }

        Ok(())
    }

    // =========================================================================
    // Helpers
    // =========================================================================

    /// Extract actor address from actor URI
    /// Example: https://example.com/users/alice -> alice@example.com
    fn extract_actor_address(&self, actor_uri: &str) -> String {
        // Try to extract domain and username from URI
        if let Some(domain_and_path) = actor_uri.split("://").nth(1) {
            let parts: Vec<&str> = domain_and_path.split('/').collect();
            if parts.len() >= 3 && parts[1] == "users" {
                let domain = parts[0];
                let username = parts[2];
                return format!("{}@{}", username, domain);
            }
        }
        // Fallback: use the full URI as address
        actor_uri.to_string()
    }

    /// Check if activity mentions the local user
    fn mentions_local_user(&self, object: &serde_json::Value) -> bool {
        // Check cc/to/tag for local user URI or address
        let check_array = |arr: &serde_json::Value| -> bool {
            if let Some(items) = arr.as_array() {
                items.iter().any(|item| {
                    if let Some(s) = item.as_str() {
                        s.contains(&self.local_address)
                    } else {
                        false
                    }
                })
            } else {
                false
            }
        };

        // Check 'to' field
        if let Some(to) = object.get("to") {
            if check_array(to) {
                return true;
            }
        }

        // Check 'cc' field
        if let Some(cc) = object.get("cc") {
            if check_array(cc) {
                return true;
            }
        }

        // Check 'tag' field for Mention type
        if let Some(tag) = object.get("tag") {
            if let Some(tags) = tag.as_array() {
                for t in tags {
                    if t.get("type").and_then(|ty| ty.as_str()) == Some("Mention") {
                        if let Some(href) = t.get("href").and_then(|h| h.as_str()) {
                            if href.contains(&self.local_address) {
                                return true;
                            }
                        }
                    }
                }
            }
        }

        false
    }

    /// Check if actor is a followee
    async fn is_followee(&self, actor_uri: &str) -> bool {
        let actor_address = self.extract_actor_address(actor_uri);
        // Check in DB if we follow this actor
        self.db
            .get_all_follow_addresses()
            .await
            .map(|addresses| addresses.contains(&actor_address))
            .unwrap_or(false)
    }

    /// Check if status is by local user
    fn is_local_status(&self, status_uri: &str) -> bool {
        // Check if URI contains local domain/address
        status_uri.contains(&self.local_address)
            || status_uri.contains("/users/")
                && status_uri.split("://").nth(1).map_or(false, |s| {
                    s.split('/')
                        .next()
                        .map_or(false, |domain| self.local_address.ends_with(domain))
                })
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_follow_target, is_local_follow_target};
    use crate::data::{Database, ProfileCache, TimelineCache};
    use crate::error::AppError;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::TempDir;

    async fn create_test_processor(
        local_address: &str,
        local_protocol: &str,
    ) -> (super::ActivityProcessor, Arc<Database>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("activity_processor_test.db");
        let db = Arc::new(Database::connect(&db_path).await.unwrap());
        let timeline_cache = Arc::new(TimelineCache::new(16));
        let profile_cache = Arc::new(ProfileCache::new());
        let http_client = Arc::new(reqwest::Client::new());

        let processor = super::ActivityProcessor::new(
            db.clone(),
            timeline_cache,
            profile_cache,
            http_client,
            local_address.to_string(),
            local_protocol.to_string(),
        );

        (processor, db, temp_dir)
    }

    #[test]
    fn is_local_follow_target_accepts_local_address_forms() {
        let local = "alice@example.com";
        let protocol = "https";

        assert!(is_local_follow_target(local, protocol, "alice@example.com"));
        assert!(is_local_follow_target(
            local,
            protocol,
            "acct:alice@example.com"
        ));
        assert!(is_local_follow_target(
            local,
            protocol,
            "ACCT:ALICE@EXAMPLE.COM"
        ));
    }

    #[test]
    fn is_local_follow_target_accepts_local_actor_uri_forms() {
        let local = "alice@example.com";
        let protocol = "https";

        assert!(is_local_follow_target(
            local,
            protocol,
            "https://example.com/users/alice"
        ));
        assert!(is_local_follow_target(
            local,
            protocol,
            "https://example.com/users/alice/"
        ));
        assert!(is_local_follow_target(
            local,
            protocol,
            "https://example.com/@alice"
        ));
        assert!(is_local_follow_target(
            local,
            protocol,
            "https://example.com:443/users/alice"
        ));
    }

    #[test]
    fn is_local_follow_target_accepts_and_enforces_configured_port() {
        let local = "alice@localhost:3000";
        let protocol = "http";

        assert!(is_local_follow_target(
            local,
            protocol,
            "http://localhost:3000/users/alice"
        ));
        assert!(is_local_follow_target(
            local,
            protocol,
            "http://localhost:3000/@alice/"
        ));
        assert!(!is_local_follow_target(
            local,
            protocol,
            "http://localhost/users/alice"
        ));
        assert!(!is_local_follow_target(
            local,
            protocol,
            "http://localhost:3001/users/alice"
        ));
        assert!(!is_local_follow_target(
            local,
            protocol,
            "https://localhost:3000/users/alice"
        ));
    }

    #[test]
    fn is_local_follow_target_enforces_configured_protocol() {
        let local = "alice@example.com";

        assert!(is_local_follow_target(
            local,
            "http",
            "http://example.com/users/alice"
        ));
        assert!(!is_local_follow_target(
            local,
            "https",
            "http://example.com/users/alice"
        ));
    }

    #[test]
    fn is_local_follow_target_rejects_other_users_or_domains() {
        let local = "alice@example.com";
        let protocol = "https";

        assert!(!is_local_follow_target(
            local,
            protocol,
            "https://example.com/users/bob"
        ));
        assert!(!is_local_follow_target(
            local,
            protocol,
            "https://evil.example/users/alice"
        ));
        assert!(!is_local_follow_target(
            local,
            protocol,
            "https://example.com:8443/users/alice"
        ));
        assert!(!is_local_follow_target(
            local,
            protocol,
            "acct:bob@example.com"
        ));
        assert!(!is_local_follow_target(
            local,
            protocol,
            "ftp://example.com/users/alice"
        ));
        assert!(!is_local_follow_target(
            local,
            protocol,
            "https://example.com/users/ALICE"
        ));
        assert!(!is_local_follow_target(local, protocol, ""));
    }

    #[test]
    fn extract_follow_target_accepts_string_and_object_id_forms() {
        let string_object = json!({
            "object": "https://example.com/users/alice"
        });
        let object_id = json!({
            "object": {
                "id": "https://example.com/users/alice"
            }
        });

        assert_eq!(
            extract_follow_target(&string_object).unwrap(),
            "https://example.com/users/alice"
        );
        assert_eq!(
            extract_follow_target(&object_id).unwrap(),
            "https://example.com/users/alice"
        );
    }

    #[test]
    fn extract_follow_target_rejects_missing_or_invalid_object() {
        let missing = json!({});
        let empty_object = json!({ "object": {} });
        let non_string_id = json!({ "object": { "id": 123 } });

        assert!(extract_follow_target(&missing).is_err());
        assert!(extract_follow_target(&empty_object).is_err());
        assert!(extract_follow_target(&non_string_id).is_err());
    }

    #[tokio::test]
    async fn handle_follow_accepts_object_id_target_for_local_actor() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let activity = json!({
            "type": "Follow",
            "id": "https://remote.example/follows/1",
            "actor": actor_uri,
            "object": {
                "id": "https://example.com/users/alice"
            }
        });

        processor.handle_follow(activity, actor_uri).await.unwrap();
        let follower_addresses = db.get_all_follower_addresses().await.unwrap();
        assert_eq!(follower_addresses, vec!["bob@remote.example".to_string()]);
    }

    #[tokio::test]
    async fn handle_follow_rejects_object_id_target_for_non_local_actor() {
        let (processor, _db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let activity = json!({
            "type": "Follow",
            "id": "https://remote.example/follows/2",
            "actor": actor_uri,
            "object": {
                "id": "https://example.com/users/ALICE"
            }
        });

        let result = processor.handle_follow(activity, actor_uri).await;
        assert!(matches!(result, Err(AppError::Validation(_))));
    }
}
