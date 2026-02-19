//! Activity delivery
//!
//! Handles delivering activities to remote servers.

#![allow(dead_code)]

use std::sync::Arc;

use crate::error::AppError;

/// Activity delivery service
///
/// Sends activities to remote inbox endpoints.
#[derive(Clone)]
pub struct ActivityDelivery {
    http_client: Arc<reqwest::Client>,
    /// Local actor URI
    actor_uri: String,
    /// Key ID for signatures
    key_id: String,
    /// Private key for signing
    private_key_pem: String,
}

/// Deduplicate identical inbox URIs while keeping distinct personal inboxes.
///
/// This preserves recipients on the same domain that use different inbox paths.
fn unique_inbox_targets(inbox_uris: Vec<String>) -> Vec<String> {
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    let mut targets = Vec::new();

    for inbox_uri in inbox_uris {
        if seen.contains(&inbox_uri) {
            continue;
        }
        seen.insert(inbox_uri.clone());
        targets.push(inbox_uri);
    }

    targets
}

fn audience_for_visibility(actor_uri: &str, visibility: &str) -> (Vec<String>, Vec<String>) {
    let public_audience = "https://www.w3.org/ns/activitystreams#Public".to_string();
    let followers_audience = format!("{}/followers", actor_uri);

    match visibility {
        "public" => (vec![public_audience], vec![followers_audience]),
        "unlisted" => (vec![followers_audience], vec![public_audience]),
        "private" => (vec![followers_audience], Vec::new()),
        "direct" => (Vec::new(), Vec::new()),
        _ => (vec![public_audience], vec![followers_audience]),
    }
}

fn build_undo_object(
    activity_uri: &str,
    activity_type: Option<&str>,
    activity_object: Option<&str>,
) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    object.insert("id".to_string(), serde_json::json!(activity_uri));
    if let Some(activity_type) = activity_type {
        object.insert("type".to_string(), serde_json::json!(activity_type));
    }
    if let Some(activity_object) = activity_object {
        object.insert("object".to_string(), serde_json::json!(activity_object));
    }
    serde_json::Value::Object(object)
}

impl ActivityDelivery {
    /// Create new delivery service
    pub fn new(
        http_client: Arc<reqwest::Client>,
        actor_uri: String,
        key_id: String,
        private_key_pem: String,
    ) -> Self {
        Self {
            http_client,
            actor_uri,
            key_id,
            private_key_pem,
        }
    }

    /// Deliver activity to a single inbox
    ///
    /// # Arguments
    /// * `inbox_uri` - Target inbox URL
    /// * `activity` - Activity JSON
    ///
    /// # Errors
    /// Returns error if delivery fails (network, signature, rejection)
    pub async fn deliver_to_inbox(
        &self,
        inbox_uri: &str,
        activity: serde_json::Value,
    ) -> Result<(), AppError> {
        // 1. Serialize activity
        let body = serde_json::to_vec(&activity)
            .map_err(|e| AppError::Validation(format!("Failed to serialize activity: {}", e)))?;

        // 2. Sign request
        let sig_headers = crate::federation::sign_request(
            "POST",
            inbox_uri,
            Some(&body),
            &self.private_key_pem,
            &self.key_id,
        )?;

        // 3. POST to inbox with signed headers
        let mut request = self
            .http_client
            .post(inbox_uri)
            .header("Content-Type", "application/activity+json")
            .header("Date", sig_headers.date)
            .header("Signature", sig_headers.signature);

        if let Some(digest) = sig_headers.digest {
            request = request.header("Digest", digest);
        }

        let response = request.body(body).send().await.map_err(|e| {
            AppError::Federation(format!("Failed to deliver to {}: {}", inbox_uri, e))
        })?;

        // 4. Handle response
        if !response.status().is_success() {
            return Err(AppError::Federation(format!(
                "Inbox {} rejected activity: HTTP {}",
                inbox_uri,
                response.status()
            )));
        }

        tracing::info!("Successfully delivered activity to {}", inbox_uri);
        Ok(())
    }

    /// Deliver activity to all followers
    ///
    /// # Arguments
    /// * `activity` - Activity JSON
    /// * `inbox_uris` - List of follower inbox URIs
    ///
    /// # Note
    /// Deduplicates identical inbox URIs while preserving distinct inbox paths.
    pub async fn deliver_to_followers(
        &self,
        activity: serde_json::Value,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        use tokio::sync::Semaphore;

        // 1. Deduplicate exact inbox URIs only.
        // Grouping by domain can drop recipients that have distinct personal inboxes.
        let total_targets = inbox_uris.len();
        let delivery_targets = unique_inbox_targets(inbox_uris);

        tracing::info!(
            "Delivering to {} unique inboxes (deduplicated from {} total)",
            delivery_targets.len(),
            total_targets
        );

        // 3. Deliver in parallel with concurrency limit
        const MAX_CONCURRENT: usize = 10;
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
        let activity = Arc::new(activity);

        let mut tasks = Vec::new();

        for inbox_uri in delivery_targets {
            let semaphore = semaphore.clone();
            let activity = activity.clone();
            let self_clone = self.clone();

            let task = tokio::spawn(async move {
                // Acquire semaphore permit
                let _permit = semaphore.acquire().await.unwrap();

                // Attempt delivery
                let result = self_clone
                    .deliver_to_inbox(&inbox_uri, (*activity).clone())
                    .await;

                DeliveryResult {
                    inbox_uri: inbox_uri.clone(),
                    success: result.is_ok(),
                    error: result.err().map(|e| e.to_string()),
                    status_code: None, // Could be extracted from error
                }
            });

            tasks.push(task);
        }

        // 4. Collect results
        let mut results = Vec::new();
        for task in tasks {
            if let Ok(result) = task.await {
                results.push(result);
            }
        }

        // Log summary
        let success_count = results.iter().filter(|r| r.success).count();
        let failure_count = results.len() - success_count;

        tracing::info!(
            "Batch delivery complete: {} succeeded, {} failed",
            success_count,
            failure_count
        );

        results
    }

    /// Send Follow activity
    ///
    /// # Arguments
    /// * `target_actor_uri` - Actor to follow
    /// * `target_inbox_uri` - Target's inbox
    pub async fn send_follow(
        &self,
        target_actor_uri: &str,
        target_inbox_uri: &str,
    ) -> Result<String, AppError> {
        // 1. Generate Follow activity with ID
        let follow_id = format!(
            "{}/follow/{}",
            self.actor_uri,
            crate::data::EntityId::new().0
        );

        self.send_follow_with_id(&follow_id, target_actor_uri, target_inbox_uri)
            .await?;

        // 3. Return activity URI
        Ok(follow_id)
    }

    /// Send Follow activity with explicit activity URI.
    pub async fn send_follow_with_id(
        &self,
        follow_activity_uri: &str,
        target_actor_uri: &str,
        target_inbox_uri: &str,
    ) -> Result<(), AppError> {
        let activity = builder::follow(follow_activity_uri, &self.actor_uri, target_actor_uri);

        self.deliver_to_inbox(target_inbox_uri, activity).await?;

        tracing::info!(
            "Sent Follow {} to {} for {}",
            follow_activity_uri,
            target_inbox_uri,
            target_actor_uri
        );

        Ok(())
    }

    /// Send Accept activity (for follow request)
    ///
    /// # Arguments
    /// * `follow_activity_uri` - Original Follow activity URI
    /// * `follower_inbox_uri` - Follower's inbox
    pub async fn send_accept(
        &self,
        follow_activity_uri: &str,
        follower_inbox_uri: &str,
    ) -> Result<(), AppError> {
        // 1. Generate Accept activity wrapping Follow
        let accept_id = format!(
            "{}/accept/{}",
            self.actor_uri,
            crate::data::EntityId::new().0
        );

        let activity = builder::accept(
            &accept_id,
            &self.actor_uri,
            serde_json::json!({
                "type": "Follow",
                "id": follow_activity_uri
            }),
        );

        // 2. Deliver to inbox
        self.deliver_to_inbox(follower_inbox_uri, activity).await?;

        tracing::info!(
            "Sent Accept to {} for Follow {}",
            follower_inbox_uri,
            follow_activity_uri
        );
        Ok(())
    }

    /// Send Reject activity (for follow request rejection)
    pub async fn send_reject(
        &self,
        follow_activity_uri: &str,
        follower_inbox_uri: &str,
    ) -> Result<(), AppError> {
        let reject_id = format!(
            "{}/reject/{}",
            self.actor_uri,
            crate::data::EntityId::new().0
        );

        let activity = builder::reject(
            &reject_id,
            &self.actor_uri,
            serde_json::json!({
                "type": "Follow",
                "id": follow_activity_uri
            }),
        );

        self.deliver_to_inbox(follower_inbox_uri, activity).await?;

        tracing::info!(
            "Sent Reject to {} for Follow {}",
            follower_inbox_uri,
            follow_activity_uri
        );

        Ok(())
    }

    /// Send Create activity (for new status)
    ///
    /// # Arguments
    /// * `status` - Status to create
    /// * `inbox_uris` - Target inboxes
    pub async fn send_create(
        &self,
        status: &crate::data::Status,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        let (to_audience, cc_audience) =
            audience_for_visibility(&self.actor_uri, status.visibility.as_str());
        let note_to: Vec<&str> = to_audience.iter().map(String::as_str).collect();
        let note_cc: Vec<&str> = cc_audience.iter().map(String::as_str).collect();

        // 1. Build Note object
        let note = if let Some(ref in_reply_to) = status.in_reply_to_uri {
            builder::note_reply(
                &status.uri,
                &self.actor_uri,
                &status.content,
                &status.created_at.to_rfc3339(),
                in_reply_to,
                note_to.clone(),
                note_cc.clone(),
            )
        } else {
            builder::note(
                &status.uri,
                &self.actor_uri,
                &status.content,
                &status.created_at.to_rfc3339(),
                note_to.clone(),
                note_cc.clone(),
            )
        };

        // 2. Wrap in Create activity
        let create_id = format!(
            "{}/create/{}",
            self.actor_uri,
            crate::data::EntityId::new().0
        );
        let activity = builder::create(&create_id, &self.actor_uri, note, note_to, note_cc);

        // 3. Deliver to inboxes
        self.deliver_to_followers(activity, inbox_uris).await
    }

    /// Send Delete activity
    pub async fn send_delete(
        &self,
        object_uri: &str,
        object_visibility: &str,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        // Build and deliver Delete activity
        let delete_id = format!(
            "{}/delete/{}",
            self.actor_uri,
            crate::data::EntityId::new().0
        );
        let (to_audience, cc_audience) =
            audience_for_visibility(&self.actor_uri, object_visibility);
        let activity = builder::delete(
            &delete_id,
            &self.actor_uri,
            object_uri,
            to_audience.iter().map(String::as_str).collect(),
            cc_audience.iter().map(String::as_str).collect(),
        );

        self.deliver_to_followers(activity, inbox_uris).await
    }

    /// Send Like activity
    pub async fn send_like(
        &self,
        status_uri: &str,
        target_inbox_uri: &str,
    ) -> Result<String, AppError> {
        // Build and deliver Like activity
        let like_id = format!("{}/like/{}", self.actor_uri, crate::data::EntityId::new().0);
        self.send_like_with_id(&like_id, status_uri, target_inbox_uri)
            .await?;
        Ok(like_id)
    }

    /// Send Like activity with explicit activity URI.
    pub async fn send_like_with_id(
        &self,
        like_activity_uri: &str,
        status_uri: &str,
        target_inbox_uri: &str,
    ) -> Result<(), AppError> {
        let activity = builder::like(like_activity_uri, &self.actor_uri, status_uri);

        self.deliver_to_inbox(target_inbox_uri, activity).await?;

        tracing::info!(
            "Sent Like {} to {} for {}",
            like_activity_uri,
            target_inbox_uri,
            status_uri
        );

        Ok(())
    }

    /// Send Undo activity
    pub async fn send_undo(
        &self,
        activity_uri: &str,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        self.send_undo_with_type(activity_uri, None, inbox_uris)
            .await
    }

    /// Send Undo activity with optional object type.
    pub async fn send_undo_with_type(
        &self,
        activity_uri: &str,
        activity_type: Option<&str>,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        // Build and deliver Undo activity
        let undo_id = format!("{}/undo/{}", self.actor_uri, crate::data::EntityId::new().0);
        let object = build_undo_object(activity_uri, activity_type, None);
        let activity = builder::undo(&undo_id, &self.actor_uri, object);

        self.deliver_to_followers(activity, inbox_uris).await
    }

    /// Send Undo activity to a single inbox.
    pub async fn send_undo_to_inbox(
        &self,
        activity_uri: &str,
        inbox_uri: &str,
    ) -> Result<(), AppError> {
        self.send_undo_to_inbox_with_type(activity_uri, None, inbox_uri)
            .await
    }

    /// Send Undo activity to a single inbox with optional object type.
    pub async fn send_undo_to_inbox_with_type(
        &self,
        activity_uri: &str,
        activity_type: Option<&str>,
        inbox_uri: &str,
    ) -> Result<(), AppError> {
        self.send_undo_to_inbox_with_type_and_object(activity_uri, activity_type, None, inbox_uri)
            .await
    }

    /// Send Undo activity to a single inbox with optional object type and target object.
    pub async fn send_undo_to_inbox_with_type_and_object(
        &self,
        activity_uri: &str,
        activity_type: Option<&str>,
        activity_object: Option<&str>,
        inbox_uri: &str,
    ) -> Result<(), AppError> {
        let undo_id = format!("{}/undo/{}", self.actor_uri, crate::data::EntityId::new().0);
        let object = build_undo_object(activity_uri, activity_type, activity_object);
        let activity = builder::undo(&undo_id, &self.actor_uri, object);

        self.deliver_to_inbox(inbox_uri, activity).await?;

        tracing::info!("Sent Undo {} to {}", activity_uri, inbox_uri);
        Ok(())
    }

    /// Send Announce activity (boost)
    pub async fn send_announce(
        &self,
        status_uri: &str,
        status_visibility: &str,
        inbox_uris: Vec<String>,
    ) -> Result<String, AppError> {
        // Build Announce activity
        let announce_id = format!(
            "{}/announce/{}",
            self.actor_uri,
            crate::data::EntityId::new().0
        );
        let results = self
            .send_announce_with_id(&announce_id, status_uri, status_visibility, inbox_uris)
            .await;

        // Check if at least one delivery succeeded
        if results.iter().any(|r| r.success) {
            tracing::info!("Sent Announce for {}", status_uri);
            Ok(announce_id)
        } else {
            Err(AppError::Federation("All deliveries failed".to_string()))
        }
    }

    /// Send Announce activity with explicit activity URI.
    pub async fn send_announce_with_id(
        &self,
        announce_activity_uri: &str,
        status_uri: &str,
        status_visibility: &str,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        let (to_audience, cc_audience) =
            audience_for_visibility(&self.actor_uri, status_visibility);
        let activity = builder::announce(
            announce_activity_uri,
            &self.actor_uri,
            status_uri,
            to_audience.iter().map(String::as_str).collect(),
            cc_audience.iter().map(String::as_str).collect(),
        );

        self.deliver_to_followers(activity, inbox_uris).await
    }
}

/// Result of a delivery attempt
#[derive(Debug, Clone)]
pub struct DeliveryResult {
    /// Target inbox URI
    pub inbox_uri: String,
    /// Whether delivery succeeded
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
    /// HTTP status code if available
    pub status_code: Option<u16>,
}

/// Build ActivityPub activity JSON
pub mod builder {
    use serde_json::Value;

    /// Build a Follow activity
    ///
    /// # Arguments
    /// * `id` - Activity ID (unique URI)
    /// * `actor` - Actor URI (follower)
    /// * `object` - Object URI (followee)
    pub fn follow(id: &str, actor: &str, object: &str) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Follow",
            "id": id,
            "actor": actor,
            "object": object
        })
    }

    /// Build an Accept activity
    ///
    /// # Arguments
    /// * `id` - Activity ID (unique URI)
    /// * `actor` - Actor URI (accepter)
    /// * `object` - Original activity being accepted (usually a Follow)
    pub fn accept(id: &str, actor: &str, object: Value) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Accept",
            "id": id,
            "actor": actor,
            "object": object
        })
    }

    /// Build a Reject activity.
    pub fn reject(id: &str, actor: &str, object: Value) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Reject",
            "id": id,
            "actor": actor,
            "object": object
        })
    }

    /// Build a Create activity
    ///
    /// # Arguments
    /// * `id` - Activity ID (unique URI)
    /// * `actor` - Actor URI (creator)
    /// * `object` - Object being created (usually a Note)
    /// * `to` - Primary recipients (public timeline, followers, etc.)
    /// * `cc` - CC recipients (mentions, etc.)
    pub fn create(id: &str, actor: &str, object: Value, to: Vec<&str>, cc: Vec<&str>) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Create",
            "id": id,
            "actor": actor,
            "object": object,
            "to": to,
            "cc": cc,
            "published": chrono::Utc::now().to_rfc3339()
        })
    }

    /// Build a Delete activity
    ///
    /// # Arguments
    /// * `id` - Activity ID (unique URI)
    /// * `actor` - Actor URI (deleter)
    /// * `object` - Object URI being deleted
    pub fn delete(id: &str, actor: &str, object: &str, to: Vec<&str>, cc: Vec<&str>) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Delete",
            "id": id,
            "actor": actor,
            "object": {
                "type": "Tombstone",
                "id": object
            },
            "to": to,
            "cc": cc
        })
    }

    /// Build a Like activity
    ///
    /// # Arguments
    /// * `id` - Activity ID (unique URI)
    /// * `actor` - Actor URI (liker)
    /// * `object` - Object URI being liked (status)
    pub fn like(id: &str, actor: &str, object: &str) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Like",
            "id": id,
            "actor": actor,
            "object": object
        })
    }

    /// Build an Announce activity (boost/reblog)
    ///
    /// # Arguments
    /// * `id` - Activity ID (unique URI)
    /// * `actor` - Actor URI (announcer)
    /// * `object` - Object URI being announced (status)
    /// * `to` - Recipients
    /// * `cc` - Secondary recipients
    pub fn announce(id: &str, actor: &str, object: &str, to: Vec<&str>, cc: Vec<&str>) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Announce",
            "id": id,
            "actor": actor,
            "object": object,
            "to": to,
            "cc": cc,
            "published": chrono::Utc::now().to_rfc3339()
        })
    }

    /// Build an Undo activity
    ///
    /// # Arguments
    /// * `id` - Activity ID (unique URI)
    /// * `actor` - Actor URI (undoer)
    /// * `object` - Original activity being undone
    pub fn undo(id: &str, actor: &str, object: Value) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Undo",
            "id": id,
            "actor": actor,
            "object": object
        })
    }

    /// Build a Note object
    ///
    /// # Arguments
    /// * `id` - Note ID (unique URI)
    /// * `attributed_to` - Actor URI (author)
    /// * `content` - HTML content
    /// * `published` - Publication timestamp (RFC3339)
    /// * `to` - Primary recipients
    /// * `cc` - CC recipients
    pub fn note(
        id: &str,
        attributed_to: &str,
        content: &str,
        published: &str,
        to: Vec<&str>,
        cc: Vec<&str>,
    ) -> Value {
        serde_json::json!({
            "type": "Note",
            "id": id,
            "attributedTo": attributed_to,
            "content": content,
            "published": published,
            "to": to,
            "cc": cc,
            "sensitive": false,
            "atomUri": id,
            "inReplyToAtomUri": null,
            "conversation": format!("tag:{},conversation", id.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("")),
            "contentMap": {
                "en": content
            }
        })
    }

    /// Build a Note object with reply information
    ///
    /// # Arguments
    /// * `id` - Note ID (unique URI)
    /// * `attributed_to` - Actor URI (author)
    /// * `content` - HTML content
    /// * `published` - Publication timestamp (RFC3339)
    /// * `in_reply_to` - URI of status being replied to
    /// * `to` - Primary recipients
    /// * `cc` - CC recipients
    pub fn note_reply(
        id: &str,
        attributed_to: &str,
        content: &str,
        published: &str,
        in_reply_to: &str,
        to: Vec<&str>,
        cc: Vec<&str>,
    ) -> Value {
        serde_json::json!({
            "type": "Note",
            "id": id,
            "attributedTo": attributed_to,
            "content": content,
            "published": published,
            "inReplyTo": in_reply_to,
            "to": to,
            "cc": cc,
            "sensitive": false,
            "atomUri": id,
            "inReplyToAtomUri": in_reply_to,
            "conversation": format!("tag:{},conversation", id.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("")),
            "contentMap": {
                "en": content
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{audience_for_visibility, build_undo_object, unique_inbox_targets};

    #[test]
    fn unique_inbox_targets_keeps_distinct_personal_inboxes_on_same_domain() {
        let targets = unique_inbox_targets(vec![
            "https://instance1.com/users/alice/inbox".to_string(),
            "https://instance1.com/users/bob/inbox".to_string(),
            "https://instance2.com/inbox".to_string(),
        ]);

        assert_eq!(
            targets,
            vec![
                "https://instance1.com/users/alice/inbox".to_string(),
                "https://instance1.com/users/bob/inbox".to_string(),
                "https://instance2.com/inbox".to_string(),
            ]
        );
    }

    #[test]
    fn unique_inbox_targets_deduplicates_identical_shared_inbox_uris() {
        let targets = unique_inbox_targets(vec![
            "https://instance1.com/inbox".to_string(),
            "https://instance1.com/inbox".to_string(),
            "https://instance2.com/inbox".to_string(),
            "https://instance2.com/inbox".to_string(),
        ]);

        assert_eq!(
            targets,
            vec![
                "https://instance1.com/inbox".to_string(),
                "https://instance2.com/inbox".to_string(),
            ]
        );
    }

    #[test]
    fn unique_inbox_targets_handles_empty_input() {
        let targets = unique_inbox_targets(vec![]);
        assert!(targets.is_empty());
    }

    #[test]
    fn audience_for_visibility_public_targets_public_then_followers() {
        let (to, cc) = audience_for_visibility("https://example.com/users/alice", "public");
        assert_eq!(to, vec!["https://www.w3.org/ns/activitystreams#Public"]);
        assert_eq!(cc, vec!["https://example.com/users/alice/followers"]);
    }

    #[test]
    fn audience_for_visibility_unlisted_targets_followers_then_public_cc() {
        let (to, cc) = audience_for_visibility("https://example.com/users/alice", "unlisted");
        assert_eq!(to, vec!["https://example.com/users/alice/followers"]);
        assert_eq!(cc, vec!["https://www.w3.org/ns/activitystreams#Public"]);
    }

    #[test]
    fn audience_for_visibility_private_targets_only_followers() {
        let (to, cc) = audience_for_visibility("https://example.com/users/alice", "private");
        assert_eq!(to, vec!["https://example.com/users/alice/followers"]);
        assert!(cc.is_empty());
    }

    #[test]
    fn audience_for_visibility_direct_targets_empty_audience() {
        let (to, cc) = audience_for_visibility("https://example.com/users/alice", "direct");
        assert!(to.is_empty());
        assert!(cc.is_empty());
    }

    #[test]
    fn build_undo_object_includes_type_id_and_optional_object_target() {
        let undo_object = build_undo_object(
            "https://local.example/follow/1",
            Some("Follow"),
            Some("https://remote.example/users/alice"),
        );
        assert_eq!(undo_object["type"], "Follow");
        assert_eq!(undo_object["id"], "https://local.example/follow/1");
        assert_eq!(undo_object["object"], "https://remote.example/users/alice");
    }
}
