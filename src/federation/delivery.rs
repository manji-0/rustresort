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
        let mut request = self.http_client
            .post(inbox_uri)
            .header("Content-Type", "application/activity+json")
            .header("Date", sig_headers.date)
            .header("Signature", sig_headers.signature);

        if let Some(digest) = sig_headers.digest {
            request = request.header("Digest", digest);
        }

        let response = request
            .body(body)
            .send()
            .await
            .map_err(|e| AppError::Federation(format!("Failed to deliver to {}: {}", inbox_uri, e)))?;

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
    /// Uses shared inbox when possible (group by domain)
    pub async fn deliver_to_followers(
        &self,
        activity: serde_json::Value,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        use std::collections::HashMap;
        use tokio::sync::Semaphore;

        // 1. Group by shared inbox domain to reduce deliveries
        let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
        
        for inbox_uri in inbox_uris {
            // Extract domain from inbox URI
            let domain = inbox_uri
                .split("://")
                .nth(1)
                .and_then(|s| s.split('/').next())
                .unwrap_or(&inbox_uri)
                .to_string();
            
            grouped.entry(domain).or_default().push(inbox_uri);
        }

        // 2. For each domain, use the first inbox (could be optimized to use shared inbox)
        let delivery_targets: Vec<String> = grouped
            .into_iter()
            .map(|(_, mut inboxes)| inboxes.remove(0))
            .collect();

        tracing::info!(
            "Delivering to {} unique inboxes (optimized from {} total)",
            delivery_targets.len(),
            delivery_targets.len()
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
        let follow_id = format!("{}/follow/{}", self.actor_uri, crate::data::EntityId::new().0);
        
        let activity = builder::follow(
            &follow_id,
            &self.actor_uri,
            target_actor_uri,
        );

        // 2. Deliver to inbox
        self.deliver_to_inbox(target_inbox_uri, activity).await?;

        tracing::info!("Sent Follow to {} for {}", target_inbox_uri, target_actor_uri);
        
        // 3. Return activity URI
        Ok(follow_id)
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
        let accept_id = format!("{}/accept/{}", self.actor_uri, crate::data::EntityId::new().0);
        
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

        tracing::info!("Sent Accept to {} for Follow {}", follower_inbox_uri, follow_activity_uri);
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
        // 1. Build Note object
        let note = if let Some(ref in_reply_to) = status.in_reply_to_uri {
            builder::note_reply(
                &status.uri,
                &self.actor_uri,
                &status.content,
                &status.created_at.to_rfc3339(),
                in_reply_to,
                vec!["https://www.w3.org/ns/activitystreams#Public"],
                vec![&format!("{}/followers", self.actor_uri)],
            )
        } else {
            builder::note(
                &status.uri,
                &self.actor_uri,
                &status.content,
                &status.created_at.to_rfc3339(),
                vec!["https://www.w3.org/ns/activitystreams#Public"],
                vec![&format!("{}/followers", self.actor_uri)],
            )
        };

        // 2. Wrap in Create activity
        let create_id = format!("{}/create/{}", self.actor_uri, crate::data::EntityId::new().0);
        let activity = builder::create(
            &create_id,
            &self.actor_uri,
            note,
            vec!["https://www.w3.org/ns/activitystreams#Public"],
            vec![&format!("{}/followers", self.actor_uri)],
        );

        // 3. Deliver to inboxes
        self.deliver_to_followers(activity, inbox_uris).await
    }

    /// Send Delete activity
    pub async fn send_delete(
        &self,
        object_uri: &str,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        // Build and deliver Delete activity
        let delete_id = format!("{}/delete/{}", self.actor_uri, crate::data::EntityId::new().0);
        let activity = builder::delete(&delete_id, &self.actor_uri, object_uri);

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
        let activity = builder::like(&like_id, &self.actor_uri, status_uri);

        self.deliver_to_inbox(target_inbox_uri, activity).await?;

        tracing::info!("Sent Like to {} for {}", target_inbox_uri, status_uri);
        Ok(like_id)
    }

    /// Send Undo activity
    pub async fn send_undo(
        &self,
        activity_uri: &str,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        // Build and deliver Undo activity
        let undo_id = format!("{}/undo/{}", self.actor_uri, crate::data::EntityId::new().0);
        
        // We need to wrap the original activity
        // For simplicity, just reference it by ID
        let activity = builder::undo(
            &undo_id,
            &self.actor_uri,
            serde_json::json!({
                "id": activity_uri
            }),
        );

        self.deliver_to_followers(activity, inbox_uris).await
    }

    /// Send Announce activity (boost)
    pub async fn send_announce(
        &self,
        status_uri: &str,
        inbox_uris: Vec<String>,
    ) -> Result<String, AppError> {
        // Build Announce activity
        let announce_id = format!("{}/announce/{}", self.actor_uri, crate::data::EntityId::new().0);
        let activity = builder::announce(
            &announce_id,
            &self.actor_uri,
            status_uri,
            vec!["https://www.w3.org/ns/activitystreams#Public", &format!("{}/followers", self.actor_uri)],
        );

        // Deliver to followers
        let results = self.deliver_to_followers(activity, inbox_uris).await;
        
        // Check if at least one delivery succeeded
        if results.iter().any(|r| r.success) {
            tracing::info!("Sent Announce for {}", status_uri);
            Ok(announce_id)
        } else {
            Err(AppError::Federation("All deliveries failed".to_string()))
        }
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

    /// Build a Create activity
    ///
    /// # Arguments
    /// * `id` - Activity ID (unique URI)
    /// * `actor` - Actor URI (creator)
    /// * `object` - Object being created (usually a Note)
    /// * `to` - Primary recipients (public timeline, followers, etc.)
    /// * `cc` - CC recipients (mentions, etc.)
    pub fn create(
        id: &str,
        actor: &str,
        object: Value,
        to: Vec<&str>,
        cc: Vec<&str>,
    ) -> Value {
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
    pub fn delete(id: &str, actor: &str, object: &str) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Delete",
            "id": id,
            "actor": actor,
            "object": {
                "type": "Tombstone",
                "id": object
            },
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
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
    /// * `to` - Recipients (usually public + followers)
    pub fn announce(id: &str, actor: &str, object: &str, to: Vec<&str>) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Announce",
            "id": id,
            "actor": actor,
            "object": object,
            "to": to,
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
