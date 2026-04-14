//! Activity delivery
//!
//! Handles delivering activities to remote servers.

use std::sync::Arc;

use axum::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

use crate::data::{Account, EntityId, MediaAttachment, Status};
use crate::error::AppError;
use crate::storage::MediaStorageRepository;

const DELIVERY_WORKER_BATCH_SIZE: usize = 32;
const DELIVERY_WORKER_IDLE_MILLIS: u64 = 250;
const DELIVERY_WORKER_MAX_ATTEMPTS: u32 = 8;

/// Persistent queue for outbound federation deliveries.
#[async_trait]
pub trait DeliveryQueue: Send + Sync {
    async fn enqueue(
        &self,
        inbox_url: &str,
        activity_json: &str,
        actor_key_id: &str,
    ) -> Result<(), AppError>;
    async fn claim_pending(&self, limit: usize) -> Result<Vec<crate::data::DeliveryJob>, AppError>;
    async fn mark_delivered(&self, job_id: &str) -> Result<(), AppError>;
    async fn mark_failed(&self, job_id: &str, error: &str) -> Result<(), AppError>;
    async fn reap_dead_jobs(&self, max_attempts: u32) -> Result<u64, AppError>;
    async fn is_blocked_by_remote(&self, actor_uri: &str) -> Result<bool, AppError>;
    async fn get_media_by_status(&self, status_id: &str) -> Result<Vec<MediaAttachment>, AppError>;
    async fn get_poll_by_status_id(
        &self,
        status_id: &str,
    ) -> Result<Option<(String, String, bool, bool, i64, i64)>, AppError>;
    async fn get_poll_options(&self, poll_id: &str)
    -> Result<Vec<(String, String, i64)>, AppError>;
}

#[async_trait]
impl DeliveryQueue for crate::data::Database {
    async fn enqueue(
        &self,
        inbox_url: &str,
        activity_json: &str,
        actor_key_id: &str,
    ) -> Result<(), AppError> {
        self.enqueue_delivery_job(inbox_url, activity_json, actor_key_id)
            .await
    }

    async fn claim_pending(&self, limit: usize) -> Result<Vec<crate::data::DeliveryJob>, AppError> {
        self.claim_pending_delivery_jobs(limit).await
    }

    async fn mark_delivered(&self, job_id: &str) -> Result<(), AppError> {
        self.mark_delivery_job_delivered(job_id).await
    }

    async fn mark_failed(&self, job_id: &str, error: &str) -> Result<(), AppError> {
        self.mark_delivery_job_failed(job_id, error).await
    }

    async fn reap_dead_jobs(&self, max_attempts: u32) -> Result<u64, AppError> {
        self.reap_dead_delivery_jobs(max_attempts).await
    }

    async fn is_blocked_by_remote(&self, actor_uri: &str) -> Result<bool, AppError> {
        crate::data::Database::is_blocked_by_remote(self, actor_uri).await
    }

    async fn get_media_by_status(&self, status_id: &str) -> Result<Vec<MediaAttachment>, AppError> {
        crate::data::Database::get_media_by_status(self, status_id).await
    }

    async fn get_poll_by_status_id(
        &self,
        status_id: &str,
    ) -> Result<Option<(String, String, bool, bool, i64, i64)>, AppError> {
        crate::data::Database::get_poll_by_status_id(self, status_id).await
    }

    async fn get_poll_options(
        &self,
        poll_id: &str,
    ) -> Result<Vec<(String, String, i64)>, AppError> {
        crate::data::Database::get_poll_options(self, poll_id).await
    }
}

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
    /// Optional storage handle for building public media URLs in ActivityPub objects.
    media_storage: Option<Arc<dyn MediaStorageRepository>>,
}

pub fn local_actor_uri(base_url: &str, username: &str) -> String {
    format!("{base_url}/users/{username}")
}

pub fn local_key_id(actor_uri: &str) -> String {
    format!("{actor_uri}#main-key")
}

pub fn build_local_delivery(
    http_client: Arc<reqwest::Client>,
    base_url: &str,
    account: &Account,
) -> ActivityDelivery {
    let actor_uri = local_actor_uri(base_url, &account.username);
    ActivityDelivery::new(
        http_client,
        actor_uri.clone(),
        local_key_id(&actor_uri),
        account.private_key_pem.clone(),
    )
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

fn push_unique_values(target: &mut Vec<String>, values: &[String]) {
    use std::collections::HashSet;

    let mut seen = target.iter().cloned().collect::<HashSet<_>>();
    for value in values {
        if seen.insert(value.clone()) {
            target.push(value.clone());
        }
    }
}

fn merge_explicit_recipient_audience(
    actor_uri: &str,
    visibility: &str,
    explicit_recipient_actor_uris: &[String],
) -> (Vec<String>, Vec<String>) {
    let (mut to_audience, mut cc_audience) = audience_for_visibility(actor_uri, visibility);
    match visibility {
        "private" | "direct" => push_unique_values(&mut to_audience, explicit_recipient_actor_uris),
        _ => push_unique_values(&mut cc_audience, explicit_recipient_actor_uris),
    }
    (to_audience, cc_audience)
}

fn actor_base_url(actor_uri: &str) -> Option<String> {
    let parsed = url::Url::parse(actor_uri).ok()?;
    Some(format!(
        "{}://{}",
        parsed.scheme(),
        parsed.host_str().map(|host| {
            if let Some(port) = parsed.port() {
                format!("{host}:{port}")
            } else {
                host.to_string()
            }
        })?
    ))
}

fn hashtag_tag_objects(base_url: &str, content: &str) -> Vec<serde_json::Value> {
    crate::data::extract_hashtags_from_content(content)
        .into_iter()
        .map(|name| {
            serde_json::json!({
                "type": "Hashtag",
                "href": format!("{}/tags/{}", base_url, name),
                "name": format!("#{}", name),
            })
        })
        .collect()
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

fn block_activity_uri(actor_uri: &str, target_actor_uri: &str) -> String {
    let digest = Sha256::digest(target_actor_uri.as_bytes());
    let suffix = URL_SAFE_NO_PAD.encode(digest);
    format!("{}/block/{}", actor_uri, suffix)
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
            media_storage: None,
        }
    }

    pub fn with_media_storage(mut self, media_storage: Arc<dyn MediaStorageRepository>) -> Self {
        self.media_storage = Some(media_storage);
        self
    }

    fn note_attachment_type(content_type: &str) -> &'static str {
        if content_type.starts_with("image/") {
            "Image"
        } else if content_type.starts_with("video/") {
            "Video"
        } else if content_type.starts_with("audio/") {
            "Audio"
        } else {
            "Document"
        }
    }

    fn local_media_attachment_object(
        &self,
        attachment: &MediaAttachment,
    ) -> Option<serde_json::Value> {
        let storage = self.media_storage.as_ref()?;
        let url = storage.get_public_url(&attachment.s3_key);
        let preview_url = attachment
            .thumbnail_s3_key
            .as_ref()
            .map(|key| storage.get_public_url(key));
        let content_type = attachment.content_type.clone();

        let mut object = serde_json::json!({
            "type": Self::note_attachment_type(&content_type),
            "mediaType": content_type,
            "url": url,
        });
        if let Some(name) = &attachment.description {
            object["name"] = serde_json::json!(name);
        }
        if let Some(blurhash) = &attachment.blurhash {
            object["blurhash"] = serde_json::json!(blurhash);
        }
        if let Some(width) = attachment.width {
            object["width"] = serde_json::json!(width);
        }
        if let Some(height) = attachment.height {
            object["height"] = serde_json::json!(height);
        }
        if let Some(preview_url) = preview_url {
            object["icon"] = serde_json::json!({
                "type": "Image",
                "mediaType": attachment.content_type,
                "url": preview_url,
            });
        }
        Some(object)
    }

    async fn enrich_status_object(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        status: &Status,
        object: &mut serde_json::Value,
        mention_tags: &[serde_json::Value],
    ) -> Result<(), AppError> {
        if let Some(summary) = &status.content_warning {
            object["summary"] = serde_json::json!(summary);
            object["sensitive"] = serde_json::json!(true);
        }
        if let Some(language) = &status.language {
            let mut content_map = serde_json::Map::new();
            content_map.insert(language.clone(), serde_json::json!(status.content.clone()));
            object["contentMap"] = serde_json::Value::Object(content_map);
        }
        let mut tags = mention_tags.to_vec();
        if let Some(base_url) = actor_base_url(&self.actor_uri) {
            tags.extend(hashtag_tag_objects(&base_url, &status.content));
        }
        if !tags.is_empty() {
            object["tag"] = serde_json::json!(tags);
        }

        let attachments = queue
            .get_media_by_status(&status.id)
            .await?
            .into_iter()
            .filter_map(|attachment| self.local_media_attachment_object(&attachment))
            .collect::<Vec<_>>();
        if !attachments.is_empty() {
            object["attachment"] = serde_json::json!(attachments);
        }

        Ok(())
    }

    async fn build_status_object_with_audience(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        status: &Status,
        note_to: &[&str],
        note_cc: &[&str],
        mention_tags: &[serde_json::Value],
    ) -> Result<serde_json::Value, AppError> {
        let poll = queue.get_poll_by_status_id(&status.id).await?;
        let mut object = if let Some((
            poll_id,
            expires_at,
            expired,
            multiple,
            _votes_count,
            voters_count,
        )) = poll
        {
            let options = queue
                .get_poll_options(&poll_id)
                .await?
                .into_iter()
                .map(|(_, title, votes_count)| {
                    serde_json::json!({
                        "type": "Note",
                        "name": title,
                        "replies": {
                            "type": "Collection",
                            "totalItems": votes_count,
                        }
                    })
                })
                .collect::<Vec<_>>();
            let mut object = serde_json::json!({
                "type": "Question",
                "id": status.uri,
                "attributedTo": self.actor_uri,
                "content": status.content,
                "published": status.created_at.to_rfc3339(),
                "to": note_to,
                "cc": note_cc,
                "endTime": expires_at,
                "votersCount": voters_count,
            });
            if expired {
                object["closed"] = serde_json::json!(expires_at);
            }
            if multiple {
                object["anyOf"] = serde_json::json!(options);
            } else {
                object["oneOf"] = serde_json::json!(options);
            }
            object
        } else if let Some(ref in_reply_to) = status.in_reply_to_uri {
            builder::note_reply(
                &status.uri,
                &self.actor_uri,
                &status.content,
                &status.created_at.to_rfc3339(),
                in_reply_to,
                note_to.to_vec(),
                note_cc.to_vec(),
            )
        } else {
            builder::note(
                &status.uri,
                &self.actor_uri,
                &status.content,
                &status.created_at.to_rfc3339(),
                note_to.to_vec(),
                note_cc.to_vec(),
            )
        };
        self.enrich_status_object(queue, status, &mut object, mention_tags)
            .await?;
        if let Some(ref quote_of_uri) = status.quote_of_uri
            && let Some(object_map) = object.as_object_mut()
        {
            object_map.insert(
                "quoteUri".to_string(),
                serde_json::Value::String(quote_of_uri.clone()),
            );
            object_map.insert(
                "quoteUrl".to_string(),
                serde_json::Value::String(quote_of_uri.clone()),
            );
        }
        Ok(object)
    }

    fn serialize_activity(activity: &serde_json::Value) -> Result<String, AppError> {
        serde_json::to_string(activity)
            .map_err(|e| AppError::Validation(format!("Failed to serialize activity: {}", e)))
    }

    async fn enqueue_serialized_activity(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        inbox_uri: &str,
        activity_json: &str,
    ) -> Result<(), AppError> {
        queue.enqueue(inbox_uri, activity_json, &self.key_id).await
    }

    async fn enqueue_activity(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        inbox_uri: &str,
        activity: &serde_json::Value,
    ) -> Result<(), AppError> {
        let activity_json = Self::serialize_activity(activity)?;
        self.enqueue_serialized_activity(queue, inbox_uri, &activity_json)
            .await
    }

    async fn should_skip_remote_delivery(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        target_actor_uri: &str,
    ) -> Result<bool, AppError> {
        if queue.is_blocked_by_remote(target_actor_uri).await? {
            tracing::info!(
                target_actor_uri,
                "Skipping outbound delivery because remote actor has blocked the local account"
            );
            return Ok(true);
        }
        Ok(false)
    }

    async fn enqueue_activity_to_followers(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        activity: &serde_json::Value,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        let delivery_targets = unique_inbox_targets(inbox_uris);
        let activity_json = match Self::serialize_activity(activity) {
            Ok(value) => value,
            Err(error) => {
                return delivery_targets
                    .into_iter()
                    .map(|inbox_uri| DeliveryResult {
                        inbox_uri,
                        success: false,
                        error: Some(error.to_string()),
                        status_code: None,
                    })
                    .collect();
            }
        };

        let mut results = Vec::with_capacity(delivery_targets.len());
        for inbox_uri in delivery_targets {
            let result = self
                .enqueue_serialized_activity(queue, &inbox_uri, &activity_json)
                .await;
            results.push(DeliveryResult {
                inbox_uri,
                success: result.is_ok(),
                error: result.err().map(|e| e.to_string()),
                status_code: None,
            });
        }
        results
    }

    fn build_follow_activity(
        &self,
        follow_activity_uri: &str,
        target_actor_uri: &str,
    ) -> serde_json::Value {
        builder::follow(follow_activity_uri, &self.actor_uri, target_actor_uri)
    }

    fn build_block_activity(
        &self,
        block_activity_uri: &str,
        target_actor_uri: &str,
    ) -> serde_json::Value {
        builder::block(block_activity_uri, &self.actor_uri, target_actor_uri)
    }

    fn build_accept_activity(&self, follow_activity_uri: &str) -> serde_json::Value {
        let accept_id = format!(
            "{}/accept/{}",
            self.actor_uri,
            crate::data::EntityId::new_string()
        );
        builder::accept(
            &accept_id,
            &self.actor_uri,
            serde_json::json!({
                "type": "Follow",
                "id": follow_activity_uri
            }),
        )
    }

    fn build_reject_activity(&self, follow_activity_uri: &str) -> serde_json::Value {
        let reject_id = format!(
            "{}/reject/{}",
            self.actor_uri,
            crate::data::EntityId::new_string()
        );
        builder::reject(
            &reject_id,
            &self.actor_uri,
            serde_json::json!({
                "type": "Follow",
                "id": follow_activity_uri
            }),
        )
    }

    async fn build_create_activity(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        status: &Status,
    ) -> Result<serde_json::Value, AppError> {
        self.build_create_activity_with_audience(queue, status, &[], &[])
            .await
    }

    fn build_create_activity_legacy(&self, status: &Status) -> serde_json::Value {
        let (to_audience, cc_audience) =
            merge_explicit_recipient_audience(&self.actor_uri, status.visibility.as_str(), &[]);
        let note_to: Vec<&str> = to_audience.iter().map(String::as_str).collect();
        let note_cc: Vec<&str> = cc_audience.iter().map(String::as_str).collect();
        let mut note = if let Some(ref in_reply_to) = status.in_reply_to_uri {
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
        if let Some(summary) = &status.content_warning {
            note["summary"] = serde_json::json!(summary);
            note["sensitive"] = serde_json::json!(true);
        }
        if let Some(language) = &status.language {
            let mut content_map = serde_json::Map::new();
            content_map.insert(language.clone(), serde_json::json!(status.content.clone()));
            note["contentMap"] = serde_json::Value::Object(content_map);
        }
        if let Some(ref quote_of_uri) = status.quote_of_uri
            && let Some(note_object) = note.as_object_mut()
        {
            note_object.insert(
                "quoteUri".to_string(),
                serde_json::Value::String(quote_of_uri.clone()),
            );
            note_object.insert(
                "quoteUrl".to_string(),
                serde_json::Value::String(quote_of_uri.clone()),
            );
        }

        builder::create(
            &format!("{}/activity", status.uri),
            &self.actor_uri,
            note,
            note_to,
            note_cc,
        )
    }

    async fn build_create_activity_with_audience(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        status: &Status,
        explicit_recipient_actor_uris: &[String],
        mention_tags: &[serde_json::Value],
    ) -> Result<serde_json::Value, AppError> {
        let (to_audience, cc_audience) = merge_explicit_recipient_audience(
            &self.actor_uri,
            status.visibility.as_str(),
            explicit_recipient_actor_uris,
        );
        let note_to: Vec<&str> = to_audience.iter().map(String::as_str).collect();
        let note_cc: Vec<&str> = cc_audience.iter().map(String::as_str).collect();
        let note = self
            .build_status_object_with_audience(queue, status, &note_to, &note_cc, mention_tags)
            .await?;
        let create_id = format!("{}/activity", status.uri);
        Ok(builder::create(
            &create_id,
            &self.actor_uri,
            note,
            note_to,
            note_cc,
        ))
    }

    async fn build_update_status_activity(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        status: &Status,
        explicit_recipient_actor_uris: &[String],
        mention_tags: &[serde_json::Value],
    ) -> Result<serde_json::Value, AppError> {
        let (to_audience, cc_audience) = merge_explicit_recipient_audience(
            &self.actor_uri,
            status.visibility.as_str(),
            explicit_recipient_actor_uris,
        );
        let note_to: Vec<&str> = to_audience.iter().map(String::as_str).collect();
        let note_cc: Vec<&str> = cc_audience.iter().map(String::as_str).collect();
        let object = self
            .build_status_object_with_audience(queue, status, &note_to, &note_cc, mention_tags)
            .await?;

        Ok(builder::update(
            &format!("{}/activity/update/{}", status.uri, EntityId::new_string()),
            &self.actor_uri,
            object,
            note_to,
            note_cc,
        ))
    }

    fn build_delete_activity(
        &self,
        object_uri: &str,
        object_visibility: &str,
        explicit_recipient_actor_uris: &[String],
    ) -> serde_json::Value {
        let delete_id = format!("{}/delete/{}", self.actor_uri, EntityId::new_string());
        let (to_audience, cc_audience) = merge_explicit_recipient_audience(
            &self.actor_uri,
            object_visibility,
            explicit_recipient_actor_uris,
        );
        builder::delete(
            &delete_id,
            &self.actor_uri,
            object_uri,
            to_audience.iter().map(String::as_str).collect(),
            cc_audience.iter().map(String::as_str).collect(),
        )
    }

    fn build_like_activity(&self, like_activity_uri: &str, status_uri: &str) -> serde_json::Value {
        builder::like(like_activity_uri, &self.actor_uri, status_uri)
    }

    fn build_poll_vote_activity(
        &self,
        vote_activity_uri: &str,
        vote_object_uri: &str,
        poll_uri: &str,
        option_title: &str,
        target_actor_uri: &str,
    ) -> serde_json::Value {
        builder::create(
            vote_activity_uri,
            &self.actor_uri,
            serde_json::json!({
                "id": vote_object_uri,
                "type": "Note",
                "name": option_title,
                "attributedTo": self.actor_uri,
                "to": [target_actor_uri],
                "inReplyTo": poll_uri,
            }),
            vec![target_actor_uri],
            Vec::new(),
        )
    }

    fn build_undo_activity(
        &self,
        activity_uri: &str,
        activity_type: Option<&str>,
        activity_object: Option<&str>,
    ) -> serde_json::Value {
        let undo_id = format!(
            "{}/undo/{}",
            self.actor_uri,
            crate::data::EntityId::new_string()
        );
        let object = build_undo_object(activity_uri, activity_type, activity_object);
        builder::undo(&undo_id, &self.actor_uri, object)
    }

    fn build_announce_activity(
        &self,
        announce_activity_uri: &str,
        status_uri: &str,
        status_visibility: &str,
    ) -> serde_json::Value {
        let (to_audience, cc_audience) =
            audience_for_visibility(&self.actor_uri, status_visibility);
        builder::announce(
            announce_activity_uri,
            &self.actor_uri,
            status_uri,
            to_audience.iter().map(String::as_str).collect(),
            cc_audience.iter().map(String::as_str).collect(),
        )
    }

    fn build_move_activity(&self, new_account_uri: &str) -> serde_json::Value {
        let move_id = format!(
            "{}/move/{}",
            self.actor_uri,
            crate::data::EntityId::new_string()
        );
        builder::move_activity(
            &move_id,
            &self.actor_uri,
            new_account_uri,
            vec![&format!("{}/followers", self.actor_uri)],
        )
    }

    fn build_update_actor_activity(
        &self,
        actor_object: serde_json::Value,
        follower_actor_uris: &[String],
    ) -> serde_json::Value {
        builder::update(
            &format!("{}/update/{}", self.actor_uri, EntityId::new_string()),
            &self.actor_uri,
            actor_object,
            vec![&format!("{}/followers", self.actor_uri)],
            follower_actor_uris.iter().map(String::as_str).collect(),
        )
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
            crate::data::EntityId::new_string()
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
        let activity = self.build_follow_activity(follow_activity_uri, target_actor_uri);

        self.deliver_to_inbox(target_inbox_uri, activity).await?;

        tracing::info!(
            "Sent Follow {} to {} for {}",
            follow_activity_uri,
            target_inbox_uri,
            target_actor_uri
        );

        Ok(())
    }

    pub async fn queue_follow_with_id(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        follow_activity_uri: &str,
        target_actor_uri: &str,
        target_inbox_uri: &str,
    ) -> Result<(), AppError> {
        if self
            .should_skip_remote_delivery(queue, target_actor_uri)
            .await?
        {
            return Ok(());
        }
        let activity = self.build_follow_activity(follow_activity_uri, target_actor_uri);
        self.enqueue_activity(queue, target_inbox_uri, &activity)
            .await
    }

    /// Compute deterministic Block activity URI for a target actor.
    pub fn block_activity_uri_for_target(&self, target_actor_uri: &str) -> String {
        block_activity_uri(&self.actor_uri, target_actor_uri)
    }

    /// Send Block activity.
    pub async fn send_block(
        &self,
        target_actor_uri: &str,
        target_inbox_uri: &str,
    ) -> Result<String, AppError> {
        let block_activity_uri = self.block_activity_uri_for_target(target_actor_uri);
        self.send_block_with_id(&block_activity_uri, target_actor_uri, target_inbox_uri)
            .await?;
        Ok(block_activity_uri)
    }

    /// Send Block activity with explicit activity URI.
    pub async fn send_block_with_id(
        &self,
        block_activity_uri: &str,
        target_actor_uri: &str,
        target_inbox_uri: &str,
    ) -> Result<(), AppError> {
        let activity = self.build_block_activity(block_activity_uri, target_actor_uri);

        self.deliver_to_inbox(target_inbox_uri, activity).await?;

        tracing::info!(
            "Sent Block {} to {} for {}",
            block_activity_uri,
            target_inbox_uri,
            target_actor_uri
        );

        Ok(())
    }

    pub async fn queue_block_with_id(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        block_activity_uri: &str,
        target_actor_uri: &str,
        target_inbox_uri: &str,
    ) -> Result<(), AppError> {
        if self
            .should_skip_remote_delivery(queue, target_actor_uri)
            .await?
        {
            return Ok(());
        }
        let activity = self.build_block_activity(block_activity_uri, target_actor_uri);
        self.enqueue_activity(queue, target_inbox_uri, &activity)
            .await
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
        let activity = self.build_accept_activity(follow_activity_uri);

        // 2. Deliver to inbox
        self.deliver_to_inbox(follower_inbox_uri, activity).await?;

        tracing::info!(
            "Sent Accept to {} for Follow {}",
            follower_inbox_uri,
            follow_activity_uri
        );
        Ok(())
    }

    pub async fn queue_accept(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        follow_activity_uri: &str,
        follower_inbox_uri: &str,
    ) -> Result<(), AppError> {
        let activity = self.build_accept_activity(follow_activity_uri);
        self.enqueue_activity(queue, follower_inbox_uri, &activity)
            .await
    }

    /// Send Reject activity (for follow request rejection)
    pub async fn send_reject(
        &self,
        follow_activity_uri: &str,
        follower_inbox_uri: &str,
    ) -> Result<(), AppError> {
        let activity = self.build_reject_activity(follow_activity_uri);

        self.deliver_to_inbox(follower_inbox_uri, activity).await?;

        tracing::info!(
            "Sent Reject to {} for Follow {}",
            follower_inbox_uri,
            follow_activity_uri
        );

        Ok(())
    }

    pub async fn queue_reject(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        follow_activity_uri: &str,
        follower_inbox_uri: &str,
    ) -> Result<(), AppError> {
        let activity = self.build_reject_activity(follow_activity_uri);
        self.enqueue_activity(queue, follower_inbox_uri, &activity)
            .await
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
        let activity = self.build_create_activity_legacy(status);

        // 3. Deliver to inboxes
        self.deliver_to_followers(activity, inbox_uris).await
    }

    pub async fn queue_create(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        status: &Status,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        let activity = match self.build_create_activity(queue, status).await {
            Ok(activity) => activity,
            Err(error) => {
                return unique_inbox_targets(inbox_uris)
                    .into_iter()
                    .map(|inbox_uri| DeliveryResult {
                        inbox_uri,
                        success: false,
                        error: Some(error.to_string()),
                        status_code: None,
                    })
                    .collect();
            }
        };
        self.enqueue_activity_to_followers(queue, &activity, inbox_uris)
            .await
    }

    pub async fn queue_create_with_audience(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        status: &Status,
        inbox_uris: Vec<String>,
        explicit_recipient_actor_uris: &[String],
        mention_tags: &[serde_json::Value],
    ) -> Vec<DeliveryResult> {
        let activity = match self
            .build_create_activity_with_audience(
                queue,
                status,
                explicit_recipient_actor_uris,
                mention_tags,
            )
            .await
        {
            Ok(activity) => activity,
            Err(error) => {
                return unique_inbox_targets(inbox_uris)
                    .into_iter()
                    .map(|inbox_uri| DeliveryResult {
                        inbox_uri,
                        success: false,
                        error: Some(error.to_string()),
                        status_code: None,
                    })
                    .collect();
            }
        };
        self.enqueue_activity_to_followers(queue, &activity, inbox_uris)
            .await
    }

    /// Send Move activity to current followers.
    pub async fn send_move(
        &self,
        new_account_uri: &str,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        let activity = self.build_move_activity(new_account_uri);
        self.deliver_to_followers(activity, inbox_uris).await
    }

    pub async fn queue_move(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        new_account_uri: &str,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        let activity = self.build_move_activity(new_account_uri);
        self.enqueue_activity_to_followers(queue, &activity, inbox_uris)
            .await
    }

    /// Send Delete activity
    pub async fn send_delete(
        &self,
        object_uri: &str,
        object_visibility: &str,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        // Build and deliver Delete activity
        let activity = self.build_delete_activity(object_uri, object_visibility, &[]);

        self.deliver_to_followers(activity, inbox_uris).await
    }

    pub async fn queue_delete(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        object_uri: &str,
        object_visibility: &str,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        let activity = self.build_delete_activity(object_uri, object_visibility, &[]);
        self.enqueue_activity_to_followers(queue, &activity, inbox_uris)
            .await
    }

    pub async fn queue_delete_with_audience(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        object_uri: &str,
        object_visibility: &str,
        inbox_uris: Vec<String>,
        explicit_recipient_actor_uris: &[String],
    ) -> Vec<DeliveryResult> {
        let activity = self.build_delete_activity(
            object_uri,
            object_visibility,
            explicit_recipient_actor_uris,
        );
        self.enqueue_activity_to_followers(queue, &activity, inbox_uris)
            .await
    }

    pub async fn queue_update_status(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        status: &Status,
        inbox_uris: Vec<String>,
        explicit_recipient_actor_uris: &[String],
        mention_tags: &[serde_json::Value],
    ) -> Vec<DeliveryResult> {
        let activity = match self
            .build_update_status_activity(
                queue,
                status,
                explicit_recipient_actor_uris,
                mention_tags,
            )
            .await
        {
            Ok(activity) => activity,
            Err(error) => {
                return unique_inbox_targets(inbox_uris)
                    .into_iter()
                    .map(|inbox_uri| DeliveryResult {
                        inbox_uri,
                        success: false,
                        error: Some(error.to_string()),
                        status_code: None,
                    })
                    .collect();
            }
        };
        self.enqueue_activity_to_followers(queue, &activity, inbox_uris)
            .await
    }

    pub async fn queue_update_actor(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        actor_object: serde_json::Value,
        inbox_uris: Vec<String>,
        follower_actor_uris: &[String],
    ) -> Vec<DeliveryResult> {
        let activity = self.build_update_actor_activity(actor_object, follower_actor_uris);
        self.enqueue_activity_to_followers(queue, &activity, inbox_uris)
            .await
    }

    /// Send Like activity
    pub async fn send_like(
        &self,
        status_uri: &str,
        target_inbox_uri: &str,
    ) -> Result<String, AppError> {
        // Build and deliver Like activity
        let like_id = format!(
            "{}/like/{}",
            self.actor_uri,
            crate::data::EntityId::new_string()
        );
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
        let activity = self.build_like_activity(like_activity_uri, status_uri);

        self.deliver_to_inbox(target_inbox_uri, activity).await?;

        tracing::info!(
            "Sent Like {} to {} for {}",
            like_activity_uri,
            target_inbox_uri,
            status_uri
        );

        Ok(())
    }

    pub async fn queue_like_with_id(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        like_activity_uri: &str,
        status_uri: &str,
        target_inbox_uri: &str,
        target_actor_uri: &str,
    ) -> Result<(), AppError> {
        if self
            .should_skip_remote_delivery(queue, target_actor_uri)
            .await?
        {
            return Ok(());
        }
        let activity = self.build_like_activity(like_activity_uri, status_uri);
        self.enqueue_activity(queue, target_inbox_uri, &activity)
            .await
    }

    pub async fn queue_poll_vote(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        poll_uri: &str,
        option_titles: &[String],
        target_actor_uri: &str,
        target_inbox_uri: &str,
    ) -> Result<(), AppError> {
        if self
            .should_skip_remote_delivery(queue, target_actor_uri)
            .await?
        {
            return Ok(());
        }

        for option_title in option_titles {
            let vote_object_uri = format!("{}/votes/{}", self.actor_uri, EntityId::new_string());
            let vote_activity_uri = format!("{vote_object_uri}/activity");
            let activity = self.build_poll_vote_activity(
                &vote_activity_uri,
                &vote_object_uri,
                poll_uri,
                option_title,
                target_actor_uri,
            );
            self.enqueue_activity(queue, target_inbox_uri, &activity)
                .await?;
        }

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
        let activity = self.build_undo_activity(activity_uri, activity_type, None);

        self.deliver_to_followers(activity, inbox_uris).await
    }

    pub async fn queue_undo_with_type(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        activity_uri: &str,
        activity_type: Option<&str>,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        let activity = self.build_undo_activity(activity_uri, activity_type, None);
        self.enqueue_activity_to_followers(queue, &activity, inbox_uris)
            .await
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
        let activity = self.build_undo_activity(activity_uri, activity_type, activity_object);

        self.deliver_to_inbox(inbox_uri, activity).await?;

        tracing::info!("Sent Undo {} to {}", activity_uri, inbox_uri);
        Ok(())
    }

    pub async fn queue_undo_to_inbox_with_type_and_object(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        activity_uri: &str,
        activity_type: Option<&str>,
        activity_object: Option<&str>,
        target_actor_uri: Option<&str>,
        inbox_uri: &str,
    ) -> Result<(), AppError> {
        if let Some(target_actor_uri) = target_actor_uri
            && self
                .should_skip_remote_delivery(queue, target_actor_uri)
                .await?
        {
            return Ok(());
        }
        let activity = self.build_undo_activity(activity_uri, activity_type, activity_object);
        self.enqueue_activity(queue, inbox_uri, &activity).await
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
            crate::data::EntityId::new_string()
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
        let activity =
            self.build_announce_activity(announce_activity_uri, status_uri, status_visibility);

        self.deliver_to_followers(activity, inbox_uris).await
    }

    pub async fn queue_announce_with_id(
        &self,
        queue: &(impl DeliveryQueue + ?Sized),
        announce_activity_uri: &str,
        status_uri: &str,
        status_visibility: &str,
        inbox_uris: Vec<String>,
    ) -> Vec<DeliveryResult> {
        let activity =
            self.build_announce_activity(announce_activity_uri, status_uri, status_visibility);
        self.enqueue_activity_to_followers(queue, &activity, inbox_uris)
            .await
    }
}

pub fn spawn_delivery_worker(state: crate::AppState) {
    tokio::spawn(async move {
        loop {
            match run_delivery_worker_once(&state).await {
                Ok(0) => {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        DELIVERY_WORKER_IDLE_MILLIS,
                    ))
                    .await;
                }
                Ok(_) => {
                    tokio::task::yield_now().await;
                }
                Err(error) => {
                    tracing::error!(%error, "Delivery worker iteration failed");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    });

    tracing::info!("Delivery worker spawned");
}

async fn run_delivery_worker_once(state: &crate::AppState) -> Result<usize, AppError> {
    let jobs = state.db.claim_pending(DELIVERY_WORKER_BATCH_SIZE).await?;
    if jobs.is_empty() {
        return Ok(0);
    }

    let account = state.db.get_account().await?.ok_or_else(|| {
        AppError::internal("local account missing while processing delivery queue")
    })?;
    let expected_key_id = local_key_id(&local_actor_uri(
        &state.config.server.base_url(),
        &account.username,
    ));
    let delivery = build_local_delivery(
        state.http_client.clone(),
        &state.config.server.base_url(),
        &account,
    );

    for job in &jobs {
        if job.actor_key_id != expected_key_id {
            tracing::warn!(
                job_id = %job.id,
                queued_key_id = %job.actor_key_id,
                current_key_id = %expected_key_id,
                "Queued delivery job key id differs from current local key id; using current key"
            );
        }

        let activity = match serde_json::from_str::<serde_json::Value>(&job.activity_json) {
            Ok(activity) => activity,
            Err(error) => {
                state
                    .db
                    .mark_failed(&job.id, &format!("invalid queued activity JSON: {}", error))
                    .await?;
                continue;
            }
        };

        match delivery.deliver_to_inbox(&job.inbox_url, activity).await {
            Ok(()) => state.db.mark_delivered(&job.id).await?,
            Err(error) => state.db.mark_failed(&job.id, &error.to_string()).await?,
        }
    }

    let reaped = state
        .db
        .reap_dead_jobs(DELIVERY_WORKER_MAX_ATTEMPTS)
        .await?;
    if reaped > 0 {
        tracing::warn!(reaped, "Dropped permanently failing delivery jobs");
    }

    Ok(jobs.len())
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

    /// Build a Block activity.
    pub fn block(id: &str, actor: &str, object: &str) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Block",
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

    /// Build an Update activity.
    pub fn update(id: &str, actor: &str, object: Value, to: Vec<&str>, cc: Vec<&str>) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Update",
            "id": id,
            "actor": actor,
            "object": object,
            "to": to,
            "cc": cc,
            "published": chrono::Utc::now().to_rfc3339()
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

    /// Build a Move activity.
    pub fn move_activity(id: &str, actor: &str, target: &str, to: Vec<&str>) -> Value {
        serde_json::json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "type": "Move",
            "id": id,
            "actor": actor,
            "object": actor,
            "target": target,
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

#[cfg(test)]
mod tests {
    use super::{
        audience_for_visibility, block_activity_uri, build_undo_object, unique_inbox_targets,
    };
    use std::sync::Arc;

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

    #[test]
    fn block_activity_uri_is_deterministic_and_target_specific() {
        let actor_uri = "https://local.example/users/alice";
        let target_a = "https://remote.example/users/bob";
        let target_b = "https://remote.example/users/carol";

        let first = block_activity_uri(actor_uri, target_a);
        let second = block_activity_uri(actor_uri, target_a);
        let third = block_activity_uri(actor_uri, target_b);

        assert_eq!(first, second);
        assert_ne!(first, third);
        assert!(first.starts_with("https://local.example/users/alice/block/"));
    }

    #[test]
    fn builder_block_has_activitypub_fields() {
        let activity = super::builder::block(
            "https://local.example/block/1",
            "https://local.example/users/alice",
            "https://remote.example/users/bob",
        );
        assert_eq!(activity["type"], "Block");
        assert_eq!(activity["id"], "https://local.example/block/1");
        assert_eq!(activity["actor"], "https://local.example/users/alice");
        assert_eq!(activity["object"], "https://remote.example/users/bob");
    }

    #[test]
    fn build_create_activity_includes_quote_fields_on_note() {
        let delivery = super::ActivityDelivery::new(
            Arc::new(reqwest::Client::new()),
            "https://local.example/users/alice".to_string(),
            "https://local.example/users/alice#main-key".to_string(),
            "private-key".to_string(),
        );
        let status = crate::data::Status {
            id: "status-1".to_string(),
            uri: "https://local.example/users/alice/statuses/status-1".to_string(),
            content: "<p>Hello</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: Some("https://remote.example/users/bob/statuses/quoted".to_string()),
            persisted_reason: crate::data::PersistedReason::Own,
            created_at: chrono::Utc::now(),
            fetched_at: None,
        };

        let activity = delivery.build_create_activity_legacy(&status);
        assert_eq!(
            activity["object"]["quoteUri"],
            "https://remote.example/users/bob/statuses/quoted"
        );
        assert_eq!(
            activity["object"]["quoteUrl"],
            "https://remote.example/users/bob/statuses/quoted"
        );
    }
}
