//! Activity processing
//!
//! Handles incoming ActivityPub activities.

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::data::{
    CachedAttachment, CachedStatus, Database, NotificationType, PersistedReason, ProfileCache,
    PushAlerts, PushPayload, Status, StatusVisibility, TimelineCache,
};
use crate::error::AppError;
use crate::service::{StreamEvent, StreamTarget, StreamingEventBus, WebPushSender};

async fn fetch_remote_actor_document(
    http_client: &reqwest::Client,
    actor_uri: &str,
) -> Result<serde_json::Value, AppError> {
    let actor_url = url::Url::parse(actor_uri).map_err(|error| {
        AppError::Federation(format!("Invalid actor URI {} ({})", actor_uri, error))
    })?;
    let response = http_client
        .get(actor_url)
        .header(
            reqwest::header::ACCEPT,
            "application/activity+json, application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"",
        )
        .send()
        .await
        .map_err(|error| {
            AppError::Federation(format!("Actor fetch failed for {}: {}", actor_uri, error))
        })?;

    if !response.status().is_success() {
        return Err(AppError::Federation(format!(
            "Actor fetch failed for {}: HTTP {}",
            actor_uri,
            response.status()
        )));
    }

    response.json().await.map_err(|error| {
        AppError::Federation(format!(
            "Failed to decode actor document {}: {}",
            actor_uri, error
        ))
    })
}

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

fn extract_attachment_dimensions(value: &serde_json::Value) -> (Option<i32>, Option<i32>) {
    let width = value
        .get("width")
        .and_then(serde_json::Value::as_i64)
        .and_then(|raw| i32::try_from(raw).ok());
    let height = value
        .get("height")
        .and_then(serde_json::Value::as_i64)
        .and_then(|raw| i32::try_from(raw).ok());
    (width, height)
}

#[derive(Debug, Clone)]
struct ParsedQuestionPoll {
    expires_at: String,
    expired: bool,
    multiple: bool,
    votes_count: i64,
    voters_count: i64,
    options: Vec<(String, i64)>,
}

fn parse_question_poll(object: &serde_json::Value) -> Option<ParsedQuestionPoll> {
    let raw_options = object
        .get("oneOf")
        .or_else(|| object.get("anyOf"))
        .and_then(serde_json::Value::as_array)?;
    if raw_options.is_empty() {
        return None;
    }

    let mut options = Vec::with_capacity(raw_options.len());
    let mut total_votes = 0_i64;
    for option in raw_options {
        let title = option
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_string();
        let option_votes = option
            .get("replies")
            .and_then(|replies| replies.get("totalItems"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        total_votes += option_votes;
        options.push((title, option_votes));
    }

    let expires_at = object
        .get("endTime")
        .and_then(serde_json::Value::as_str)
        .or_else(|| object.get("closed").and_then(serde_json::Value::as_str))?
        .to_string();
    let expired = object.get("closed").is_some_and(|value| match value {
        serde_json::Value::Bool(expired) => *expired,
        serde_json::Value::String(timestamp) => !timestamp.trim().is_empty(),
        _ => false,
    }) || chrono::DateTime::parse_from_rfc3339(&expires_at)
        .ok()
        .is_some_and(|timestamp| timestamp.with_timezone(&Utc) <= Utc::now());

    Some(ParsedQuestionPoll {
        expires_at,
        expired,
        multiple: object.get("anyOf").is_some(),
        votes_count: total_votes,
        voters_count: object
            .get("votersCount")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(total_votes),
        options,
    })
}

fn push_alert_enabled(alerts: &PushAlerts, notification_type: NotificationType) -> bool {
    match notification_type {
        NotificationType::Mention => alerts.mention,
        NotificationType::Quote => alerts.quote,
        NotificationType::Favourite => alerts.favourite,
        NotificationType::Reblog => alerts.reblog,
        NotificationType::Follow => alerts.follow,
        NotificationType::FollowRequest => alerts.follow_request,
        NotificationType::Status => alerts.status,
        NotificationType::Poll => alerts.poll,
        NotificationType::Update => alerts.update,
        NotificationType::QuotedUpdate => alerts.quoted_update,
        NotificationType::AdminSignUp => alerts.admin_sign_up,
        NotificationType::AdminReport => alerts.admin_report,
        NotificationType::SeveredRelationships | NotificationType::ModerationWarning => false,
    }
}

fn push_notification_title(notification_type: NotificationType) -> &'static str {
    match notification_type {
        NotificationType::Mention => "New mention",
        NotificationType::Quote => "New quote",
        NotificationType::Favourite => "New favourite",
        NotificationType::Reblog => "New reblog",
        NotificationType::Follow => "New follower",
        NotificationType::FollowRequest => "New follow request",
        NotificationType::Status => "New status",
        NotificationType::Poll => "Poll update",
        NotificationType::Update => "Status updated",
        NotificationType::QuotedUpdate => "Quoted status updated",
        NotificationType::AdminSignUp => "New signup",
        NotificationType::AdminReport => "New report",
        NotificationType::SeveredRelationships => "Relationships severed",
        NotificationType::ModerationWarning => "Moderation warning",
    }
}

fn push_notification_body(notification: &crate::data::Notification) -> String {
    match notification.notification_type {
        NotificationType::Follow => {
            format!("{} followed you", notification.origin_account_address)
        }
        NotificationType::FollowRequest => {
            format!(
                "{} requested to follow you",
                notification.origin_account_address
            )
        }
        NotificationType::Mention => {
            format!("{} mentioned you", notification.origin_account_address)
        }
        NotificationType::Quote => {
            format!("{} quoted your post", notification.origin_account_address)
        }
        NotificationType::Favourite => {
            format!(
                "{} favourited your post",
                notification.origin_account_address
            )
        }
        NotificationType::Reblog => {
            format!("{} boosted your post", notification.origin_account_address)
        }
        NotificationType::Status => {
            format!(
                "{} posted a new status",
                notification.origin_account_address
            )
        }
        NotificationType::Poll => "A poll you participated in has ended".to_string(),
        NotificationType::Update => "A status you interacted with was edited".to_string(),
        NotificationType::QuotedUpdate => "A quoted status was edited".to_string(),
        NotificationType::AdminSignUp => "A new user signed up".to_string(),
        NotificationType::AdminReport => "A new report was filed".to_string(),
        NotificationType::SeveredRelationships => "Some relationships were severed".to_string(),
        NotificationType::ModerationWarning => "A moderation warning was issued".to_string(),
    }
}

fn normalize_identity_candidate(value: &str) -> &str {
    value.trim().trim_end_matches('/')
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

fn extract_activity_object_id(object: Option<&serde_json::Value>) -> Option<&str> {
    let object = object?;
    object
        .as_str()
        .or_else(|| object.get("id").and_then(|id| id.as_str()))
}

fn extract_move_object_actor_uri(activity: &serde_json::Value) -> Option<String> {
    extract_activity_object_id(activity.get("object"))
        .map(|value| value.trim().trim_end_matches('/').to_string())
}

fn extract_move_target_uri(activity: &serde_json::Value) -> Option<String> {
    extract_activity_object_id(activity.get("target"))
        .map(|value| value.trim().trim_end_matches('/').to_string())
}

fn extract_move_target_inbox_uri(activity: &serde_json::Value) -> Option<String> {
    activity
        .get("target")
        .and_then(|target| target.get("inbox"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn activity_value_contains_identity(value: &serde_json::Value, identity: &str) -> bool {
    let normalized_identity = normalize_identity_candidate(identity);
    match value {
        serde_json::Value::String(raw) => {
            normalize_identity_candidate(raw).eq_ignore_ascii_case(normalized_identity)
        }
        serde_json::Value::Array(values) => values
            .iter()
            .any(|entry| activity_value_contains_identity(entry, identity)),
        _ => false,
    }
}

fn persisted_reason_priority(reason: PersistedReason) -> u8 {
    match reason {
        PersistedReason::Own => 6,
        PersistedReason::Reposted | PersistedReason::Favourited | PersistedReason::Bookmarked => 5,
        PersistedReason::ReplyToOwn => 4,
        PersistedReason::Mentioned => 3,
        PersistedReason::Timeline => 2,
        PersistedReason::CacheOnly => 1,
    }
}

fn merge_persisted_reason(existing: PersistedReason, incoming: PersistedReason) -> PersistedReason {
    if persisted_reason_priority(existing) >= persisted_reason_priority(incoming) {
        existing
    } else {
        incoming
    }
}

fn status_changed(previous: Option<&Status>, current: &Status) -> bool {
    let Some(previous) = previous else {
        return false;
    };

    previous.content != current.content
        || previous.content_warning != current.content_warning
        || previous.visibility != current.visibility
        || previous.language != current.language
        || previous.in_reply_to_uri != current.in_reply_to_uri
        || previous.boost_of_uri != current.boost_of_uri
        || previous.quote_of_uri != current.quote_of_uri
}

fn extract_first_uri_reference(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(uri) if !uri.trim().is_empty() => Some(uri.to_string()),
        serde_json::Value::Array(values) => values.iter().find_map(extract_first_uri_reference),
        serde_json::Value::Object(_) => value
            .get("id")
            .or_else(|| value.get("href"))
            .or_else(|| value.get("url"))
            .and_then(extract_first_uri_reference),
        _ => None,
    }
}

fn normalize_actor_uri_for_compare(value: &str) -> &str {
    value.trim().trim_end_matches('/')
}

fn object_attributed_to_matches_actor(object: &serde_json::Value, actor_uri: &str) -> bool {
    let expected = normalize_actor_uri_for_compare(actor_uri);
    object
        .get("attributedTo")
        .and_then(extract_first_uri_reference)
        .is_some_and(|attributed_to| normalize_actor_uri_for_compare(&attributed_to) == expected)
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

fn local_actor_uri_from_address(protocol: &str, address: &str) -> String {
    let Some((username, domain)) = address.split_once('@') else {
        return address.to_string();
    };
    format!(
        "{}://{}/users/{}",
        if protocol.eq_ignore_ascii_case("http") {
            "http"
        } else {
            "https"
        },
        domain,
        username
    )
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
    Move,
    // Add more as needed
}

impl ActivityType {
    /// Parse activity type from string
    pub fn parse(s: &str) -> Option<Self> {
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
            "Move" => Some(Self::Move),
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
    federation_fetch_client: Option<Arc<reqwest::Client>>,
    /// Local account address for comparison
    local_address: String,
    /// Local instance protocol
    local_protocol: String,
    /// Activity delivery service for sending responses
    delivery: Option<Arc<super::ActivityDelivery>>,
    /// Real-time event bus for notifications and timeline updates.
    streaming_event_bus: Option<Arc<dyn StreamingEventBus>>,
    /// Optional Web Push sender for notification fan-out.
    web_push_sender: Option<Arc<dyn WebPushSender>>,
}

impl ActivityProcessor {
    /// Create new activity processor
    pub fn new(
        db: Arc<Database>,
        timeline_cache: Arc<TimelineCache>,
        profile_cache: Arc<ProfileCache>,
        local_address: String,
        local_protocol: String,
    ) -> Self {
        Self {
            db,
            timeline_cache,
            profile_cache,
            federation_fetch_client: None,
            local_address,
            local_protocol,
            delivery: None,
            streaming_event_bus: None,
            web_push_sender: None,
        }
    }

    /// Set the HTTP client used for remote actor discovery when processing
    /// inbound activities.
    pub fn with_federation_fetch_client(mut self, client: Arc<reqwest::Client>) -> Self {
        self.federation_fetch_client = Some(client);
        self
    }

    /// Set activity delivery service
    ///
    /// This allows the processor to send activities (like Accept) in response to incoming activities.
    pub fn with_delivery(mut self, delivery: Arc<super::ActivityDelivery>) -> Self {
        self.delivery = Some(delivery);
        self
    }

    /// Set streaming event bus for real-time notification fan-out.
    pub fn with_streaming_event_bus(
        mut self,
        streaming_event_bus: Arc<dyn StreamingEventBus>,
    ) -> Self {
        self.streaming_event_bus = Some(streaming_event_bus);
        self
    }

    pub fn with_web_push_sender(mut self, web_push_sender: Arc<dyn WebPushSender>) -> Self {
        self.web_push_sender = Some(web_push_sender);
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

        let Some(activity_type) = ActivityType::parse(activity_type_str) else {
            tracing::debug!(
                activity_type = activity_type_str,
                actor_uri,
                "Ignoring unsupported inbound ActivityPub activity type"
            );
            return Ok(());
        };

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

        if self.is_actor_locally_blocked(actor_uri).await? {
            return Err(AppError::Forbidden);
        }

        let actor_is_followee = if matches!(
            activity_type,
            ActivityType::Create | ActivityType::Update | ActivityType::Announce
        ) {
            self.is_followee(actor_uri).await
        } else {
            false
        };

        // 3. Decide whether this activity should be handled at all.
        let persistence_decision = self.decide_persistence(&activity, actor_is_followee);
        if persistence_decision == PersistenceDecision::Ignore
            && activity_type != ActivityType::Update
        {
            return Ok(());
        }

        // 4. Dispatch to type-specific handler
        match activity_type {
            ActivityType::Create => {
                self.handle_create(activity, actor_uri, persistence_decision, actor_is_followee)
                    .await
            }
            ActivityType::Update => {
                self.handle_update(activity, actor_uri, persistence_decision, actor_is_followee)
                    .await
            }
            ActivityType::Delete => self.handle_delete(activity, actor_uri).await,
            ActivityType::Follow => self.handle_follow(activity, actor_uri).await,
            ActivityType::Accept => self.handle_accept(activity, actor_uri).await,
            ActivityType::Reject => self.handle_reject(activity, actor_uri).await,
            ActivityType::Undo => self.handle_undo(activity, actor_uri).await,
            ActivityType::Like => self.handle_like(activity, actor_uri).await,
            ActivityType::Announce => {
                self.handle_announce(activity, actor_uri, persistence_decision, actor_is_followee)
                    .await
            }
            ActivityType::Block => self.handle_block(activity, actor_uri).await,
            ActivityType::Move => self.handle_move(activity, actor_uri).await,
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
            .and_then(ActivityType::parse);

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
                        // Check if the quote mentions us or quotes one of our posts.
                        if self.mentions_local_user(object)
                            || self
                                .extract_quote_uri_from_object(object)
                                .as_deref()
                                .is_some_and(|quote_uri| self.is_local_status(quote_uri))
                        {
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
                if actor_is_followee {
                    return PersistenceDecision::Persist;
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
                    if let Some(in_reply_to) = object.get("inReplyTo").and_then(|r| r.as_str())
                        && self.is_local_status(in_reply_to)
                    {
                        // Reply to our post -> Persist (notification)
                        return PersistenceDecision::Persist;
                    }
                    if self
                        .extract_quote_uri_from_object(object)
                        .as_deref()
                        .is_some_and(|quote_uri| self.is_local_status(quote_uri))
                    {
                        return PersistenceDecision::Persist;
                    }
                    // Create from followee -> Persist for durable timelines and restart safety.
                    if actor_is_followee {
                        return PersistenceDecision::Persist;
                    }
                }
                PersistenceDecision::Ignore
            }
            Some(ActivityType::Update) => {
                if let Some(object) = activity.get("object") {
                    let object_type = object
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or_default();
                    if matches!(object_type, "Note" | "Article" | "Question") {
                        if self.mentions_local_user(object) {
                            return PersistenceDecision::Persist;
                        }
                        if let Some(in_reply_to) = object.get("inReplyTo").and_then(|r| r.as_str())
                            && self.is_local_status(in_reply_to)
                        {
                            return PersistenceDecision::Persist;
                        }
                        if self
                            .extract_quote_uri_from_object(object)
                            .as_deref()
                            .is_some_and(|quote_uri| self.is_local_status(quote_uri))
                        {
                            return PersistenceDecision::Persist;
                        }
                        if actor_is_followee {
                            return PersistenceDecision::Persist;
                        }
                    } else {
                        // Profile-like updates should still refresh cache state.
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
            Some(ActivityType::Reject) => {
                // Reject of our Follow -> Persist (removes pending follow row)
                PersistenceDecision::Persist
            }
            Some(ActivityType::Undo) => {
                // Undo Follow -> Persist (removes follower)
                PersistenceDecision::Persist
            }
            Some(ActivityType::Block) => {
                // Remote blocks targeting the local account must always be processed
                // so delivery suppression and follow teardown can be enforced.
                PersistenceDecision::Persist
            }
            Some(ActivityType::Move) => {
                // Move of an account we follow should update the local follow state.
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
        actor_is_followee: bool,
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
        if object_type != "Note" && object_type != "Article" && object_type != "Question" {
            return Ok(()); // Ignore other object types for now
        }
        if !object_attributed_to_matches_actor(object, actor_uri) {
            return Err(AppError::Unauthorized);
        }

        // Extract actor address
        let actor_address = self.extract_actor_address(actor_uri);
        let activity_uri = activity.get("id").and_then(|id| id.as_str());
        let should_persist_notification =
            matches!(persistence_decision, PersistenceDecision::Persist);
        let should_cache_status = matches!(persistence_decision, PersistenceDecision::CacheOnly);
        let mentions_local = self.mentions_local_user(object);
        let replies_to_local = object
            .get("inReplyTo")
            .and_then(|r| r.as_str())
            .is_some_and(|in_reply_to| self.is_local_status(in_reply_to));
        let quote_target_uri = self.extract_quote_uri_from_object(object);
        let quotes_local = quote_target_uri
            .as_deref()
            .is_some_and(|quote_uri| self.is_local_status(quote_uri));

        if should_persist_notification {
            let persisted_reason = if mentions_local || replies_to_local || quotes_local {
                PersistedReason::Mentioned
            } else {
                PersistedReason::Timeline
            };
            if let Some(status) = self
                .upsert_remote_status_from_object(object, actor_uri, persisted_reason, false)
                .await?
            {
                self.publish_remote_status_update(&status, actor_is_followee)
                    .await;
            }
        }

        // 3. Check for mentions -> create notification
        if should_persist_notification && mentions_local {
            // Get the status URI
            let status_uri = object
                .get("id")
                .and_then(|id| id.as_str())
                .map(|s| s.to_string());

            // Create mention notification
            let notification = crate::data::Notification {
                id: crate::data::EntityId::new_string(),
                notification_type: NotificationType::Mention,
                origin_account_address: actor_address.clone(),
                status_uri,
                read: false,
                created_at: chrono::Utc::now(),
            };

            self.insert_notification_and_publish(&notification, activity_uri)
                .await?;
        }

        // 4. Check if reply to our post -> create notification
        if should_persist_notification && replies_to_local {
            // Get the status URI
            let status_uri = object
                .get("id")
                .and_then(|id| id.as_str())
                .map(|s| s.to_string());

            // Create reply notification (if not already created as mention)
            if !mentions_local {
                let notification = crate::data::Notification {
                    id: crate::data::EntityId::new_string(),
                    notification_type: NotificationType::Mention, // Replies are also mentions
                    origin_account_address: actor_address.clone(),
                    status_uri,
                    read: false,
                    created_at: chrono::Utc::now(),
                };

                self.insert_notification_and_publish(&notification, activity_uri)
                    .await?;
            }
        }

        if should_persist_notification && quotes_local {
            let status_uri = object
                .get("id")
                .and_then(|id| id.as_str())
                .map(|s| s.to_string());
            let notification = crate::data::Notification {
                id: crate::data::EntityId::new_string(),
                notification_type: NotificationType::Quote,
                origin_account_address: actor_address.clone(),
                status_uri,
                read: false,
                created_at: chrono::Utc::now(),
            };

            self.insert_notification_and_publish(&notification, activity_uri)
                .await?;
        }

        // 5. Cache followee posts without persisting to DB.
        if should_cache_status
            && let Some(cached_status) = self.cache_status_from_object(object, actor_uri).await
        {
            self.publish_cached_status_update(&cached_status, true)
                .await;
        }

        Ok(())
    }

    /// Handle Update activity (profile update)
    async fn handle_update(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
        persistence_decision: PersistenceDecision,
        actor_is_followee: bool,
    ) -> Result<(), AppError> {
        let object = activity
            .get("object")
            .ok_or_else(|| AppError::Validation("Missing object in Update".to_string()))?;
        let object_type = object
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or_default();

        if matches!(object_type, "Note" | "Article" | "Question") {
            if !object_attributed_to_matches_actor(object, actor_uri) {
                return Err(AppError::Unauthorized);
            }
            let mentions_local = self.mentions_local_user(object);
            let replies_to_local = object
                .get("inReplyTo")
                .and_then(|r| r.as_str())
                .is_some_and(|in_reply_to| self.is_local_status(in_reply_to));
            let quote_target_uri = self.extract_quote_uri_from_object(object);
            let quotes_local = quote_target_uri
                .as_deref()
                .is_some_and(|quote_uri| self.is_local_status(quote_uri));
            let status_uri = object.get("id").and_then(|id| id.as_str());
            let existing_status = if let Some(status_uri) = status_uri {
                self.db.get_status_by_uri(status_uri).await?
            } else {
                None
            };
            let is_newly_persisted = existing_status.is_none();
            let known_in_db = existing_status.is_some();
            let known_in_cache = if let Some(status_uri) = status_uri {
                self.timeline_cache.get_by_uri(status_uri).await.is_some()
            } else {
                false
            };

            let should_persist_status =
                matches!(persistence_decision, PersistenceDecision::Persist)
                    || known_in_db
                    || mentions_local
                    || replies_to_local
                    || quotes_local;
            let should_cache_status =
                matches!(persistence_decision, PersistenceDecision::CacheOnly)
                    || actor_is_followee
                    || known_in_cache;

            if should_persist_status {
                let persisted_reason = if mentions_local || replies_to_local || quotes_local {
                    PersistedReason::Mentioned
                } else {
                    PersistedReason::Timeline
                };
                if let Some(status) = self
                    .upsert_remote_status_from_object(object, actor_uri, persisted_reason, true)
                    .await?
                {
                    let activity_uri = activity.get("id").and_then(|id| id.as_str());
                    if status_changed(existing_status.as_ref(), &status)
                        && self.local_user_interacts_with_status(&status.id).await?
                    {
                        let notification = crate::data::Notification {
                            id: crate::data::EntityId::new_string(),
                            notification_type: NotificationType::Update,
                            origin_account_address: status.account_address.clone(),
                            status_uri: Some(status.uri.clone()),
                            read: false,
                            created_at: chrono::Utc::now(),
                        };
                        self.insert_notification_and_publish(&notification, activity_uri)
                            .await?;
                    }
                    if status_changed(existing_status.as_ref(), &status) {
                        for quote_status in self
                            .db
                            .get_local_statuses_by_quote_of_uri(&status.uri)
                            .await?
                        {
                            let notification = crate::data::Notification {
                                id: crate::data::EntityId::new_string(),
                                notification_type: NotificationType::QuotedUpdate,
                                origin_account_address: status.account_address.clone(),
                                status_uri: Some(quote_status.uri.clone()),
                                read: false,
                                created_at: chrono::Utc::now(),
                            };
                            self.insert_notification_and_publish(&notification, activity_uri)
                                .await?;
                        }
                    }
                    if is_newly_persisted
                        && quote_target_uri
                            .as_deref()
                            .is_some_and(|quote_uri| self.is_local_status(quote_uri))
                    {
                        let notification = crate::data::Notification {
                            id: crate::data::EntityId::new_string(),
                            notification_type: NotificationType::Quote,
                            origin_account_address: status.account_address.clone(),
                            status_uri: Some(status.uri.clone()),
                            read: false,
                            created_at: chrono::Utc::now(),
                        };
                        self.insert_notification_and_publish(&notification, activity_uri)
                            .await?;
                    }
                    self.publish_remote_status_update(&status, actor_is_followee)
                        .await;
                }
            } else if should_cache_status
                && let Some(cached_status) = self.cache_status_from_object(object, actor_uri).await
            {
                self.publish_cached_status_update(&cached_status, actor_is_followee)
                    .await;
            }
        } else {
            self.profile_cache
                .update_from_activity(actor_uri, activity)
                .await;
            if let Some(profile) = self.profile_cache.get_by_uri(actor_uri).await {
                self.db
                    .upsert_remote_profile(&crate::data::RemoteProfile::from(profile.as_ref()))
                    .await?;
            }
        }
        Ok(())
    }

    async fn local_user_interacts_with_status(&self, status_id: &str) -> Result<bool, AppError> {
        Ok(self.db.is_reposted(status_id).await?
            || self.db.is_favourited(status_id).await?
            || self.db.is_bookmarked(status_id).await?)
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
            let actor_is_followee = self.is_followee(actor_uri).await;
            let cached_status = self.timeline_cache.get_by_uri(&uri).await;
            let persisted_status = self.db.get_status_by_uri(&uri).await?;

            if let Some(status) = persisted_status {
                if !status.is_local
                    && follow_addresses_match(
                        &actor_address,
                        &status.account_address,
                        actor_scheme.as_deref(),
                    )
                {
                    if cached_status.is_some() {
                        self.timeline_cache.remove_by_uri(&uri).await;
                    }
                    self.publish_remote_status_delete(&status, actor_is_followee)
                        .await;
                    self.db.delete_status(&status.id).await?;
                    return Ok(());
                } else if !status.is_local {
                    tracing::debug!(
                        "Delete actor {} does not own persisted status {}, ignoring",
                        actor_address,
                        uri
                    );
                }
            }

            if let Some(cached_status) = cached_status {
                if follow_addresses_match(
                    &actor_address,
                    &cached_status.account_address,
                    actor_scheme.as_deref(),
                ) {
                    self.publish_cached_status_delete(&cached_status, actor_is_followee)
                        .await;
                    self.timeline_cache.remove_by_uri(&uri).await;
                } else {
                    tracing::debug!(
                        "Delete actor {} does not own cached status {}, ignoring",
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

        // 2. Resolve actor metadata for later Accept delivery.
        let activity_actor = activity
            .get("actor")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::String(actor_uri.to_string()));
        let mut canonical_actor_uri = actor_uri.to_string();
        let inbox_uri = if let Some(inbox_uri) = activity_actor
            .get("inbox")
            .and_then(|i| i.as_str())
            .map(|s| s.to_string())
        {
            if let Some(embedded_actor_uri) = activity_actor.get("id").and_then(|id| id.as_str()) {
                canonical_actor_uri = embedded_actor_uri.to_string();
            }
            crate::api::mastodon::federation_delivery::validate_actor_and_inbox_urls(
                &canonical_actor_uri,
                &inbox_uri,
            )
            .await?;
            inbox_uri
        } else if let Some(client) = &self.federation_fetch_client {
            match crate::api::mastodon::federation_delivery::resolve_remote_actor_and_inbox_with_dependencies(
                self.db.as_ref(),
                self.profile_cache.as_ref(),
                client.as_ref(),
                actor_uri,
            )
            .await
            {
                Ok((resolved_actor_uri, resolved_inbox_uri)) => {
                    canonical_actor_uri = resolved_actor_uri;
                    resolved_inbox_uri
                }
                Err(error) => {
                    tracing::warn!(
                        actor_uri,
                        %error,
                        "failed to resolve canonical actor inbox for Follow"
                    );
                    return Err(error);
                }
            }
        } else {
            return Err(AppError::Federation(
                "Cannot process Follow without actor inbox metadata or federation fetch client"
                    .to_string(),
            ));
        };

        // Extract actor address from URI
        let actor_address = self.extract_actor_address(&canonical_actor_uri);

        // Get the Follow activity ID
        let follow_activity_uri = activity
            .get("id")
            .and_then(|id| id.as_str())
            .unwrap_or(actor_uri)
            .to_string();

        // 3. Add to followers table
        let follower = crate::data::Follower {
            id: crate::data::EntityId::new_string(),
            follower_address: actor_address.clone(),
            actor_uri: Some(canonical_actor_uri),
            inbox_uri: inbox_uri.clone(),
            uri: follow_activity_uri.clone(),
            created_at: chrono::Utc::now(),
        };

        // 4. Queue Accept activity before committing the follower row so
        // remote state does not advance unless we can actually acknowledge it.
        if let Some(ref delivery) = self.delivery {
            delivery
                .queue_accept(self.db.as_ref(), &follow_activity_uri, &inbox_uri)
                .await?;
            tracing::info!("Queued Accept for {}", inbox_uri);
        } else {
            tracing::warn!("No delivery service configured, cannot send Accept");
        }

        self.db.insert_follower(&follower).await?;

        // 5. Create notification
        let notification = crate::data::Notification {
            id: crate::data::EntityId::new_string(),
            notification_type: NotificationType::Follow,
            origin_account_address: actor_address,
            status_uri: None,
            read: false,
            created_at: chrono::Utc::now(),
        };

        self.insert_notification_and_publish(
            &notification,
            activity.get("id").and_then(|id| id.as_str()),
        )
        .await?;

        Ok(())
    }

    /// Handle Block activity targeting the local account.
    async fn handle_block(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        let object = activity
            .get("object")
            .ok_or_else(|| AppError::Validation("Missing object in Block".to_string()))?;
        let Some(target) = object
            .as_str()
            .or_else(|| object.get("id").and_then(|id| id.as_str()))
        else {
            return Err(AppError::Validation("Missing block target".to_string()));
        };

        if !is_local_follow_target(&self.local_address, &self.local_protocol, target) {
            return Ok(());
        }

        self.db.record_remote_block(actor_uri).await?;
        let actor_address = self.extract_actor_address(actor_uri);
        let actor_default_port = url::Url::parse(actor_uri)
            .ok()
            .and_then(|url| default_port_for_scheme(url.scheme()));
        let _ = self
            .db
            .delete_follow(&actor_address, actor_default_port)
            .await;
        let _ = self
            .db
            .delete_follower(&actor_address, actor_default_port)
            .await;
        tracing::info!(actor_uri, "Recorded remote block");
        Ok(())
    }

    /// Handle Accept activity (follow accepted)
    async fn handle_accept(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        let actor_address = self.extract_actor_address(actor_uri);
        let actor_default_port = url::Url::parse(actor_uri)
            .ok()
            .and_then(|url| default_port_for_scheme(url.scheme()));
        let Some(existing_follow) = self
            .db
            .get_follow(&actor_address, actor_default_port)
            .await?
        else {
            tracing::debug!(
                "Ignoring Accept from {} without a matching follow row",
                actor_uri
            );
            return Ok(());
        };

        if let Some(object_follow_uri) = extract_activity_object_id(activity.get("object"))
            && existing_follow.uri != object_follow_uri
        {
            tracing::debug!(
                "Ignoring Accept from {} because Follow URI {} does not match stored {}",
                actor_uri,
                object_follow_uri,
                existing_follow.uri
            );
            return Ok(());
        }

        self.db
            .mark_follow_accepted(&actor_address, actor_uri, actor_default_port)
            .await?;

        if let Some(client) = &self.federation_fetch_client {
            let _ = crate::api::mastodon::federation_delivery::resolve_remote_actor_and_inbox_with_dependencies(
                self.db.as_ref(),
                self.profile_cache.as_ref(),
                client.as_ref(),
                actor_uri,
            )
            .await;
        }

        Ok(())
    }

    /// Handle Reject activity (follow rejected)
    async fn handle_reject(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        let actor_address = self.extract_actor_address(actor_uri);
        let actor_default_port = url::Url::parse(actor_uri)
            .ok()
            .and_then(|url| default_port_for_scheme(url.scheme()));
        let Some(existing_follow) = self
            .db
            .get_follow(&actor_address, actor_default_port)
            .await?
        else {
            tracing::debug!(
                "Ignoring Reject from {} without a matching follow row",
                actor_uri
            );
            return Ok(());
        };

        if let Some(object_follow_uri) = extract_activity_object_id(activity.get("object"))
            && existing_follow.uri != object_follow_uri
        {
            tracing::debug!(
                "Ignoring Reject from {} because Follow URI {} does not match stored {}",
                actor_uri,
                object_follow_uri,
                existing_follow.uri
            );
            return Ok(());
        }

        self.db
            .delete_follow(&actor_address, actor_default_port)
            .await?;
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
                        self.remove_remote_interaction_for_undo(obj, &actor_address)
                            .await?;
                        self.remove_notification_for_undo(obj, &actor_address)
                            .await?;
                        Ok(())
                    }
                    "Block" => {
                        let target = obj
                            .get("object")
                            .and_then(|value| {
                                value
                                    .as_str()
                                    .or_else(|| value.get("id").and_then(|id| id.as_str()))
                            })
                            .unwrap_or_default();
                        if is_local_follow_target(&self.local_address, &self.local_protocol, target)
                        {
                            self.db.remove_remote_block(actor_uri).await?;
                        }
                        Ok(())
                    }
                    _ => Ok(()),
                }
            } else if let Some(follow_uri) = obj.as_str() {
                // Compact Undo representation where object is the activity URI.
                let removed_follower = self
                    .db
                    .delete_follower_by_address_and_uri(
                        &actor_address,
                        follow_uri,
                        actor_default_port,
                    )
                    .await?;
                let removed_interactions = self
                    .remove_remote_interaction_for_undo_activity_uri(follow_uri, &actor_address)
                    .await?;
                let removed_notifications = if removed_follower || removed_interactions {
                    self.db
                        .delete_notifications_by_activity_uri(follow_uri)
                        .await?
                } else {
                    0
                };
                if removed_follower || removed_interactions || removed_notifications > 0 {
                    return Ok(());
                }

                // Fallback to Follow activity URI semantics for older implementations.
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
        let local_status = self.db.get_status_by_uri(object).await?;
        if let Some(status) = local_status.as_ref().filter(|status| status.is_local) {
            self.db
                .upsert_remote_favourite(
                    &status.id,
                    &actor_address,
                    activity.get("id").and_then(|id| id.as_str()),
                )
                .await?;
        }

        // 2. Create notification
        let notification = crate::data::Notification {
            id: crate::data::EntityId::new_string(),
            notification_type: NotificationType::Favourite,
            origin_account_address: actor_address,
            status_uri: Some(object.to_string()),
            read: false,
            created_at: chrono::Utc::now(),
        };

        self.insert_notification_and_publish(
            &notification,
            activity.get("id").and_then(|id| id.as_str()),
        )
        .await?;
        if let Some(status) = local_status.as_ref().filter(|status| status.is_local) {
            self.publish_local_status_update(&status).await;
        }

        Ok(())
    }

    /// Handle Move activity.
    async fn handle_move(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
    ) -> Result<(), AppError> {
        let Some(object_actor_uri) = extract_move_object_actor_uri(&activity) else {
            return Err(AppError::Validation("Missing object in Move".to_string()));
        };
        if !normalize_identity_candidate(&object_actor_uri)
            .eq_ignore_ascii_case(normalize_identity_candidate(actor_uri))
        {
            tracing::debug!(
                actor_uri,
                object_actor_uri,
                "Ignoring Move because object does not match actor"
            );
            return Ok(());
        }

        let Some(target_uri) = extract_move_target_uri(&activity) else {
            return Err(AppError::Validation("Missing target in Move".to_string()));
        };
        if normalize_identity_candidate(&target_uri)
            .eq_ignore_ascii_case(normalize_identity_candidate(actor_uri))
        {
            tracing::debug!(actor_uri, "Ignoring Move because target equals actor");
            return Ok(());
        }

        let embedded_target = activity.get("target").filter(|value| value.is_object());
        if let Some(target) = embedded_target {
            let Some(also_known_as) = target.get("alsoKnownAs") else {
                tracing::debug!(
                    actor_uri,
                    target_uri,
                    "Ignoring Move because embedded target lacks alsoKnownAs"
                );
                return Ok(());
            };
            if !activity_value_contains_identity(also_known_as, actor_uri) {
                tracing::debug!(
                    actor_uri,
                    target_uri,
                    "Ignoring Move because embedded target alsoKnownAs does not reference actor"
                );
                return Ok(());
            }
        }

        let actor_address = self.extract_actor_address(actor_uri);
        let actor_default_port = url::Url::parse(actor_uri)
            .ok()
            .and_then(|url| default_port_for_scheme(url.scheme()));
        let Some(_existing_follow) = self
            .db
            .get_follow(&actor_address, actor_default_port)
            .await?
        else {
            tracing::debug!(actor_uri, "Ignoring Move without an existing follow row");
            return Ok(());
        };

        let target_default_port = url::Url::parse(&target_uri)
            .ok()
            .and_then(|url| default_port_for_scheme(url.scheme()));
        let target_address = self.extract_actor_address(&target_uri);

        if self
            .db
            .get_follow(&target_address, target_default_port)
            .await?
            .is_some()
        {
            self.db
                .delete_follow(&actor_address, actor_default_port)
                .await?;
            return Ok(());
        }

        let target_inbox_uri = if let Some(inbox_uri) = extract_move_target_inbox_uri(&activity) {
            if embedded_target.is_none() {
                let Some(client) = &self.federation_fetch_client else {
                    return Err(AppError::Federation(
                        "Cannot verify Move target without federation fetch client".to_string(),
                    ));
                };
                let actor_document = fetch_remote_actor_document(client, &target_uri).await?;
                let Some(also_known_as) = actor_document.get("alsoKnownAs") else {
                    tracing::debug!(
                        actor_uri,
                        target_uri,
                        "Ignoring Move because target actor document lacks alsoKnownAs"
                    );
                    return Ok(());
                };
                if !activity_value_contains_identity(also_known_as, actor_uri) {
                    tracing::debug!(
                        actor_uri,
                        target_uri,
                        "Ignoring Move because target actor document does not reference actor via alsoKnownAs"
                    );
                    return Ok(());
                }
            }
            inbox_uri
        } else if let Some(client) = &self.federation_fetch_client {
            let (resolved_actor_uri, resolved_inbox_uri) = crate::api::mastodon::federation_delivery::resolve_remote_actor_and_inbox_with_dependencies(
                self.db.as_ref(),
                self.profile_cache.as_ref(),
                client.as_ref(),
                &target_uri,
            )
            .await?;
            if !normalize_identity_candidate(&resolved_actor_uri)
                .eq_ignore_ascii_case(normalize_identity_candidate(&target_uri))
            {
                tracing::debug!(
                    actor_uri,
                    target_uri,
                    resolved_actor_uri,
                    "Ignoring Move because resolved target actor differs from activity target"
                );
                return Ok(());
            }
            let actor_document = fetch_remote_actor_document(client, &resolved_actor_uri).await?;
            let Some(also_known_as) = actor_document.get("alsoKnownAs") else {
                tracing::debug!(
                    actor_uri,
                    target_uri,
                    "Ignoring Move because resolved target actor document lacks alsoKnownAs"
                );
                return Ok(());
            };
            if !activity_value_contains_identity(also_known_as, actor_uri) {
                tracing::debug!(
                    actor_uri,
                    target_uri,
                    "Ignoring Move because resolved target actor document does not reference actor via alsoKnownAs"
                );
                return Ok(());
            }
            resolved_inbox_uri
        } else {
            return Err(AppError::Federation(
                "Cannot process Move without federation fetch client".to_string(),
            ));
        };

        let Some(delivery) = &self.delivery else {
            return Err(AppError::Federation(
                "Cannot process Move without outbound delivery".to_string(),
            ));
        };

        let local_actor_uri =
            local_actor_uri_from_address(&self.local_protocol, &self.local_address);
        let follow_activity_uri = format!(
            "{}/follow/{}",
            local_actor_uri,
            crate::data::EntityId::new_string()
        );
        let inserted = self
            .db
            .insert_follow_if_absent(
                &crate::data::Follow {
                    id: crate::data::EntityId::new_string(),
                    target_address: target_address.clone(),
                    actor_uri: Some(target_uri.clone()),
                    uri: follow_activity_uri.clone(),
                    created_at: chrono::Utc::now(),
                },
                target_default_port,
            )
            .await?;

        if inserted {
            if let Err(error) = delivery
                .queue_follow_with_id(
                    self.db.as_ref(),
                    &follow_activity_uri,
                    &target_uri,
                    &target_inbox_uri,
                )
                .await
            {
                let _ = self
                    .db
                    .delete_follow(&target_address, target_default_port)
                    .await;
                return Err(error);
            }
        }

        self.db
            .delete_follow(&actor_address, actor_default_port)
            .await?;
        Ok(())
    }

    /// Handle Announce activity (boost)
    async fn handle_announce(
        &self,
        activity: serde_json::Value,
        actor_uri: &str,
        persistence_decision: PersistenceDecision,
        actor_is_followee: bool,
    ) -> Result<(), AppError> {
        let object = activity
            .get("object")
            .ok_or_else(|| AppError::Validation("Missing object in Announce".to_string()))?;

        // Extract actor address
        let actor_address = self.extract_actor_address(actor_uri);

        // Check if it's a quote boost (embedded object) or regular boost (URI)
        if object.is_object() {
            // Quote boost: Announce with embedded Note/Article
            if !object_attributed_to_matches_actor(object, actor_uri) {
                return Err(AppError::Unauthorized);
            }
            let mentions_local = self.mentions_local_user(object);
            let quote_target_uri = self.extract_quote_uri_from_object(object);
            let quotes_local = quote_target_uri
                .as_deref()
                .is_some_and(|quote_uri| self.is_local_status(quote_uri));

            if mentions_local || quotes_local || actor_is_followee {
                let persisted_reason = if mentions_local || quotes_local {
                    PersistedReason::Mentioned
                } else {
                    PersistedReason::Timeline
                };
                if let Some(status) = self
                    .upsert_remote_status_from_object(object, actor_uri, persisted_reason, false)
                    .await?
                {
                    self.publish_remote_status_update(&status, actor_is_followee)
                        .await;
                }

                let status_uri = object
                    .get("id")
                    .and_then(|id| id.as_str())
                    .map(|s| s.to_string());

                if quotes_local {
                    let notification = crate::data::Notification {
                        id: crate::data::EntityId::new_string(),
                        notification_type: NotificationType::Quote,
                        origin_account_address: actor_address.clone(),
                        status_uri: status_uri.clone(),
                        read: false,
                        created_at: chrono::Utc::now(),
                    };

                    self.insert_notification_and_publish(
                        &notification,
                        activity.get("id").and_then(|id| id.as_str()),
                    )
                    .await?;
                }

                if mentions_local {
                    let notification = crate::data::Notification {
                        id: crate::data::EntityId::new_string(),
                        notification_type: NotificationType::Mention,
                        origin_account_address: actor_address,
                        status_uri,
                        read: false,
                        created_at: chrono::Utc::now(),
                    };

                    self.insert_notification_and_publish(
                        &notification,
                        activity.get("id").and_then(|id| id.as_str()),
                    )
                    .await?;
                }
            }
            // Otherwise ignore non-relevant quote Announce objects.
        } else if let Some(object_uri) = object.as_str() {
            // Regular boost: just a URI reference
            // Check if it's our status being boosted
            if self.is_local_status(object_uri) {
                let mut boosted_status = None;
                if let Some(status) = self
                    .db
                    .get_status_by_uri(object_uri)
                    .await?
                    .filter(|status| status.is_local)
                {
                    self.db
                        .upsert_remote_repost(
                            &status.id,
                            &actor_address,
                            activity.get("id").and_then(|id| id.as_str()),
                        )
                        .await?;
                    boosted_status = Some(status);
                }
                // Create reblog notification for boost of our status
                let notification = crate::data::Notification {
                    id: crate::data::EntityId::new_string(),
                    notification_type: NotificationType::Reblog,
                    origin_account_address: actor_address,
                    status_uri: Some(object_uri.to_string()),
                    read: false,
                    created_at: chrono::Utc::now(),
                };

                self.insert_notification_and_publish(
                    &notification,
                    activity.get("id").and_then(|id| id.as_str()),
                )
                .await?;
                if let Some(status) = boosted_status.as_ref() {
                    self.publish_local_status_update(status).await;
                }
            } else if matches!(persistence_decision, PersistenceDecision::Persist)
                && actor_is_followee
                && let Some(status) = self
                    .upsert_remote_announce_status(
                        &activity,
                        actor_uri,
                        object_uri,
                        PersistedReason::Timeline,
                    )
                    .await?
            {
                self.publish_remote_status_update(&status, true).await;
            }
        }

        Ok(())
    }

    async fn upsert_remote_announce_status(
        &self,
        activity: &serde_json::Value,
        actor_uri: &str,
        object_uri: &str,
        persisted_reason: PersistedReason,
    ) -> Result<Option<Status>, AppError> {
        let Some(activity_uri) = activity.get("id").and_then(|id| id.as_str()) else {
            return Ok(None);
        };

        let created_at = activity
            .get("published")
            .and_then(|published| published.as_str())
            .and_then(|published| DateTime::parse_from_rfc3339(published).ok())
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        let visibility = StatusVisibility::parse(&self.extract_visibility(activity))
            .unwrap_or(StatusVisibility::Private);
        let account_address = self.extract_actor_address(actor_uri);

        let cached = CachedStatus {
            id: activity_uri.to_string(),
            uri: activity_uri.to_string(),
            content: String::new(),
            account_address: account_address.clone(),
            created_at,
            visibility: visibility.as_str().to_string(),
            attachments: Vec::new(),
            reply_to_uri: None,
            boost_of_uri: Some(object_uri.to_string()),
            quote_of_uri: None,
        };
        self.timeline_cache.insert(cached).await;

        let mut status = Status {
            id: activity_uri.to_string(),
            uri: activity_uri.to_string(),
            content: String::new(),
            content_warning: None,
            visibility,
            language: None,
            account_address,
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: Some(object_uri.to_string()),
            quote_of_uri: None,
            persisted_reason,
            created_at,
            fetched_at: Some(Utc::now()),
        };

        if let Some(existing) = self.db.get_status_by_uri(activity_uri).await? {
            if existing.is_local {
                return Ok(Some(existing));
            }
            status.id = existing.id.clone();
            status.created_at = existing.created_at;
            status.persisted_reason =
                merge_persisted_reason(existing.persisted_reason, persisted_reason);
            self.db.update_status(&status).await?;
            return Ok(Some(status));
        }

        self.db.insert_status(&status).await?;
        Ok(Some(status))
    }

    async fn upsert_remote_status_from_object(
        &self,
        object: &serde_json::Value,
        actor_uri: &str,
        persisted_reason: PersistedReason,
        capture_edit_snapshot: bool,
    ) -> Result<Option<Status>, AppError> {
        let Some(status_uri) = object.get("id").and_then(|id| id.as_str()) else {
            return Ok(None);
        };

        let cached = self.cached_status_from_object(object, actor_uri)?;
        self.timeline_cache.insert(cached.clone()).await;
        let quote_of_uri = self.extract_quote_uri_from_object(object);

        let mut status = Status {
            id: status_uri.to_string(),
            uri: status_uri.to_string(),
            content: cached.content.clone(),
            content_warning: object
                .get("summary")
                .and_then(|summary| summary.as_str())
                .map(str::to_string)
                .filter(|summary| !summary.trim().is_empty()),
            visibility: StatusVisibility::parse(&cached.visibility)
                .unwrap_or(StatusVisibility::Private),
            language: object
                .get("contentMap")
                .and_then(|map| map.as_object())
                .and_then(|map| map.keys().next().cloned())
                .or_else(|| {
                    object
                        .get("language")
                        .and_then(|language| language.as_str())
                        .map(str::to_string)
                }),
            account_address: cached.account_address.clone(),
            is_local: false,
            in_reply_to_uri: cached.reply_to_uri.clone(),
            boost_of_uri: cached.boost_of_uri.clone(),
            quote_of_uri,
            persisted_reason,
            created_at: cached.created_at,
            fetched_at: Some(Utc::now()),
        };

        if let Some(existing) = self.db.get_status_by_uri(status_uri).await? {
            if existing.is_local {
                return Ok(Some(existing));
            }
            status.id = existing.id.clone();
            status.created_at = existing.created_at;
            status.persisted_reason =
                merge_persisted_reason(existing.persisted_reason, persisted_reason);
            if capture_edit_snapshot
                && (existing.content != status.content
                    || existing.content_warning != status.content_warning
                    || existing.visibility != status.visibility
                    || existing.language != status.language
                    || existing.in_reply_to_uri != status.in_reply_to_uri
                    || existing.boost_of_uri != status.boost_of_uri
                    || existing.quote_of_uri != status.quote_of_uri)
            {
                self.db
                    .update_status_with_edit_snapshot(&existing, &status)
                    .await?;
            } else {
                self.db.update_status(&status).await?;
            }
            let attachments = self.remote_status_attachments_from_object(&status.id, object);
            self.db
                .replace_remote_status_attachments(&status.id, &attachments)
                .await?;
            self.replace_remote_poll_for_status(&status.id, object)
                .await?;
            return Ok(Some(status));
        }

        self.db.insert_status(&status).await?;
        let attachments = self.remote_status_attachments_from_object(&status.id, object);
        self.db
            .replace_remote_status_attachments(&status.id, &attachments)
            .await?;
        self.replace_remote_poll_for_status(&status.id, object)
            .await?;
        Ok(Some(status))
    }

    async fn replace_remote_poll_for_status(
        &self,
        status_id: &str,
        object: &serde_json::Value,
    ) -> Result<(), AppError> {
        if let Some(poll) = parse_question_poll(object) {
            self.db
                .replace_poll_for_status(
                    status_id,
                    &poll.expires_at,
                    poll.expired,
                    poll.multiple,
                    poll.votes_count,
                    poll.voters_count,
                    &poll.options,
                )
                .await?;
        } else {
            self.db.delete_poll_by_status_id(status_id).await?;
        }
        Ok(())
    }

    fn cached_status_from_object(
        &self,
        object: &serde_json::Value,
        actor_uri: &str,
    ) -> Result<CachedStatus, AppError> {
        let status_uri = object
            .get("id")
            .and_then(|id| id.as_str())
            .ok_or_else(|| AppError::Validation("Missing object id".to_string()))?;
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
                .or_else(|| object.get("name").and_then(|name| name.as_str()))
                .unwrap_or_default(),
        );

        Ok(CachedStatus {
            id: status_uri.to_string(),
            uri: status_uri.to_string(),
            content: sanitized_content,
            account_address: self.extract_actor_address(actor_uri),
            created_at,
            visibility: self.extract_visibility(object),
            attachments: self.extract_cached_attachments(object),
            reply_to_uri: object
                .get("inReplyTo")
                .and_then(|reply| reply.as_str())
                .map(str::to_string),
            boost_of_uri: None,
            quote_of_uri: self.extract_quote_uri_from_object(object),
        })
    }

    async fn cache_status_from_object(
        &self,
        object: &serde_json::Value,
        actor_uri: &str,
    ) -> Option<CachedStatus> {
        let cached_status = self.cached_status_from_object(object, actor_uri).ok()?;
        self.timeline_cache.insert(cached_status.clone()).await;
        Some(cached_status)
    }

    fn extract_quote_uri_from_object(&self, object: &serde_json::Value) -> Option<String> {
        ["quoteUri", "quoteUrl", "_misskey_quote"]
            .into_iter()
            .find_map(|key| object.get(key).and_then(extract_first_uri_reference))
    }

    fn remote_status_attachments_from_object(
        &self,
        status_id: &str,
        object: &serde_json::Value,
    ) -> Vec<crate::data::RemoteStatusAttachment> {
        let Some(values) = object
            .get("attachment")
            .and_then(serde_json::Value::as_array)
        else {
            return Vec::new();
        };

        values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                let remote_url = if let Some(url) = value.as_str() {
                    url.to_string()
                } else {
                    extract_first_uri_reference(value.get("url")?)?
                };
                let (width, height) = extract_attachment_dimensions(value);
                Some(crate::data::RemoteStatusAttachment {
                    id: format!("{status_id}:remote:{index}"),
                    status_id: status_id.to_string(),
                    remote_url: remote_url.clone(),
                    preview_url: value
                        .get("icon")
                        .and_then(|icon| icon.get("url"))
                        .and_then(extract_first_uri_reference),
                    content_type: value
                        .get("mediaType")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("application/octet-stream")
                        .to_string(),
                    description: value
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                    blurhash: value
                        .get("blurhash")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                    width,
                    height,
                    created_at: Utc::now(),
                })
            })
            .collect()
    }

    async fn remove_notification_for_undo(
        &self,
        object: &serde_json::Value,
        actor_address: &str,
    ) -> Result<(), AppError> {
        let Some(obj_type) = object.get("type").and_then(|t| t.as_str()) else {
            return Ok(());
        };

        let notification_type = match obj_type {
            "Like" => NotificationType::Favourite,
            "Announce" => NotificationType::Reblog,
            _ => return Ok(()),
        };
        let status_uri = object
            .get("object")
            .and_then(|value| value.as_str())
            .or_else(|| {
                object
                    .get("object")
                    .and_then(|value| value.get("id"))
                    .and_then(|id| id.as_str())
            });

        if let Some(activity_uri) = object.get("id").and_then(|id| id.as_str()) {
            let owned_by_actor = match obj_type {
                "Like" => self
                    .db
                    .get_remote_favourite_actor_and_status_by_activity_uri(activity_uri)
                    .await?
                    .is_none_or(|(stored_actor, _)| {
                        stored_actor.eq_ignore_ascii_case(actor_address)
                    }),
                "Announce" => self
                    .db
                    .get_remote_repost_actor_and_status_by_activity_uri(activity_uri)
                    .await?
                    .is_none_or(|(stored_actor, _)| {
                        stored_actor.eq_ignore_ascii_case(actor_address)
                    }),
                _ => true,
            };
            if !owned_by_actor {
                return Ok(());
            }
        }

        self.db
            .delete_notifications_by_identity(notification_type, actor_address, status_uri)
            .await?;
        Ok(())
    }

    async fn remove_remote_interaction_for_undo(
        &self,
        object: &serde_json::Value,
        actor_address: &str,
    ) -> Result<bool, AppError> {
        let Some(obj_type) = object.get("type").and_then(|t| t.as_str()) else {
            return Ok(false);
        };

        let status_uri = object
            .get("object")
            .and_then(|value| value.as_str())
            .or_else(|| {
                object
                    .get("object")
                    .and_then(|value| value.get("id"))
                    .and_then(|id| id.as_str())
            });
        let Some(status_uri) = status_uri else {
            return Ok(false);
        };
        let Some(status) = self.db.get_status_by_uri(status_uri).await? else {
            return Ok(false);
        };
        if !status.is_local {
            return Ok(false);
        }

        let removed = match obj_type {
            "Like" => {
                if let Some(activity_uri) = object.get("id").and_then(|id| id.as_str()) {
                    if let Some((stored_actor, stored_status_id)) = self
                        .db
                        .get_remote_favourite_actor_and_status_by_activity_uri(activity_uri)
                        .await?
                    {
                        if stored_actor.eq_ignore_ascii_case(actor_address)
                            && stored_status_id == status.id
                        {
                            self.db
                                .delete_remote_favourite_by_activity_uri(activity_uri)
                                .await?
                        } else {
                            false
                        }
                    } else {
                        self.db
                            .delete_remote_favourite_by_actor_and_status(actor_address, &status.id)
                            .await?
                    }
                } else {
                    self.db
                        .delete_remote_favourite_by_actor_and_status(actor_address, &status.id)
                        .await?
                }
            }
            "Announce" => {
                if let Some(activity_uri) = object.get("id").and_then(|id| id.as_str()) {
                    if self
                        .remove_remote_announce_status_by_activity_uri(activity_uri, actor_address)
                        .await?
                    {
                        return Ok(true);
                    }
                    if let Some((stored_actor, stored_status_id)) = self
                        .db
                        .get_remote_repost_actor_and_status_by_activity_uri(activity_uri)
                        .await?
                    {
                        if stored_actor.eq_ignore_ascii_case(actor_address)
                            && stored_status_id == status.id
                        {
                            self.db
                                .delete_remote_repost_by_activity_uri(activity_uri)
                                .await?
                        } else {
                            false
                        }
                    } else {
                        self.db
                            .delete_remote_repost_by_actor_and_status(actor_address, &status.id)
                            .await?
                    }
                } else {
                    self.db
                        .delete_remote_repost_by_actor_and_status(actor_address, &status.id)
                        .await?
                }
            }
            _ => false,
        };

        if removed {
            self.publish_local_status_update(&status).await;
        }
        Ok(removed)
    }

    async fn remove_remote_interaction_for_undo_activity_uri(
        &self,
        activity_uri: &str,
        actor_address: &str,
    ) -> Result<bool, AppError> {
        let mut removed_any = false;

        if let Some((stored_actor, status_id)) = self
            .db
            .get_remote_favourite_actor_and_status_by_activity_uri(activity_uri)
            .await?
            && stored_actor.eq_ignore_ascii_case(actor_address)
            && self
                .db
                .delete_remote_favourite_by_activity_uri(activity_uri)
                .await?
        {
            removed_any = true;
            if let Some(status) = self
                .db
                .get_status(&status_id)
                .await?
                .filter(|status| status.is_local)
            {
                self.publish_local_status_update(&status).await;
            }
        }

        if let Some((stored_actor, status_id)) = self
            .db
            .get_remote_repost_actor_and_status_by_activity_uri(activity_uri)
            .await?
            && stored_actor.eq_ignore_ascii_case(actor_address)
            && self
                .db
                .delete_remote_repost_by_activity_uri(activity_uri)
                .await?
        {
            removed_any = true;
            if let Some(status) = self
                .db
                .get_status(&status_id)
                .await?
                .filter(|status| status.is_local)
            {
                self.publish_local_status_update(&status).await;
            }
        }

        if self
            .remove_remote_announce_status_by_activity_uri(activity_uri, actor_address)
            .await?
        {
            removed_any = true;
        }

        Ok(removed_any)
    }

    async fn remove_remote_announce_status_by_activity_uri(
        &self,
        activity_uri: &str,
        actor_address: &str,
    ) -> Result<bool, AppError> {
        let Some(status) = self.db.get_status_by_uri(activity_uri).await? else {
            return Ok(false);
        };
        if status.is_local || status.boost_of_uri.is_none() {
            return Ok(false);
        }
        if !status.account_address.eq_ignore_ascii_case(actor_address) {
            return Ok(false);
        }

        let include_home_stream = self
            .db
            .get_all_follow_addresses()
            .await
            .map(|addresses| {
                addresses
                    .iter()
                    .any(|address| address.eq_ignore_ascii_case(&status.account_address))
            })
            .unwrap_or(false);
        self.publish_remote_status_delete(&status, include_home_stream)
            .await;
        self.timeline_cache.remove_by_uri(activity_uri).await;
        self.db.delete_status(&status.id).await?;
        Ok(true)
    }

    fn local_account_id(&self) -> &str {
        self.local_address
            .split_once('@')
            .map(|(username, _)| username)
            .unwrap_or(self.local_address.as_str())
    }

    async fn insert_notification_and_publish(
        &self,
        notification: &crate::data::Notification,
        activity_uri: Option<&str>,
    ) -> Result<(), AppError> {
        let inserted = self
            .db
            .insert_notification_if_new(notification, activity_uri)
            .await?;
        if inserted {
            self.publish_notification(notification).await;
            self.send_web_push_notification(notification).await;
        }
        Ok(())
    }

    async fn publish_notification(&self, notification: &crate::data::Notification) {
        let Some(streaming_event_bus) = &self.streaming_event_bus else {
            return;
        };

        let local_account_id = self.local_account_id().to_string();
        let event = StreamEvent::Notification {
            payload: serde_json::json!({
                "id": notification.id.as_str(),
                "type": notification.notification_type.as_str(),
                "status_uri": notification.status_uri.as_deref(),
                "origin_account_address": notification.origin_account_address.as_str(),
                "created_at": notification.created_at.to_rfc3339(),
            }),
            targets: vec![StreamTarget::User {
                account_id: local_account_id,
            }],
        };

        if let Err(error) = streaming_event_bus.publish(event).await {
            tracing::warn!(%error, "failed to publish notification stream event");
        }
    }

    async fn send_web_push_notification(&self, notification: &crate::data::Notification) {
        let Some(web_push_sender) = &self.web_push_sender else {
            return;
        };

        let Ok(Some(subscription)) = self.db.get_push_subscription().await else {
            return;
        };
        let alerts: PushAlerts = match serde_json::from_str(&subscription.alerts_json) {
            Ok(alerts) => alerts,
            Err(error) => {
                tracing::warn!(%error, "invalid stored push alerts JSON");
                return;
            }
        };
        if !push_alert_enabled(&alerts, notification.notification_type) {
            return;
        }

        let payload = PushPayload {
            notification_id: notification.id.clone(),
            notification_type: notification.notification_type.as_str().to_string(),
            title: push_notification_title(notification.notification_type).to_string(),
            body: push_notification_body(notification),
            status_uri: notification.status_uri.clone(),
        };
        if let Err(error) = web_push_sender.send(&subscription, &payload).await {
            tracing::warn!(%error, "failed to send web push notification");
        }
    }

    fn local_default_port(&self) -> Option<u16> {
        default_port_for_scheme(&self.local_protocol)
    }

    async fn stream_targets_for_remote_status(
        &self,
        account_address: &str,
        visibility: StatusVisibility,
        include_home_stream: bool,
        content: &str,
    ) -> Result<Vec<StreamTarget>, AppError> {
        let mut targets = std::collections::HashSet::new();
        let local_account_id = self.local_account_id().to_string();

        if include_home_stream {
            targets.insert(StreamTarget::User {
                account_id: local_account_id.clone(),
            });
        }

        match visibility {
            StatusVisibility::Public => {
                targets.insert(StreamTarget::Public);
                for hashtag in crate::data::extract_hashtags_from_content(content) {
                    targets.insert(StreamTarget::Hashtag { hashtag });
                }
            }
            StatusVisibility::Direct => {
                targets.insert(StreamTarget::Direct {
                    account_id: local_account_id,
                });
            }
            StatusVisibility::Unlisted | StatusVisibility::Private => {}
        }

        if !matches!(visibility, StatusVisibility::Direct) {
            for list_id in self
                .db
                .get_list_ids_for_account(account_address, self.local_default_port())
                .await?
            {
                targets.insert(StreamTarget::List { list_id });
            }
        }

        Ok(targets.into_iter().collect())
    }

    async fn publish_remote_status_update(&self, status: &Status, include_home_stream: bool) {
        let Some(streaming_event_bus) = &self.streaming_event_bus else {
            return;
        };

        let Ok(targets) = self
            .stream_targets_for_remote_status(
                status.account_address.as_str(),
                status.visibility,
                include_home_stream,
                status.content.as_str(),
            )
            .await
        else {
            return;
        };
        if targets.is_empty() {
            return;
        }

        let event = StreamEvent::Update {
            payload: serde_json::json!({
                "id": status.id.as_str(),
                "uri": status.uri.as_str(),
                "visibility": status.visibility.as_str(),
                "created_at": status.created_at.to_rfc3339(),
            }),
            targets,
        };

        if let Err(error) = streaming_event_bus.publish(event).await {
            tracing::warn!(%error, "failed to publish remote status update event");
        }
    }

    async fn publish_remote_status_delete(&self, status: &Status, include_home_stream: bool) {
        let Some(streaming_event_bus) = &self.streaming_event_bus else {
            return;
        };

        let Ok(targets) = self
            .stream_targets_for_remote_status(
                status.account_address.as_str(),
                status.visibility,
                include_home_stream,
                status.content.as_str(),
            )
            .await
        else {
            return;
        };
        if targets.is_empty() {
            return;
        }

        let event = StreamEvent::Delete {
            payload: serde_json::json!({
                "id": status.id.as_str(),
                "uri": status.uri.as_str(),
            }),
            targets,
        };

        if let Err(error) = streaming_event_bus.publish(event).await {
            tracing::warn!(%error, "failed to publish remote status delete event");
        }
    }

    async fn publish_local_status_update(&self, status: &Status) {
        let Some(streaming_event_bus) = &self.streaming_event_bus else {
            return;
        };

        let mut targets = std::collections::HashSet::new();
        let local_account_id = self.local_account_id().to_string();
        targets.insert(StreamTarget::User {
            account_id: local_account_id.clone(),
        });

        match status.visibility {
            StatusVisibility::Public => {
                targets.insert(StreamTarget::Public);
                targets.insert(StreamTarget::PublicLocal);
                for hashtag in crate::data::extract_hashtags_from_content(status.content.as_str()) {
                    targets.insert(StreamTarget::Hashtag { hashtag });
                }
            }
            StatusVisibility::Direct => {
                targets.insert(StreamTarget::Direct {
                    account_id: local_account_id,
                });
            }
            StatusVisibility::Unlisted | StatusVisibility::Private => {}
        }

        let event = StreamEvent::Update {
            payload: serde_json::json!({
                "id": status.id.as_str(),
                "uri": status.uri.as_str(),
                "visibility": status.visibility.as_str(),
                "created_at": status.created_at.to_rfc3339(),
            }),
            targets: targets.into_iter().collect(),
        };

        if let Err(error) = streaming_event_bus.publish(event).await {
            tracing::warn!(%error, "failed to publish local status update event");
        }
    }

    async fn publish_cached_status_update(
        &self,
        cached_status: &CachedStatus,
        include_home_stream: bool,
    ) {
        let Some(streaming_event_bus) = &self.streaming_event_bus else {
            return;
        };

        let visibility =
            StatusVisibility::parse(&cached_status.visibility).unwrap_or(StatusVisibility::Private);
        let Ok(targets) = self
            .stream_targets_for_remote_status(
                cached_status.account_address.as_str(),
                visibility,
                include_home_stream,
                cached_status.content.as_str(),
            )
            .await
        else {
            return;
        };
        if targets.is_empty() {
            return;
        }

        let event = StreamEvent::Update {
            payload: serde_json::json!({
                "id": cached_status.id.as_str(),
                "uri": cached_status.uri.as_str(),
                "visibility": cached_status.visibility.as_str(),
                "created_at": cached_status.created_at.to_rfc3339(),
            }),
            targets,
        };

        if let Err(error) = streaming_event_bus.publish(event).await {
            tracing::warn!(%error, "failed to publish cached remote status update event");
        }
    }

    async fn publish_cached_status_delete(
        &self,
        cached_status: &CachedStatus,
        include_home_stream: bool,
    ) {
        let Some(streaming_event_bus) = &self.streaming_event_bus else {
            return;
        };

        let visibility =
            StatusVisibility::parse(&cached_status.visibility).unwrap_or(StatusVisibility::Private);
        let Ok(targets) = self
            .stream_targets_for_remote_status(
                cached_status.account_address.as_str(),
                visibility,
                include_home_stream,
                cached_status.content.as_str(),
            )
            .await
        else {
            return;
        };
        if targets.is_empty() {
            return;
        }

        let event = StreamEvent::Delete {
            payload: serde_json::json!({
                "id": cached_status.id.as_str(),
                "uri": cached_status.uri.as_str(),
            }),
            targets,
        };

        if let Err(error) = streaming_event_bus.publish(event).await {
            tracing::warn!(%error, "failed to publish cached remote status delete event");
        }
    }

    // =========================================================================
    // Helpers
    // =========================================================================

    fn extract_visibility(&self, object: &serde_json::Value) -> String {
        const PUBLIC_AUDIENCE: &str = "https://www.w3.org/ns/activitystreams#Public";
        let Some((local_username, local_domain)) = self.local_address.split_once('@') else {
            return "private".to_string();
        };
        let local_actor_paths = [
            format!(
                "{}://{}/users/{}",
                self.local_protocol, local_domain, local_username
            ),
            format!(
                "{}://{}/@{}",
                self.local_protocol, local_domain, local_username
            ),
            format!("acct:{}", self.local_address),
            self.local_address.clone(),
        ];

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
        let contains_local_identity = |audience: &serde_json::Value| -> bool {
            if let Some(value) = audience.as_str() {
                let normalized = normalize_identity_candidate(value);
                return local_actor_paths
                    .iter()
                    .any(|candidate| normalized.eq_ignore_ascii_case(candidate));
            }
            audience
                .as_array()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(normalize_identity_candidate)
                        .any(|value| {
                            local_actor_paths
                                .iter()
                                .any(|candidate| value.eq_ignore_ascii_case(candidate))
                        })
                })
                .unwrap_or(false)
        };

        if object.get("to").is_some_and(contains_public) {
            "public".to_string()
        } else if object.get("cc").is_some_and(contains_public) {
            "unlisted".to_string()
        } else if object.get("to").is_some_and(contains_local_identity)
            || object.get("cc").is_some_and(contains_local_identity)
        {
            "direct".to_string()
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

            let Some(url) = value.get("url").and_then(extract_first_uri_reference) else {
                continue;
            };

            attachments.push(CachedAttachment {
                url,
                thumbnail_url: value
                    .get("icon")
                    .and_then(|icon| icon.get("url"))
                    .and_then(extract_first_uri_reference),
                content_type: value
                    .get("mediaType")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("application/octet-stream")
                    .to_string(),
                description: value
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                blurhash: value
                    .get("blurhash")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            });
        }

        attachments
    }

    /// Extract actor address from actor URI
    /// Example: https://example.com/users/alice -> alice@example.com
    fn extract_actor_address(&self, actor_uri: &str) -> String {
        if let Ok(parsed) = url::Url::parse(actor_uri)
            && let Some(host) = parsed.host_str()
        {
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
        // Fallback: use the full URI as address
        actor_uri.to_string()
    }

    /// Check if activity mentions the local user
    fn mentions_local_user(&self, object: &serde_json::Value) -> bool {
        let Some((local_username, local_domain)) = self.local_address.split_once('@') else {
            return false;
        };
        let local_actor_paths = [
            format!(
                "{}://{}/users/{}",
                self.local_protocol, local_domain, local_username
            ),
            format!(
                "{}://{}/@{}",
                self.local_protocol, local_domain, local_username
            ),
            format!("acct:{}", self.local_address),
            self.local_address.clone(),
        ];

        let matches_local_identity = |value: &str| -> bool {
            let normalized = normalize_identity_candidate(value);
            normalized.eq_ignore_ascii_case(&self.local_address)
                || local_actor_paths
                    .iter()
                    .any(|candidate| normalized.eq_ignore_ascii_case(candidate))
        };

        // Check cc/to/tag for local user URI or address.
        let check_audience = |audience: &serde_json::Value| -> bool {
            if let Some(value) = audience.as_str() {
                return matches_local_identity(value);
            }

            audience
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .any(matches_local_identity)
                })
                .unwrap_or(false)
        };

        // Check 'to' field
        if let Some(to) = object.get("to")
            && check_audience(to)
        {
            return true;
        }

        // Check 'cc' field
        if let Some(cc) = object.get("cc")
            && check_audience(cc)
        {
            return true;
        }

        // Check 'tag' field for Mention type
        if let Some(tag) = object.get("tag")
            && let Some(tags) = tag.as_array()
        {
            for t in tags {
                if t.get("type").and_then(|ty| ty.as_str()) == Some("Mention")
                    && let Some(href) = t.get("href").and_then(|h| h.as_str())
                    && matches_local_identity(href)
                {
                    return true;
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

    async fn is_actor_locally_blocked(&self, actor_uri: &str) -> Result<bool, AppError> {
        let actor_address = self.extract_actor_address(actor_uri);
        let actor_default_port = url::Url::parse(actor_uri)
            .ok()
            .and_then(|url| default_port_for_scheme(url.scheme()));
        if self
            .db
            .is_account_blocked(&actor_address, actor_default_port)
            .await?
        {
            return Ok(true);
        }

        self.db.is_actor_uri_blocked(actor_uri).await
    }

    /// Check if status is by local user
    fn is_local_status(&self, status_uri: &str) -> bool {
        // Check if URI contains local domain/address
        status_uri.contains(&self.local_address)
            || status_uri.contains("/users/")
                && status_uri.split("://").nth(1).is_some_and(|s| {
                    s.split('/')
                        .next()
                        .is_some_and(|domain| self.local_address.ends_with(domain))
                })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PersistenceDecision, extract_follow_target, is_local_follow_target, parse_question_poll,
    };
    use crate::data::{
        CachedProfile, CachedStatus, Database, EntityId, Follow, Follower, NotificationType,
        PersistedReason, ProfileCache, PushAlerts, PushPayload, PushSubscription, TimelineCache,
    };
    use crate::error::AppError;
    use crate::service::WebPushSender;
    use axum::async_trait;
    use chrono::Utc;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
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

        let processor = super::ActivityProcessor::new(
            db.clone(),
            timeline_cache.clone(),
            profile_cache.clone(),
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

    #[derive(Debug, Default)]
    struct MockWebPushSender {
        sent: Mutex<Vec<(PushSubscription, PushPayload)>>,
    }

    #[async_trait]
    impl WebPushSender for MockWebPushSender {
        async fn send(
            &self,
            subscription: &PushSubscription,
            payload: &PushPayload,
        ) -> Result<(), AppError> {
            self.sent
                .lock()
                .unwrap()
                .push((subscription.clone(), payload.clone()));
            Ok(())
        }

        async fn server_key(&self) -> Result<String, AppError> {
            Ok("mock-server-key".to_string())
        }
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

    #[test]
    fn parse_question_poll_extracts_options_and_counts() {
        let object = json!({
            "type": "Question",
            "endTime": "2026-01-10T00:00:00Z",
            "votersCount": 3,
            "oneOf": [
                {
                    "name": "yes",
                    "replies": { "totalItems": 2 }
                },
                {
                    "name": "no",
                    "replies": { "totalItems": 1 }
                }
            ]
        });

        let poll = parse_question_poll(&object).expect("question poll should parse");
        assert_eq!(poll.expires_at, "2026-01-10T00:00:00Z");
        assert!(!poll.multiple);
        assert_eq!(poll.votes_count, 3);
        assert_eq!(poll.voters_count, 3);
        assert_eq!(
            poll.options,
            vec![("yes".to_string(), 2), ("no".to_string(), 1)]
        );
    }

    #[tokio::test]
    async fn handle_follow_accepts_object_id_target_for_local_actor() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let activity = json!({
            "type": "Follow",
            "id": "https://remote.example/follows/1",
            "actor": {
                "id": actor_uri,
                "inbox": "https://remote.example/users/bob/inbox"
            },
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
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let delivery = Arc::new(crate::federation::ActivityDelivery::new(
            Arc::new(reqwest::Client::new()),
            "https://example.com/users/alice".to_string(),
            "https://example.com/users/alice#main-key".to_string(),
            TEST_PRIVATE_KEY_PEM.to_string(),
        ));
        let processor = processor.with_delivery(delivery);

        let actor_uri = "https://remote.example/users/bob".to_string();
        let activity = json!({
            "type": "Follow",
            "id": format!("{actor_uri}/follows/1"),
            "actor": {
                "id": actor_uri,
                "inbox": "https://remote.example/users/bob/inbox"
            },
            "object": "https://example.com/users/alice"
        });

        processor.handle_follow(activity, &actor_uri).await.unwrap();

        let jobs = db.claim_pending_delivery_jobs(10).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].inbox_url, "https://remote.example/users/bob/inbox");
        assert!(jobs[0].activity_json.contains("\"type\":\"Accept\""));
    }

    #[tokio::test]
    async fn handle_follow_sends_web_push_for_enabled_follow_alerts() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let sender = Arc::new(MockWebPushSender::default());
        let processor = processor.with_web_push_sender(sender.clone());

        db.upsert_push_subscription(
            "https://push.example.test/subscription/1",
            "p256dh-test",
            "auth-test",
            &PushAlerts {
                follow: true,
                ..PushAlerts::default()
            },
            "all",
        )
        .await
        .unwrap();

        let actor_uri = "https://remote.example/users/bob".to_string();
        let activity = json!({
            "type": "Follow",
            "id": format!("{actor_uri}/follows/1"),
            "actor": {
                "id": actor_uri,
                "inbox": "https://remote.example/users/bob/inbox"
            },
            "object": "https://example.com/users/alice"
        });

        processor
            .handle_follow(activity, "https://remote.example/users/bob")
            .await
            .unwrap();

        let sent = sender.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].0.endpoint,
            "https://push.example.test/subscription/1"
        );
        assert_eq!(sent[0].1.notification_type, "follow");
        assert_eq!(sent[0].1.title, "New follower");
        assert!(sent[0].1.body.contains("bob@remote.example followed you"));
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
    async fn remote_status_attachments_from_object_accepts_link_arrays() {
        let (processor, _db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let object = json!({
            "attachment": [{
                "type": "Document",
                "mediaType": "image/jpeg",
                "url": [{
                    "type": "Link",
                    "href": "https://remote.example/media/original.jpg"
                }],
                "icon": {
                    "url": [{
                        "type": "Link",
                        "href": "https://remote.example/media/preview.jpg"
                    }]
                },
                "name": "preview",
                "blurhash": "hash"
            }]
        });

        let attachments = processor.remote_status_attachments_from_object("status-1", &object);
        assert_eq!(attachments.len(), 1);
        assert_eq!(
            attachments[0].remote_url,
            "https://remote.example/media/original.jpg"
        );
        assert_eq!(
            attachments[0].preview_url.as_deref(),
            Some("https://remote.example/media/preview.jpg")
        );
        assert_eq!(attachments[0].blurhash.as_deref(), Some("hash"));
    }

    #[tokio::test]
    async fn handle_follow_rejects_embedded_loopback_inbox() {
        let (processor, _db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let activity = json!({
            "type": "Follow",
            "id": "https://remote.example/follows/loopback-inbox",
            "actor": {
                "id": actor_uri,
                "inbox": "http://127.0.0.1/inbox"
            },
            "object": "https://example.com/users/alice"
        });

        let result = processor.handle_follow(activity, actor_uri).await;
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[tokio::test]
    async fn process_ignores_unknown_activity_type() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let activity = json!({
            "type": "Flag",
            "id": "https://remote.example/activities/flag-1",
            "actor": "https://remote.example/users/bob",
            "object": "https://example.com/users/alice"
        });

        processor
            .process(activity, "https://remote.example/users/bob")
            .await
            .unwrap();

        assert!(db.get_all_follower_addresses().await.unwrap().is_empty());
        assert!(
            db.get_notifications(10, None, false)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn handle_update_applies_profile_cache_updates() {
        let (processor, db, _timeline_cache, profile_cache, _temp_dir) =
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

        processor
            .handle_update(activity, actor_uri, PersistenceDecision::CacheOnly, false)
            .await
            .unwrap();

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

        let persisted = db.list_remote_profiles().await.unwrap();
        let bob = persisted
            .iter()
            .find(|profile| profile.address == "bob@remote.example")
            .expect("persisted profile should exist");
        assert_eq!(bob.display_name.as_deref(), Some("Bob Updated"));
        assert_eq!(bob.note.as_deref(), Some("after"));
        assert_eq!(bob.public_key_pem, "new-key");
        assert_eq!(bob.inbox_uri, "https://remote.example/inbox-new");
        assert_eq!(bob.followers_count, Some(10));
        assert_eq!(bob.following_count, Some(20));
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
                "attributedTo": actor_uri,
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
                "attributedTo": actor_uri,
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
            id: EntityId::new_string(),
            follower_address: "bob@remote.example".to_string(),
            actor_uri: None,
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
            id: EntityId::new_string(),
            follower_address: "bob@remote.example".to_string(),
            actor_uri: None,
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
            id: EntityId::new_string(),
            follower_address: "Bob@Remote.Example".to_string(),
            actor_uri: None,
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
            id: EntityId::new_string(),
            follower_address: "bob@remote.example".to_string(),
            actor_uri: None,
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
            id: EntityId::new_string(),
            follower_address: "bob@remote.example".to_string(),
            actor_uri: None,
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
            id: EntityId::new_string(),
            follower_address: "bob@remote.example".to_string(),
            actor_uri: None,
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
    async fn process_create_from_followee_persists_status_case_insensitive_match() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/1";

        let follow = Follow {
            id: EntityId::new_string(),
            target_address: "Bob@Remote.Example".to_string(),
            actor_uri: None,
            uri: "https://example.com/users/alice/follow/1".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "attributedTo": actor_uri,
                "id": status_uri,
                "content": "<p>Hello from followee</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());
        let persisted = db
            .get_status_by_uri(status_uri)
            .await
            .unwrap()
            .expect("followee status should be persisted");
        assert_eq!(persisted.persisted_reason, PersistedReason::Timeline);
    }

    #[tokio::test]
    async fn process_announce_from_followee_persists_remote_reblog_wrapper() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let announce_activity_uri = "https://remote.example/activities/announce-1";
        let announced_status_uri = "https://remote.example/users/carol/statuses/target-1";

        db.insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: "bob@remote.example".to_string(),
            actor_uri: Some(actor_uri.to_string()),
            uri: "https://example.com/follows/bob".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

        let activity = json!({
            "type": "Announce",
            "id": announce_activity_uri,
            "actor": actor_uri,
            "published": "2026-01-07T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"],
            "object": announced_status_uri
        });

        processor.process(activity, actor_uri).await.unwrap();

        let persisted = db
            .get_status_by_uri(announce_activity_uri)
            .await
            .unwrap()
            .expect("followee announce should be persisted");
        assert_eq!(
            persisted.boost_of_uri.as_deref(),
            Some(announced_status_uri)
        );
        assert_eq!(persisted.persisted_reason, PersistedReason::Timeline);
        assert_eq!(
            timeline_cache
                .get_by_uri(announce_activity_uri)
                .await
                .and_then(|status| status.boost_of_uri.clone()),
            Some(announced_status_uri.to_string())
        );
    }

    #[tokio::test]
    async fn process_create_from_followee_with_default_https_port_actor_uri_persists_status() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example:443/users/bob";
        let status_uri = "https://remote.example:443/users/bob/statuses/port-normalized";

        let follow = Follow {
            id: EntityId::new_string(),
            target_address: "bob@remote.example".to_string(),
            actor_uri: None,
            uri: "https://example.com/users/alice/follow/port-normalized".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "attributedTo": actor_uri,
                "id": status_uri,
                "content": "<p>Hello from :443 actor URI</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());
        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn process_create_from_followee_with_explicit_default_port_follow_address_persists_status()
     {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/port-follow-row";

        let follow = Follow {
            id: EntityId::new_string(),
            target_address: "bob@remote.example:443".to_string(),
            actor_uri: None,
            uri: "https://example.com/users/alice/follow/port-follow-row".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "attributedTo": actor_uri,
                "id": status_uri,
                "content": "<p>Hello from explicit :443 follow row</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());
        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn process_create_from_followee_with_at_actor_uri_persists_status() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/@bob";
        let status_uri = "https://remote.example/@bob/statuses/2";

        let follow = Follow {
            id: EntityId::new_string(),
            target_address: "bob@remote.example".to_string(),
            actor_uri: None,
            uri: "https://example.com/users/alice/follow/2".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "attributedTo": actor_uri,
                "id": status_uri,
                "content": "<p>Hello from @bob</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());
        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn process_create_from_followee_with_accounts_actor_uri_persists_status() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/accounts/bob";
        let status_uri = "https://remote.example/accounts/bob/statuses/3";

        let follow = Follow {
            id: EntityId::new_string(),
            target_address: "bob@remote.example".to_string(),
            actor_uri: None,
            uri: "https://example.com/users/alice/follow/3".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "attributedTo": actor_uri,
                "id": status_uri,
                "content": "<p>Hello from /accounts/bob</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());
        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn process_create_from_followee_with_ipv6_actor_uri_persists_status() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://[2001:db8::1]/users/bob";
        let status_uri = "https://[2001:db8::1]/users/bob/statuses/ipv6";

        let follow = Follow {
            id: EntityId::new_string(),
            target_address: "bob@[2001:db8::1]".to_string(),
            actor_uri: None,
            uri: "https://example.com/users/alice/follow/ipv6".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "attributedTo": actor_uri,
                "id": status_uri,
                "content": "<p>Hello from IPv6 actor</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(timeline_cache.get_by_uri(status_uri).await.is_some());
        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn process_create_from_followee_sanitizes_cached_content() {
        let (processor, db, timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/sanitized";

        let follow = Follow {
            id: EntityId::new_string(),
            target_address: "bob@remote.example".to_string(),
            actor_uri: None,
            uri: "https://example.com/users/alice/follow/sanitized".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "attributedTo": actor_uri,
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
            id: EntityId::new_string(),
            target_address: "bob@remote.example".to_string(),
            actor_uri: None,
            uri: "https://example.com/users/alice/follow/3".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let create_activity = json!({
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "attributedTo": actor_uri,
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
                quote_of_uri: None,
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
            id: EntityId::new_string(),
            target_address: "bob@remote.example".to_string(),
            actor_uri: None,
            uri: "https://example.com/users/alice/follow/4".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let status = crate::data::Status {
            id: EntityId::new_string(),
            uri: status_uri.to_string(),
            content: "<p>Persisted remote status</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "bob@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Bookmarked,
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
            id: EntityId::new_string(),
            target_address: "bob@remote.example".to_string(),
            actor_uri: None,
            uri: "https://example.com/users/alice/follow/5".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let status = crate::data::Status {
            id: EntityId::new_string(),
            uri: status_uri.to_string(),
            content: "<p>Owned by another actor</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "alice@another.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Bookmarked,
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
            id: EntityId::new_string(),
            uri: status_uri.to_string(),
            content: "<p>Persisted remote status after unfollow</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "bob@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Favourited,
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
            id: EntityId::new_string(),
            uri: status_uri.to_string(),
            content: "<p>Persisted remote status</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "bob@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Bookmarked,
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
            id: EntityId::new_string(),
            uri: status_uri.to_string(),
            content: "<p>Owned by IPv6 actor</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "bob@[2001:db8::1]".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Bookmarked,
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
                quote_of_uri: None,
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
            id: EntityId::new_string(),
            uri: status_uri.to_string(),
            content: "<p>Owned by bob without explicit port</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "bob@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Bookmarked,
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
    async fn process_create_mention_persists_remote_status_and_notification() {
        let (processor, db, _timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/mention";

        let activity = json!({
            "id": "https://remote.example/activities/create-mention",
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "attributedTo": actor_uri,
                "id": status_uri,
                "content": "<p>Hello @alice@example.com</p>",
                "published": "2026-01-01T00:00:00Z",
                "tag": [{
                    "type": "Mention",
                    "href": "https://example.com/users/alice"
                }]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        let persisted = db
            .get_status_by_uri(status_uri)
            .await
            .unwrap()
            .expect("mentioned status should be persisted");
        assert_eq!(persisted.id, status_uri);
        assert_eq!(persisted.persisted_reason, PersistedReason::Mentioned);

        let notifications = db.get_notifications(10, None, false).await.unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(
            notifications[0].notification_type,
            NotificationType::Mention
        );
        assert_eq!(notifications[0].status_uri.as_deref(), Some(status_uri));
    }

    #[tokio::test]
    async fn process_update_note_updates_persisted_remote_status_and_snapshots_edit() {
        let (processor, db, _timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/editable";
        let status = crate::data::Status {
            id: status_uri.to_string(),
            uri: status_uri.to_string(),
            content: "<p>before</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "bob@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Mentioned,
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        };
        db.insert_status(&status).await.unwrap();

        let activity = json!({
            "type": "Update",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "attributedTo": actor_uri,
                "id": status_uri,
                "content": "<p>after #edited</p>",
                "summary": "cw",
                "published": "2026-01-01T00:00:00Z",
                "to": ["https://www.w3.org/ns/activitystreams#Public"]
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        let updated = db
            .get_status_by_uri(status_uri)
            .await
            .unwrap()
            .expect("updated status should exist");
        assert_eq!(updated.content, "<p>after #edited</p>");
        assert_eq!(updated.content_warning.as_deref(), Some("cw"));
        let edits = db.get_status_edits(&updated.id, 10).await.unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].1, "<p>before</p>");
    }

    #[tokio::test]
    async fn process_follow_redelivery_upserts_existing_follower() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let first = json!({
            "id": "https://remote.example/follows/1",
            "type": "Follow",
            "actor": {
                "id": actor_uri,
                "inbox": "https://remote.example/users/bob/inbox"
            },
            "object": "https://example.com/users/alice"
        });
        let second = json!({
            "id": "https://remote.example/follows/1",
            "type": "Follow",
            "actor": {
                "id": actor_uri,
                "inbox": "https://remote.example/users/bob/inbox"
            },
            "object": "https://example.com/users/alice"
        });

        processor.process(first, actor_uri).await.unwrap();
        processor.process(second, actor_uri).await.unwrap();

        assert_eq!(db.count_follower_addresses().await.unwrap(), 1);
        let notifications = db.get_notifications(10, None, false).await.unwrap();
        assert_eq!(notifications.len(), 1);
    }

    #[tokio::test]
    async fn process_accept_marks_follow_as_accepted() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let follow = Follow {
            id: EntityId::new_string(),
            target_address: "bob@remote.example".to_string(),
            actor_uri: None,
            uri: "https://example.com/users/alice/follow/accept".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Accept",
            "actor": actor_uri,
            "object": {
                "type": "Follow",
                "id": follow.uri
            }
        });

        processor.process(activity, actor_uri).await.unwrap();
        assert!(
            db.is_follow_accepted("bob@remote.example", Some(443))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn process_reject_removes_follow_row() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let follow = Follow {
            id: EntityId::new_string(),
            target_address: "bob@remote.example".to_string(),
            actor_uri: None,
            uri: "https://example.com/users/alice/follow/reject".to_string(),
            created_at: Utc::now(),
        };
        db.insert_follow(&follow).await.unwrap();

        let activity = json!({
            "type": "Reject",
            "actor": actor_uri,
            "object": {
                "type": "Follow",
                "id": follow.uri
            }
        });

        processor.process(activity, actor_uri).await.unwrap();
        assert!(
            db.get_follow("bob@remote.example", Some(443))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn process_undo_like_removes_notification_by_activity_uri() {
        let (processor, db, _temp_dir) = create_test_processor("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://example.com/users/alice/statuses/local-1";

        let like_activity = json!({
            "id": "https://remote.example/likes/1",
            "type": "Like",
            "actor": actor_uri,
            "object": status_uri
        });
        processor.process(like_activity, actor_uri).await.unwrap();
        assert_eq!(
            db.get_notifications(10, None, false).await.unwrap().len(),
            1
        );

        let undo_activity = json!({
            "type": "Undo",
            "actor": actor_uri,
            "object": {
                "type": "Like",
                "id": "https://remote.example/likes/1",
                "object": status_uri
            }
        });
        processor.process(undo_activity, actor_uri).await.unwrap();

        assert!(
            db.get_notifications(10, None, false)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn process_create_detects_mention_from_single_to_string_with_trailing_slash() {
        let (processor, db, _timeline_cache, _temp_dir) =
            create_test_processor_with_timeline("alice@example.com", "https").await;
        let actor_uri = "https://remote.example/users/bob";
        let status_uri = "https://remote.example/users/bob/statuses/single-to";

        let activity = json!({
            "id": "https://remote.example/activities/create-single-to",
            "type": "Create",
            "actor": actor_uri,
            "object": {
                "type": "Note",
                "attributedTo": actor_uri,
                "id": status_uri,
                "content": "<p>Hello</p>",
                "published": "2026-01-01T00:00:00Z",
                "to": "https://example.com/users/alice/"
            }
        });

        processor.process(activity, actor_uri).await.unwrap();

        assert!(db.get_status_by_uri(status_uri).await.unwrap().is_some());
        let notifications = db.get_notifications(10, None, false).await.unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(
            notifications[0].notification_type,
            NotificationType::Mention
        );
        assert_eq!(notifications[0].status_uri.as_deref(), Some(status_uri));
    }
}
