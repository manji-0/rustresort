//! Activity processing
//!
//! Handles incoming ActivityPub activities.

#![allow(dead_code)]

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::data::{CachedAttachment, CachedStatus, Database, ProfileCache, TimelineCache};
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
    let host = parsed.host_str()?;
    let normalized_host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    Some((normalized_host, parsed.port()))
}

fn format_authority_host(host: &str) -> String {
    let bare_host = host.trim_start_matches('[').trim_end_matches(']');
    if bare_host.contains(':') {
        format!("[{}]", bare_host)
    } else {
        bare_host.to_string()
    }
}

fn push_unique_domain_candidate(candidates: &mut Vec<String>, candidate: String) {
    if !candidate.is_empty() && !candidates.contains(&candidate) {
        candidates.push(candidate);
    }
}

fn append_domain_candidates(candidates: &mut Vec<String>, host: &str, port: Option<u16>) {
    let normalized_host = host.to_ascii_lowercase();
    push_unique_domain_candidate(candidates, normalized_host.clone());

    if normalized_host.contains(':') {
        let bracketed_host = format_authority_host(&normalized_host);
        push_unique_domain_candidate(candidates, bracketed_host.clone());
        if let Some(port) = port {
            push_unique_domain_candidate(candidates, format!("{}:{}", normalized_host, port));
            push_unique_domain_candidate(candidates, format!("{}:{}", bracketed_host, port));
        }
        return;
    }

    if let Some(port) = port {
        push_unique_domain_candidate(candidates, format!("{}:{}", normalized_host, port));
    }
}

fn extract_username_from_actor_path(path: &str) -> Option<&str> {
    let mut parts = path
        .trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty());
    let first_segment = parts.next()?;

    if let Some(username) = first_segment.strip_prefix('@') {
        return (!username.is_empty()).then_some(username);
    }

    if first_segment.eq_ignore_ascii_case("users")
        || first_segment.eq_ignore_ascii_case("accounts")
        || first_segment.eq_ignore_ascii_case("u")
        || first_segment.eq_ignore_ascii_case("profile")
    {
        let username = parts.next()?;
        return (!username.is_empty()).then_some(username);
    }

    None
}

fn parse_account_address(address: &str) -> Option<(String, String, Option<u16>)> {
    let (username, domain) = address.split_once('@')?;
    let (host, port) = parse_host_and_port(domain)?;
    Some((
        username.to_ascii_lowercase(),
        host.to_ascii_lowercase(),
        port,
    ))
}

fn follow_addresses_match(
    actor_address: &str,
    follow_address: &str,
    actor_scheme: Option<&str>,
) -> bool {
    let Some((actor_user, actor_host, actor_port)) = parse_account_address(actor_address) else {
        return actor_address.eq_ignore_ascii_case(follow_address);
    };
    let Some((follow_user, follow_host, follow_port)) = parse_account_address(follow_address)
    else {
        return actor_address.eq_ignore_ascii_case(follow_address);
    };

    if actor_user != follow_user || !actor_host.eq_ignore_ascii_case(&follow_host) {
        return false;
    }

    if let Some(default_port) = actor_scheme.and_then(default_port_for_scheme) {
        return actor_port.unwrap_or(default_port) == follow_port.unwrap_or(default_port);
    }

    actor_port == follow_port
}

fn sanitize_remote_html(content: &str) -> String {
    ammonia::clean(content)
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

fn extract_delete_target_uri(activity: &serde_json::Value) -> Option<String> {
    let object = activity.get("object")?;

    if let Some(uri) = object.as_str() {
        return Some(uri.to_string());
    }

    let is_tombstone = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("Tombstone"));

    if is_tombstone {
        return object
            .get("object")
            .and_then(serde_json::Value::as_str)
            .or_else(|| object.get("id").and_then(serde_json::Value::as_str))
            .map(str::to_string);
    }

    object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| object.get("object").and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

fn actor_domains_for_blocklist(actor_uri: &str) -> Vec<String> {
    let mut candidates = Vec::new();

    if let Ok(parsed) = url::Url::parse(actor_uri) {
        if let Some(host) = parsed.host_str() {
            append_domain_candidates(&mut candidates, host, parsed.port());
        }
        return candidates;
    }

    let Some(authority) = actor_uri
        .split("://")
        .nth(1)
        .and_then(|v| v.split('/').next())
    else {
        return candidates;
    };
    let authority = authority.to_ascii_lowercase();
    push_unique_domain_candidate(&mut candidates, authority.clone());

    if let Some((host, port)) = parse_host_and_port(&authority) {
        append_domain_candidates(&mut candidates, &host, port);
    }

    candidates
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
        let mut actor_is_blocked = false;
        for candidate in actor_domains_for_blocklist(actor_uri) {
            if self.db.is_domain_blocked(&candidate).await? {
                actor_is_blocked = true;
                break;
            }
        }
        if actor_is_blocked {
            return Err(AppError::Forbidden);
        }

        let actor_is_followee = if activity_type == ActivityType::Create {
            self.is_followee(actor_uri).await
        } else {
            false
        };

        // 3. Decide whether this activity should be handled at all.
        let persistence_decision = self.decide_persistence(&activity, actor_is_followee);
        if persistence_decision == PersistenceDecision::Ignore {
            return Ok(());
        }

        // 4. Dispatch to type-specific handler
        match activity_type {
            ActivityType::Create => {
                self.handle_create(activity, actor_uri, persistence_decision)
                    .await
            }
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
    fn decide_persistence(
        &self,
        activity: &serde_json::Value,
        actor_is_followee: bool,
    ) -> PersistenceDecision {
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
                    // Create from followee -> CacheOnly
                    if actor_is_followee {
                        return PersistenceDecision::CacheOnly;
                    }
                }
                PersistenceDecision::Ignore
            }
            Some(ActivityType::Delete) => {
                // Deletes should always be processed.
                // Ownership is verified in handle_delete().
                PersistenceDecision::CacheOnly
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
        persistence_decision: PersistenceDecision,
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
        let should_persist_notification =
            matches!(persistence_decision, PersistenceDecision::Persist);
        let should_cache_status = matches!(persistence_decision, PersistenceDecision::CacheOnly);

        // 3. Check for mentions -> create notification
        if should_persist_notification && self.mentions_local_user(object) {
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
        if should_persist_notification {
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
                            origin_account_address: actor_address.clone(),
                            status_uri,
                            read: false,
                            created_at: chrono::Utc::now(),
                        };

                        self.db.insert_notification(&notification).await?;
                    }
                }
            }
        }

        // 5. Cache followee posts without persisting to DB.
        if should_cache_status {
            if let Some(status_uri) = object.get("id").and_then(|id| id.as_str()) {
                let created_at = object
                    .get("published")
                    .and_then(|published| published.as_str())
                    .and_then(|published| DateTime::parse_from_rfc3339(published).ok())
                    .map(|timestamp| timestamp.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);
                let sanitized_content = sanitize_remote_html(
                    object
                        .get("content")
                        .and_then(|content| content.as_str())
                        .unwrap_or_default(),
                );

                let cached = CachedStatus {
                    id: status_uri.to_string(),
                    uri: status_uri.to_string(),
                    content: sanitized_content,
                    account_address: actor_address,
                    created_at,
                    visibility: self.extract_visibility(object),
                    attachments: self.extract_cached_attachments(object),
                    reply_to_uri: object
                        .get("inReplyTo")
                        .and_then(|reply| reply.as_str())
                        .map(str::to_string),
                    boost_of_uri: None,
                };
                self.timeline_cache.insert(cached).await;
            }
        }

        Ok(())
    }

    /// Handle Update activity (profile update)
    async fn handle_update(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        self.profile_cache
            .update_from_activity(actor_uri, activity)
            .await;
        Ok(())
    }

    /// Handle Delete activity
    async fn handle_delete(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        let deleted_uri = extract_delete_target_uri(&activity);

        if let Some(uri) = deleted_uri {
            let actor_address = self.extract_actor_address(actor_uri);
            let actor_scheme = url::Url::parse(actor_uri)
                .ok()
                .map(|url| url.scheme().to_ascii_lowercase());

            if let Some(cached_status) = self.timeline_cache.get_by_uri(&uri).await {
                if follow_addresses_match(
                    &actor_address,
                    &cached_status.account_address,
                    actor_scheme.as_deref(),
                ) {
                    self.timeline_cache.remove_by_uri(&uri).await;
                } else {
                    tracing::debug!(
                        "Delete actor {} does not own cached status {}, ignoring",
                        actor_address,
                        uri
                    );
                }
            }

            if let Some(status) = self.db.get_status_by_uri(&uri).await? {
                if !status.is_local
                    && follow_addresses_match(
                        &actor_address,
                        &status.account_address,
                        actor_scheme.as_deref(),
                    )
                {
                    self.db.delete_status(&status.id).await?;
                } else if !status.is_local {
                    tracing::debug!(
                        "Delete actor {} does not own persisted status {}, ignoring",
                        actor_address,
                        uri
                    );
                }
            }
        }

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
        let actor_address = self.extract_actor_address(actor_uri);
        let actor_default_port = url::Url::parse(actor_uri)
            .ok()
            .and_then(|url| default_port_for_scheme(url.scheme()));

        // 1. Get the undone activity
        let object = activity.get("object");

        if let Some(obj) = object {
            // Check the type of the undone activity
            if let Some(obj_type) = obj.get("type").and_then(|t| t.as_str()) {
                match obj_type {
                    "Follow" => {
                        let Ok(target) = extract_follow_target(obj) else {
                            tracing::debug!("Undo Follow missing target object, ignoring");
                            return Ok(());
                        };
                        if !is_local_follow_target(
                            &self.local_address,
                            &self.local_protocol,
                            &target,
                        ) {
                            tracing::debug!("Undo Follow target is not local actor, ignoring");
                            return Ok(());
                        }

                        if let Some(follow_uri) = obj.get("id").and_then(|id| id.as_str()) {
                            let removed = self
                                .db
                                .delete_follower_by_address_and_uri(
                                    &actor_address,
                                    follow_uri,
                                    actor_default_port,
                                )
                                .await?;
                            if removed {
                                tracing::info!(
                                    "Unfollowed by {} via Follow activity URI {}",
                                    actor_address,
                                    follow_uri
                                );
                            } else {
                                tracing::debug!(
                                    "Undo Follow id did not match follower row for actor {}, uri {}",
                                    actor_address,
                                    follow_uri
                                );
                            }
                        } else {
                            // Fallback for minimal Undo payloads that omit Follow.id.
                            self.db
                                .delete_follower(&actor_address, actor_default_port)
                                .await?;
                            tracing::info!("Unfollowed by {} via address fallback", actor_address);
                        }
                        Ok(())
                    }
                    "Like" | "Announce" => {
                        // Could remove notification, but for simplicity just ignore
                        Ok(())
                    }
                    _ => Ok(()),
                }
            } else if let Some(follow_uri) = obj.as_str() {
                // Compact Undo representation where object is the Follow activity URI.
                let removed = self
                    .db
                    .delete_follower_by_address_and_uri(
                        &actor_address,
                        follow_uri,
                        actor_default_port,
                    )
                    .await?;
                if removed {
                    tracing::info!(
                        "Unfollowed by {} via follow activity URI {}",
                        actor_address,
                        follow_uri
                    );
                } else {
                    tracing::debug!(
                        "Undo with URI object did not match follower row for actor {}, uri {}",
                        actor_address,
                        follow_uri
                    );
                }
                Ok(())
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

    fn extract_visibility(&self, object: &serde_json::Value) -> String {
        const PUBLIC_AUDIENCE: &str = "https://www.w3.org/ns/activitystreams#Public";

        let contains_public = |audience: &serde_json::Value| -> bool {
            if let Some(value) = audience.as_str() {
                return value == PUBLIC_AUDIENCE;
            }
            audience
                .as_array()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .any(|value| value == PUBLIC_AUDIENCE)
                })
                .unwrap_or(false)
        };

        if object.get("to").is_some_and(contains_public) {
            "public".to_string()
        } else if object.get("cc").is_some_and(contains_public) {
            "unlisted".to_string()
        } else {
            "private".to_string()
        }
    }

    fn extract_cached_attachments(&self, object: &serde_json::Value) -> Vec<CachedAttachment> {
        let mut attachments = Vec::new();

        let Some(values) = object
            .get("attachment")
            .and_then(serde_json::Value::as_array)
        else {
            return attachments;
        };

        for value in values {
            if let Some(url) = value.as_str() {
                attachments.push(CachedAttachment {
                    url: url.to_string(),
                    thumbnail_url: None,
                    content_type: "application/octet-stream".to_string(),
                    description: None,
                    blurhash: None,
                });
                continue;
            }

            let Some(url) = value.get("url").and_then(serde_json::Value::as_str) else {
                continue;
            };

            attachments.push(CachedAttachment {
                url: url.to_string(),
                thumbnail_url: None,
                content_type: value
                    .get("mediaType")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("application/octet-stream")
                    .to_string(),
                description: value
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                blurhash: None,
            });
        }

        attachments
    }

    /// Extract actor address from actor URI
    /// Example: https://example.com/users/alice -> alice@example.com
    fn extract_actor_address(&self, actor_uri: &str) -> String {
        if let Ok(parsed) = url::Url::parse(actor_uri) {
            if let Some(host) = parsed.host_str() {
                let normalized_host = host.to_ascii_lowercase();
                let authority_host = format_authority_host(&normalized_host);
                let normalized_port = parsed.port();
                let domain = match normalized_port {
                    Some(port) => format!("{}:{}", authority_host, port),
                    None => authority_host,
                };

                if let Some(username) = extract_username_from_actor_path(parsed.path()) {
                    return format!("{}@{}", username.to_ascii_lowercase(), domain);
                }
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
        let actor_scheme = url::Url::parse(actor_uri)
            .ok()
            .map(|url| url.scheme().to_ascii_lowercase());
        // Check in DB if we follow this actor
        self.db
            .get_all_follow_addresses()
            .await
            .map(|addresses| {
                addresses.iter().any(|address| {
                    follow_addresses_match(&actor_address, address, actor_scheme.as_deref())
                })
            })
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
    use crate::data::{
        CachedProfile, CachedStatus, Database, EntityId, Follow, Follower, ProfileCache,
        TimelineCache,
    };
    use crate::error::AppError;
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::TempDir;

    const TEST_PRIVATE_KEY_PEM: &str = include_str!("../../tests/fixtures/test_private_key.pem");

    async fn create_test_processor_with_timeline_and_profile(
        local_address: &str,
        local_protocol: &str,
    ) -> (
        super::ActivityProcessor,
        Arc<Database>,
        Arc<TimelineCache>,
        Arc<ProfileCache>,
        TempDir,
    ) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("activity_processor_test.db");
        let db = Arc::new(Database::connect(&db_path).await.unwrap());
        let timeline_cache = Arc::new(TimelineCache::new(16).await.unwrap());
        let profile_cache = Arc::new(ProfileCache::new(86400).await.unwrap());
        let http_client = Arc::new(reqwest::Client::new());

        let processor = super::ActivityProcessor::new(
            db.clone(),
            timeline_cache.clone(),
            profile_cache.clone(),
            http_client,
            local_address.to_string(),
            local_protocol.to_string(),
        );

        (processor, db, timeline_cache, profile_cache, temp_dir)
    }

    async fn create_test_processor_with_timeline(
        local_address: &str,
        local_protocol: &str,
    ) -> (
        super::ActivityProcessor,
        Arc<Database>,
        Arc<TimelineCache>,
        TempDir,
    ) {
        let (processor, db, timeline_cache, _profile_cache, temp_dir) =
            create_test_processor_with_timeline_and_profile(local_address, local_protocol).await;
        (processor, db, timeline_cache, temp_dir)
    }

    async fn create_test_processor(
        local_address: &str,
        local_protocol: &str,
    ) -> (super::ActivityProcessor, Arc<Database>, TempDir) {
        let (processor, db, _timeline_cache, temp_dir) =
            create_test_processor_with_timeline(local_address, local_protocol).await;
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
    async fn handle_follow_sends_accept_when_delivery_is_configured() {
        use axum::{Router, routing::post};
        use http::StatusCode;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::net::TcpListener;

        let deliveries = Arc::new(AtomicUsize::new(0));
        let deliveries_for_route = deliveries.clone();
        let app = Router::new().route(
            "/users/bob/inbox",
            post(move || {
                let deliveries = deliveries_for_route.clone();
                async move {
                    deliveries.fetch_add(1, Ordering::SeqCst);
                    StatusCode::ACCEPTED
                }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let (processor, _db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let delivery = Arc::new(crate::federation::ActivityDelivery::new(
            Arc::new(reqwest::Client::new()),
            "https://example.com/users/alice".to_string(),
            "https://example.com/users/alice#main-key".to_string(),
            TEST_PRIVATE_KEY_PEM.to_string(),
        ));
        let processor = processor.with_delivery(delivery);

        let actor_uri = format!("http://{addr}/users/bob");
        let activity = json!({
            "type": "Follow",
            "id": format!("{actor_uri}/follows/1"),
            "actor": actor_uri,
            "object": "https://example.com/users/alice"
        });

        processor.handle_follow(activity, &actor_uri).await.unwrap();

        assert_eq!(deliveries.load(Ordering::SeqCst), 1);
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

    #[tokio::test]
    async fn handle_update_applies_profile_cache_updates() {
        let (processor, _db, _timeline_cache, profile_cache, _temp_dir) =
            create_test_processor_with_timeline_and_profile("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        profile_cache
            .insert(CachedProfile {
                address: "bob@remote.example".to_string(),
                uri: actor_uri.to_string(),
                display_name: Some("Bob".to_string()),
                note: Some("before".to_string()),
                avatar_url: None,
                header_url: None,
                public_key_pem: "old-key".to_string(),
                inbox_uri: "https://remote.example/inbox-old".to_string(),
                outbox_uri: Some("https://remote.example/outbox-old".to_string()),
                followers_count: Some(1),
                following_count: Some(2),
                fetched_at: Utc::now(),
            })
            .await;

        let activity = json!({
            "type": "Update",
            "actor": actor_uri,
            "object": {
                "id": actor_uri,
                "name": "Bob Updated",
                "summary": "after",
                "publicKey": {
                    "publicKeyPem": "new-key"
                },
                "inbox": "https://remote.example/inbox-new",
                "followersCount": 10,
                "followingCount": 20
            }
        });

        processor.handle_update(activity, actor_uri).await.unwrap();

        let updated = profile_cache
            .get("bob@remote.example")
            .await
            .expect("profile should exist");
        assert_eq!(updated.display_name.as_deref(), Some("Bob Updated"));
        assert_eq!(updated.note.as_deref(), Some("after"));
        assert_eq!(updated.public_key_pem, "new-key");
        assert_eq!(updated.inbox_uri, "https://remote.example/inbox-new");
        assert_eq!(updated.followers_count, Some(10));
        assert_eq!(updated.following_count, Some(20));
    }

    #[tokio::test]
    async fn process_rejects_blocked_domain_when_actor_uri_has_explicit_default_port() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        db.block_domain("remote.example").await.unwrap();

        let actor_uri = "https://remote.example:443/users/bob";
        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "id": "https://remote.example/statuses/blocked",
                "content": "<p>blocked</p>",
                "published": "2026-01-01T00:00:00Z"
            }
        });

        let result = processor.process(activity, actor_uri).await;
        assert!(matches!(result, Err(AppError::Forbidden)));
    }

    #[tokio::test]
    async fn process_rejects_blocked_domain_with_explicit_non_default_port_entry() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        db.block_domain("remote.example:8443").await.unwrap();

        let actor_uri = "https://remote.example:8443/users/bob";
        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "id": "https://remote.example:8443/statuses/blocked",
                "content": "<p>blocked</p>",
                "published": "2026-01-01T00:00:00Z"
            }
        });

        let result = processor.process(activity, actor_uri).await;
        assert!(matches!(result, Err(AppError::Forbidden)));
    }

    #[tokio::test]
    async fn process_undo_follow_without_id_removes_follower() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";

        let follower = Follower {
            id: EntityId::new().0,
            follower_address: "bob@remote.example".to_string(),
            inbox_uri: "https://remote.example/users/bob/inbox".to_string(),
            uri: "https://remote.example/follows/1".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follower(&follower).await.unwrap();

        let activity = json!({
            "type": "Undo",
            "actor": actor_uri,
            "object": {
                "type": "Follow",
                "object": "https://example.com/users/alice"
            }
        });

        processor.process(activity, actor_uri).await.unwrap();
        let follower_addresses = db.get_all_follower_addresses().await.unwrap();
        assert!(!follower_addresses.contains(&"bob@remote.example".to_string()));
    }

    #[tokio::test]
    async fn process_undo_follow_without_id_removes_follower_for_default_https_port_variant() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example:443/users/bob";

        let follower = Follower {
            id: EntityId::new().0,
            follower_address: "bob@remote.example".to_string(),
            inbox_uri: "https://remote.example/users/bob/inbox".to_string(),
            uri: "https://remote.example/follows/no-id-port-variant".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follower(&follower).await.unwrap();

        let activity = json!({
            "type": "Undo",
            "actor": actor_uri,
            "object": {
                "type": "Follow",
                "object": "https://example.com/users/alice"
            }
        });

        processor.process(activity, actor_uri).await.unwrap();
        let follower_addresses = db.get_all_follower_addresses().await.unwrap();
        assert!(follower_addresses.is_empty());
    }

    #[tokio::test]
    async fn process_undo_follow_removes_mixed_case_follower_address() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";

        let follower = Follower {
            id: EntityId::new().0,
            follower_address: "Bob@Remote.Example".to_string(),
            inbox_uri: "https://remote.example/users/bob/inbox".to_string(),
            uri: "https://remote.example/follows/mixed-case".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follower(&follower).await.unwrap();

        let activity = json!({
            "type": "Undo",
            "actor": actor_uri,
            "object": {
                "type": "Follow",
                "id": "https://remote.example/follows/mixed-case",
                "object": "https://example.com/users/alice"
            }
        });

        processor.process(activity, actor_uri).await.unwrap();
        let follower_addresses = db.get_all_follower_addresses().await.unwrap();
        assert!(follower_addresses.is_empty());
    }

    #[tokio::test]
    async fn process_undo_follow_with_uri_object_removes_matching_follower() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let follow_uri = "https://remote.example/follows/uri-form";

        let follower = Follower {
            id: EntityId::new().0,
            follower_address: "bob@remote.example".to_string(),
            inbox_uri: "https://remote.example/users/bob/inbox".to_string(),
            uri: follow_uri.to_string(),
            created_at: Utc::now(),
        };
        db.insert_follower(&follower).await.unwrap();

        let activity = json!({
            "type": "Undo",
            "actor": actor_uri,
            "object": follow_uri
        });

        processor.process(activity, actor_uri).await.unwrap();
        let follower_addresses = db.get_all_follower_addresses().await.unwrap();
        assert!(follower_addresses.is_empty());
    }

    #[tokio::test]
    async fn process_undo_follow_with_mismatched_follow_id_keeps_follower() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";

        let follower = Follower {
            id: EntityId::new().0,
            follower_address: "bob@remote.example".to_string(),
            inbox_uri: "https://remote.example/users/bob/inbox".to_string(),
            uri: "https://remote.example/follows/current".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follower(&follower).await.unwrap();

        let activity = json!({
            "type": "Undo",
            "actor": actor_uri,
            "object": {
                "type": "Follow",
                "id": "https://remote.example/follows/old",
                "object": "https://example.com/users/alice"
            }
        });

        processor.process(activity, actor_uri).await.unwrap();
        let follower_addresses = db.get_all_follower_addresses().await.unwrap();
        assert_eq!(follower_addresses, vec!["bob@remote.example".to_string()]);
    }

    #[tokio::test]
    async fn process_undo_follow_with_non_local_target_keeps_follower() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";

        let follower = Follower {
            id: EntityId::new().0,
            follower_address: "bob@remote.example".to_string(),
            inbox_uri: "https://remote.example/users/bob/inbox".to_string(),
            uri: "https://remote.example/follows/2".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follower(&follower).await.unwrap();

        let activity = json!({
            "type": "Undo",
            "actor": actor_uri,
            "object": {
                "type": "Follow",
                "object": "https://example.net/users/alice"
            }
        });

        processor.process(activity, actor_uri).await.unwrap();
        let follower_addresses = db.get_all_follower_addresses().await.unwrap();
        assert!(follower_addresses.contains(&"bob@remote.example".to_string()));
    }

    #[tokio::test]
    async fn process_create_from_followee_caches_status_without_db_persist_case_insensitive_match()
    {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/1";

        let follow = Follow {
            id: EntityId::new().0,
            target_address: "Bob@Remote.Example".to_string(),
            uri: "https://example.com/users/alice/follow/1".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "id": status_uri,
                "content": "<p>Hello from followee</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());
        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn process_create_from_followee_with_default_https_port_actor_uri_caches_status() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example:443/users/bob";
        let status_uri = "https://remote.example:443/users/bob/statuses/port-normalized";

        let follow = Follow {
            id: EntityId::new().0,
            target_address: "bob@remote.example".to_string(),
            uri: "https://example.com/users/alice/follow/port-normalized".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "id": status_uri,
                "content": "<p>Hello from :443 actor URI</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());
        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn process_create_from_followee_with_explicit_default_port_follow_address_caches_status()
    {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/port-follow-row";

        let follow = Follow {
            id: EntityId::new().0,
            target_address: "bob@remote.example:443".to_string(),
            uri: "https://example.com/users/alice/follow/port-follow-row".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "id": status_uri,
                "content": "<p>Hello from explicit :443 follow row</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());
        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn process_create_from_followee_with_at_actor_uri_caches_status() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/@bob";
        let status_uri = "https://remote.example/@bob/statuses/2";

        let follow = Follow {
            id: EntityId::new().0,
            target_address: "bob@remote.example".to_string(),
            uri: "https://example.com/users/alice/follow/2".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "id": status_uri,
                "content": "<p>Hello from @bob</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());
        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn process_create_from_followee_with_accounts_actor_uri_caches_status() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/accounts/bob";
        let status_uri = "https://remote.example/accounts/bob/statuses/3";

        let follow = Follow {
            id: EntityId::new().0,
            target_address: "bob@remote.example".to_string(),
            uri: "https://example.com/users/alice/follow/3".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "id": status_uri,
                "content": "<p>Hello from /accounts/bob</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());
        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn process_create_from_followee_with_ipv6_actor_uri_caches_status() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://[2001:db8::1]/users/bob";
        let status_uri = "https://[2001:db8::1]/users/bob/statuses/ipv6";

        let follow = Follow {
            id: EntityId::new().0,
            target_address: "bob@[2001:db8::1]".to_string(),
            uri: "https://example.com/users/alice/follow/ipv6".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "id": status_uri,
                "content": "<p>Hello from IPv6 actor</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());
        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn process_create_from_followee_sanitizes_cached_content() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/sanitized";

        let follow = Follow {
            id: EntityId::new().0,
            target_address: "bob@remote.example".to_string(),
            uri: "https://example.com/users/alice/follow/sanitized".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "id": status_uri,
                "content": "<p>Hello</p><script>alert(1)</script><a href=\"javascript:alert(2)\">click</a>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        let cached = timeline_cache
            .get_by_uri(status_uri)
            .await
            .expect("cached status should exist");
        let lowered = cached.content.to_ascii_lowercase();
        assert!(cached.content.contains("<p>Hello</p>"));
        assert!(!lowered.contains("<script"));
        assert!(!lowered.contains("javascript:"));
    }

    #[tokio::test]
    async fn process_delete_from_followee_removes_cached_status() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/3";

        let follow = Follow {
            id: EntityId::new().0,
            target_address: "bob@remote.example".to_string(),
            uri: "https://example.com/users/alice/follow/3".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let create_activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "id": status_uri,
                "content": "<p>To be deleted</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });
        processor.process(create_activity, actor_uri).await.unwrap();
        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());

        let delete_activity = json!({
            "type": "Delete",
            "actor": actor_uri,
            "object": status_uri
        });
        processor.process(delete_activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_none());
    }

    #[tokio::test]
    async fn process_delete_removes_cached_status_when_cache_id_differs_from_uri() {
        let (processor, _db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/cache-key-mismatch";

        timeline_cache
            .insert(CachedStatus {
                id: "cache-entry-1".to_string(),
                uri: status_uri.to_string(),
                content: "<p>Cached only</p>".to_string(),
                account_address: "bob@remote.example".to_string(),
                created_at: Utc::now(),
                visibility: "public".to_string(),
                attachments: vec![],
                reply_to_uri: None,
                boost_of_uri: None,
            })
            .await;
        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());

        let delete_activity = json!({
            "type": "Delete",
            "actor": actor_uri,
            "object": status_uri
        });
        processor.process(delete_activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_none());
    }

    #[tokio::test]
    async fn process_delete_from_followee_removes_persisted_remote_status() {
        let (processor, db, _timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/4";

        let follow = Follow {
            id: EntityId::new().0,
            target_address: "bob@remote.example".to_string(),
            uri: "https://example.com/users/alice/follow/4".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let status = crate::data::Status {
            id: EntityId::new().0,
            uri: status_uri.to_string(),
            content: "<p>Persisted remote status</p>".to_string(),
            content_warning: None,
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: "bob@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: "bookmarked".to_string(),
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        };
        db.insert_status(&status).await.unwrap();

        let delete_activity = json!({
            "type": "Delete",
            "actor": actor_uri,
            "object": status_uri
        });
        processor.process(delete_activity, actor_uri).await.unwrap();

        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn process_delete_does_not_remove_persisted_status_owned_by_another_actor() {
        let (processor, db, _timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://another.example/users/alice/statuses/5";

        let follow = Follow {
            id: EntityId::new().0,
            target_address: "bob@remote.example".to_string(),
            uri: "https://example.com/users/alice/follow/5".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let status = crate::data::Status {
            id: EntityId::new().0,
            uri: status_uri.to_string(),
            content: "<p>Owned by another actor</p>".to_string(),
            content_warning: None,
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: "alice@another.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: "bookmarked".to_string(),
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        };
        db.insert_status(&status).await.unwrap();

        let delete_activity = json!({
            "type": "Delete",
            "actor": actor_uri,
            "object": status_uri
        });
        processor.process(delete_activity, actor_uri).await.unwrap();

        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn process_delete_without_follow_row_still_removes_owned_persisted_status() {
        let (processor, db, _timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/6";

        let status = crate::data::Status {
            id: EntityId::new().0,
            uri: status_uri.to_string(),
            content: "<p>Persisted remote status after unfollow</p>".to_string(),
            content_warning: None,
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: "bob@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: "favourited".to_string(),
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        };
        db.insert_status(&status).await.unwrap();

        let delete_activity = json!({
            "type": "Delete",
            "actor": actor_uri,
            "object": status_uri
        });
        processor.process(delete_activity, actor_uri).await.unwrap();

        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn process_delete_tombstone_object_field_removes_persisted_remote_status() {
        let (processor, db, _timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/tombstone";

        let status = crate::data::Status {
            id: EntityId::new().0,
            uri: status_uri.to_string(),
            content: "<p>Persisted remote status</p>".to_string(),
            content_warning: None,
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: "bob@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: "bookmarked".to_string(),
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        };
        db.insert_status(&status).await.unwrap();

        let delete_activity = json!({
            "type": "Delete",
            "actor": actor_uri,
            "object": {
                "type": "Tombstone",
                "id": "https://remote.example/tombstones/1",
                "object": status_uri
            }
        });
        processor.process(delete_activity, actor_uri).await.unwrap();

        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn process_delete_with_ipv6_actor_uri_removes_owned_persisted_status() {
        let (processor, db, _timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://[2001:db8::1]/users/bob";
        let status_uri = "https://[2001:db8::1]/users/bob/statuses/owned";

        let status = crate::data::Status {
            id: EntityId::new().0,
            uri: status_uri.to_string(),
            content: "<p>Owned by IPv6 actor</p>".to_string(),
            content_warning: None,
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: "bob@[2001:db8::1]".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: "bookmarked".to_string(),
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        };
        db.insert_status(&status).await.unwrap();

        let delete_activity = json!({
            "type": "Delete",
            "actor": actor_uri,
            "object": status_uri
        });
        processor.process(delete_activity, actor_uri).await.unwrap();

        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn process_delete_removes_cached_status_owned_by_default_https_port_variant() {
        let (processor, _db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example:443/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/7";

        timeline_cache
            .insert(CachedStatus {
                id: "cache-entry-7".to_string(),
                uri: status_uri.to_string(),
                content: "<p>Owned by bob without explicit port</p>".to_string(),
                account_address: "bob@remote.example".to_string(),
                created_at: Utc::now(),
                visibility: "public".to_string(),
                attachments: vec![],
                reply_to_uri: None,
                boost_of_uri: None,
            })
            .await;
        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());

        let delete_activity = json!({
            "type": "Delete",
            "actor": actor_uri,
            "object": status_uri
        });
        processor.process(delete_activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_none());
    }

    #[tokio::test]
    async fn process_delete_removes_persisted_status_owned_by_default_https_port_variant() {
        let (processor, db, _timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example:443/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/8";

        let status = crate::data::Status {
            id: EntityId::new().0,
            uri: status_uri.to_string(),
            content: "<p>Owned by bob without explicit port</p>".to_string(),
            content_warning: None,
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: "bob@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: "bookmarked".to_string(),
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        };
        db.insert_status(&status).await.unwrap();

        let delete_activity = json!({
            "type": "Delete",
            "actor": actor_uri,
            "object": status_uri
        });
        processor.process(delete_activity, actor_uri).await.unwrap();

        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_none());
    }
}
