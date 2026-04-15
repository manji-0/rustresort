//! In-memory caches backed by process-local hash maps.
//!
//! These caches are volatile and cleared on restart.

use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::sync::RwLock;

use crate::error::AppError;

const TIMELINE_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const PROFILE_PRUNE_INTERVAL_MS: i64 = 60 * 1000;

// =============================================================================
// Cached Status (lightweight version for timeline)
// =============================================================================

/// Cached status for timeline display
///
/// This is a lightweight version of Status, only containing
/// fields needed for timeline rendering.
#[derive(Debug, Clone)]
pub struct CachedStatus {
    pub id: String,
    pub uri: String,
    pub content: String,
    /// Account address (user@domain)
    pub account_address: String,
    pub created_at: DateTime<Utc>,
    pub visibility: String,
    pub attachments: Vec<CachedAttachment>,
    pub reply_to_uri: Option<String>,
    pub boost_of_uri: Option<String>,
    pub quote_of_uri: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TimelineCursorKey {
    pub created_at: DateTime<Utc>,
    pub id: String,
}

/// Cached media attachment
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedAttachment {
    pub url: String,
    pub thumbnail_url: Option<String>,
    pub content_type: String,
    pub description: Option<String>,
    pub blurhash: Option<String>,
}

fn ttl_seconds_to_millis(ttl_seconds: u64) -> i64 {
    let max_ttl_seconds = (i64::MAX as u64) / 1000;
    let bounded_seconds = ttl_seconds.min(max_ttl_seconds);
    (bounded_seconds as i64) * 1000
}

fn extract_url(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(url) => Some(url.to_string()),
        serde_json::Value::Array(values) => values.iter().find_map(extract_url),
        serde_json::Value::Object(_) => value
            .get("url")
            .and_then(extract_url)
            .or_else(|| value.get("href").and_then(extract_url)),
        _ => None,
    }
}

fn extract_public_key_pem(actor_document: &serde_json::Value) -> Option<String> {
    actor_document
        .get("publicKeyPem")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            actor_document
                .get("publicKey")
                .and_then(|value| value.get("publicKeyPem"))
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        })
}

fn actor_bool(actor_document: &serde_json::Value, key: &str) -> Option<bool> {
    actor_document.get(key).and_then(|value| value.as_bool())
}

fn extract_explicit_port_from_domain(domain: &str) -> Option<u16> {
    let domain = domain.trim();

    if let Some(rest) = domain.strip_prefix('[') {
        let (_, tail) = rest.split_once(']')?;
        let port_str = tail.strip_prefix(':')?;
        if port_str.is_empty() || !port_str.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        return port_str.parse::<u16>().ok();
    }

    let (host_part, port_str) = domain.rsplit_once(':')?;
    if host_part.is_empty()
        || host_part.contains(':')
        || port_str.is_empty()
        || !port_str.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }

    port_str.parse::<u16>().ok()
}

fn webfinger_urls_for_domain(domain: &str, resource: &str) -> Result<Vec<url::Url>, AppError> {
    url::Url::parse(&format!("http://{}", domain)).map_err(|error| {
        AppError::Federation(format!(
            "Failed to parse remote account domain {}: {}",
            domain, error
        ))
    })?;

    let schemes: &[&str] = match extract_explicit_port_from_domain(domain) {
        Some(80) => &["http"],
        Some(443) | None => &["https"],
        Some(_) => &["https", "http"],
    };

    schemes
        .iter()
        .map(|scheme| {
            let mut url =
                url::Url::parse(&format!("{}://{}/.well-known/webfinger", scheme, domain))
                    .map_err(|error| {
                        AppError::Federation(format!(
                            "Failed to build WebFinger URL for {}: {}",
                            domain, error
                        ))
                    })?;
            url.query_pairs_mut().append_pair("resource", resource);
            Ok(url)
        })
        .collect()
}

fn is_supported_webfinger_link_type(link_type: &str) -> bool {
    let normalized = link_type.trim().to_ascii_lowercase();
    normalized.contains("activity+json")
        || (normalized.contains("ld+json") && normalized.contains("activitystreams"))
}

fn extract_actor_uri_from_webfinger(webfinger: &serde_json::Value) -> Option<String> {
    webfinger
        .get("links")
        .and_then(|value| value.as_array())
        .and_then(|links| {
            links.iter().find_map(|link| {
                let rel = link.get("rel").and_then(|value| value.as_str())?;
                if rel != "self" {
                    return None;
                }
                let link_type = link.get("type").and_then(|value| value.as_str())?;
                if !is_supported_webfinger_link_type(link_type) {
                    return None;
                }
                link.get("href")
                    .and_then(|value| value.as_str())
                    .map(|href| href.to_string())
            })
        })
}

fn parse_actor_uri_address(address: &str) -> Option<String> {
    let parsed = url::Url::parse(address.trim()).ok()?;
    match parsed.scheme() {
        "http" | "https" => Some(parsed.to_string()),
        _ => None,
    }
}

async fn discover_actor_uri(
    http_client: &reqwest::Client,
    address: &str,
) -> Result<String, AppError> {
    if let Some(actor_uri) = parse_actor_uri_address(address) {
        return Ok(actor_uri);
    }

    let (username, domain) = address.split_once('@').ok_or_else(|| {
        AppError::Validation("Invalid account address format for profile cache".to_string())
    })?;

    if username.is_empty() || domain.is_empty() {
        return Err(AppError::Validation(
            "Invalid account address format for profile cache".to_string(),
        ));
    }

    let resource = format!("acct:{}@{}", username, domain);
    let webfinger_urls = webfinger_urls_for_domain(domain, &resource)?;
    let mut last_error = None;

    for webfinger_url in webfinger_urls {
        let response = match http_client
            .get(webfinger_url.clone())
            .header("Accept", "application/jrd+json, application/json")
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                last_error = Some(AppError::Federation(format!(
                    "WebFinger request failed for {} via {}: {}",
                    resource, webfinger_url, error
                )));
                continue;
            }
        };

        if !response.status().is_success() {
            last_error = Some(AppError::Federation(format!(
                "WebFinger request failed for {} via {}: HTTP {}",
                resource,
                webfinger_url,
                response.status()
            )));
            continue;
        }

        let webfinger: serde_json::Value = match response.json().await {
            Ok(webfinger) => webfinger,
            Err(error) => {
                last_error = Some(AppError::Federation(format!(
                    "Failed to decode WebFinger response for {} via {}: {}",
                    resource, webfinger_url, error
                )));
                continue;
            }
        };

        if let Some(actor_uri) = extract_actor_uri_from_webfinger(&webfinger) {
            return Ok(actor_uri);
        }

        last_error = Some(AppError::Federation(format!(
            "WebFinger response for {} via {} did not include an ActivityPub actor URL",
            resource, webfinger_url
        )));
    }

    Err(last_error.unwrap_or_else(|| {
        AppError::Federation(format!(
            "Failed to discover actor URI from WebFinger for {}",
            resource
        ))
    }))
}

async fn fetch_actor_document(
    http_client: &reqwest::Client,
    actor_uri: &str,
) -> Result<serde_json::Value, AppError> {
    let response = http_client
        .get(actor_uri)
        .header(
            "Accept",
            "application/activity+json, application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"",
        )
        .send()
        .await
        .map_err(|error| {
            AppError::Federation(format!(
                "Actor fetch failed for {}: {}",
                actor_uri, error
            ))
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

fn build_cached_profile_from_actor(
    address: &str,
    actor_uri: &str,
    actor_document: &serde_json::Value,
) -> Option<CachedProfile> {
    let public_key_pem = extract_public_key_pem(actor_document)?;

    let canonical_actor_uri = actor_document
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or(actor_uri)
        .to_string();
    let inbox_uri = actor_document
        .get("inbox")
        .and_then(|value| value.as_str())?
        .to_string();

    if url::Url::parse(&canonical_actor_uri).is_err() || url::Url::parse(&inbox_uri).is_err() {
        return None;
    }

    let profile_fields_json = crate::profile_fields::serialize_profile_fields(
        &crate::profile_fields::extract_profile_fields_from_actor(actor_document),
    )
    .ok()
    .flatten();

    Some(CachedProfile {
        address: address.to_string(),
        uri: canonical_actor_uri,
        display_name: actor_document
            .get("name")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        note: actor_document
            .get("summary")
            .or_else(|| actor_document.get("note"))
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        profile_fields_json,
        locked: actor_bool(actor_document, "manuallyApprovesFollowers").unwrap_or(false),
        bot: actor_bool(actor_document, "bot").unwrap_or(false),
        discoverable: actor_bool(actor_document, "discoverable").unwrap_or(true),
        indexable: actor_bool(actor_document, "indexable").unwrap_or(true),
        avatar_url: actor_document.get("icon").and_then(extract_url),
        header_url: actor_document.get("image").and_then(extract_url),
        public_key_pem,
        inbox_uri,
        outbox_uri: actor_document
            .get("outbox")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        followers_count: actor_document
            .get("followersCount")
            .and_then(|value| value.as_u64()),
        following_count: actor_document
            .get("followingCount")
            .and_then(|value| value.as_u64()),
        fetched_at: Utc::now(),
    })
}

fn extract_username_from_actor_uri(actor_uri: &str) -> Option<String> {
    let parsed = url::Url::parse(actor_uri).ok()?;
    let mut parts = parsed
        .path()
        .trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty());
    let first = parts.next()?;

    if let Some(username) = first.strip_prefix('@') {
        return (!username.is_empty()).then(|| username.to_string());
    }

    if matches!(first, "users" | "accounts" | "u" | "profile") {
        let username = parts.next()?;
        return (!username.is_empty()).then(|| username.to_string());
    }

    None
}

fn actor_address_from_document(
    actor_uri: &str,
    actor_document: &serde_json::Value,
) -> Option<String> {
    let parsed = url::Url::parse(actor_uri).ok()?;
    let host = parsed.host_str()?;
    let normalized_host = host.to_ascii_lowercase();
    let authority_host = if normalized_host.contains(':') {
        format!("[{}]", normalized_host)
    } else {
        normalized_host.clone()
    };
    let authority = match parsed.port() {
        Some(port) => format!("{}:{}", authority_host, port),
        None => authority_host,
    };
    let username = actor_document
        .get("preferredUsername")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| extract_username_from_actor_uri(actor_uri))?;
    Some(format!("{}@{}", username, authority))
}

// =============================================================================
// Timeline Cache
// =============================================================================

#[derive(Debug, Clone)]
struct CachedTimelineEntry {
    status: CachedStatus,
    inserted_at_ms: i64,
}

#[derive(Default)]
struct TimelineCacheState {
    entries_by_id: HashMap<String, CachedTimelineEntry>,
    id_by_uri: HashMap<String, String>,
}

/// Timeline cache (volatile, max 2000 items)
///
/// Stores recent statuses from followees.
pub struct TimelineCache {
    state: RwLock<TimelineCacheState>,
    /// Maximum lifetime for cached timeline entries (7 days).
    ttl_ms: i64,
    /// Maximum items to keep
    max_items: usize,
}

impl TimelineCache {
    /// Create new timeline cache
    ///
    /// # Arguments
    /// * `max_items` - Maximum number of statuses to cache
    pub async fn new(max_items: usize) -> Result<Self, AppError> {
        Ok(Self {
            state: RwLock::new(TimelineCacheState::default()),
            ttl_ms: TIMELINE_TTL_MS,
            max_items: max_items.max(1),
        })
    }

    fn remove_entry_locked(state: &mut TimelineCacheState, id: &str) -> bool {
        let Some(removed) = state.entries_by_id.remove(id) else {
            return false;
        };

        if matches!(state.id_by_uri.get(&removed.status.uri), Some(mapped_id) if mapped_id == id) {
            state.id_by_uri.remove(&removed.status.uri);
        }

        true
    }

    fn update_size_metric(&self, count: usize) {
        use crate::metrics::CACHE_SIZE;
        CACHE_SIZE
            .with_label_values(&["timeline"])
            .set(count.min(i64::MAX as usize) as i64);
    }

    fn prune_expired_locked(&self, state: &mut TimelineCacheState, now_ms: i64) {
        let cutoff = now_ms.saturating_sub(self.ttl_ms);
        let expired_ids: Vec<String> = state
            .entries_by_id
            .iter()
            .filter(|(_, entry)| entry.inserted_at_ms < cutoff)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired_ids {
            Self::remove_entry_locked(state, &id);
        }
    }

    fn enforce_capacity_locked(&self, state: &mut TimelineCacheState) {
        if state.entries_by_id.len() <= self.max_items {
            return;
        }

        let mut ranked: Vec<(String, i64)> = state
            .entries_by_id
            .iter()
            .map(|(id, entry)| (id.clone(), entry.status.created_at.timestamp_millis()))
            .collect();
        ranked.sort_by(|(id_a, created_at_a), (id_b, created_at_b)| {
            created_at_b.cmp(created_at_a).then_with(|| id_b.cmp(id_a))
        });

        for (id, _) in ranked.into_iter().skip(self.max_items) {
            Self::remove_entry_locked(state, &id);
        }
    }

    /// Insert status into cache
    ///
    /// Automatically evicts oldest items when capacity is reached.
    pub async fn insert(&self, status: CachedStatus) {
        let inserted_at_ms = Utc::now().timestamp_millis();
        let now_ms = Utc::now().timestamp_millis();
        let mut state = self.state.write().await;

        self.prune_expired_locked(&mut state, now_ms);

        if let Some(existing_id) = state.id_by_uri.get(&status.uri).cloned()
            && existing_id != status.id
        {
            Self::remove_entry_locked(&mut state, &existing_id);
        }

        if let Some(existing_uri) = state
            .entries_by_id
            .get(&status.id)
            .map(|entry| entry.status.uri.clone())
            && existing_uri != status.uri
            && matches!(
                state.id_by_uri.get(&existing_uri),
                Some(mapped_id) if mapped_id == &status.id
            )
        {
            state.id_by_uri.remove(&existing_uri);
        }

        let status_id = status.id.clone();
        let status_uri = status.uri.clone();
        state.entries_by_id.insert(
            status_id.clone(),
            CachedTimelineEntry {
                status,
                inserted_at_ms,
            },
        );
        state.id_by_uri.insert(status_uri, status_id);

        self.enforce_capacity_locked(&mut state);
        let size = state.entries_by_id.len();
        drop(state);

        self.update_size_metric(size);
    }

    /// Get status by ID
    pub async fn get(&self, id: &str) -> Option<Arc<CachedStatus>> {
        let now_ms = Utc::now().timestamp_millis();
        let mut state = self.state.write().await;
        self.prune_expired_locked(&mut state, now_ms);
        let value = state
            .entries_by_id
            .get(id)
            .map(|entry| Arc::new(entry.status.clone()));

        use crate::metrics::{CACHE_HITS_TOTAL, CACHE_MISSES_TOTAL};
        if value.is_some() {
            CACHE_HITS_TOTAL.with_label_values(&["timeline"]).inc();
        } else {
            CACHE_MISSES_TOTAL.with_label_values(&["timeline"]).inc();
        }

        value
    }

    /// Get status by URI
    pub async fn get_by_uri(&self, uri: &str) -> Option<Arc<CachedStatus>> {
        let now_ms = Utc::now().timestamp_millis();
        let mut state = self.state.write().await;
        self.prune_expired_locked(&mut state, now_ms);

        let id = state.id_by_uri.get(uri).cloned()?;
        let Some(entry) = state.entries_by_id.get(&id) else {
            state.id_by_uri.remove(uri);
            return None;
        };
        Some(Arc::new(entry.status.clone()))
    }

    /// Remove status from cache
    pub async fn remove(&self, id: &str) {
        let mut state = self.state.write().await;
        if !Self::remove_entry_locked(&mut state, id) {
            return;
        }
        let size = state.entries_by_id.len();
        drop(state);

        self.update_size_metric(size);
    }

    /// Remove status from cache by ActivityPub URI.
    pub async fn remove_by_uri(&self, uri: &str) {
        let mut state = self.state.write().await;
        let Some(id) = state.id_by_uri.get(uri).cloned() else {
            return;
        };
        if !Self::remove_entry_locked(&mut state, &id) {
            return;
        }
        let size = state.entries_by_id.len();
        drop(state);

        self.update_size_metric(size);
    }

    /// Get home timeline
    ///
    /// Returns statuses from followees, sorted by created_at desc.
    ///
    /// # Arguments
    /// * `followee_addresses` - Set of addresses the user follows
    /// * `limit` - Maximum results
    /// * `max_id` - Return statuses older than this ID
    pub async fn get_home_timeline(
        &self,
        followee_addresses: &HashSet<String>,
        limit: usize,
        max_cursor: Option<&TimelineCursorKey>,
        min_cursor: Option<&TimelineCursorKey>,
    ) -> Vec<Arc<CachedStatus>> {
        if followee_addresses.is_empty() || limit == 0 {
            return Vec::new();
        }

        let now_ms = Utc::now().timestamp_millis();
        let mut state = self.state.write().await;
        self.prune_expired_locked(&mut state, now_ms);

        let mut statuses: Vec<CachedStatus> = state
            .entries_by_id
            .values()
            .map(|entry| &entry.status)
            .filter(|status| followee_addresses.contains(&status.account_address))
            .filter(|status| {
                max_cursor
                    .map(|cursor| is_before_cursor(status, cursor))
                    .unwrap_or(true)
            })
            .filter(|status| {
                min_cursor
                    .map(|cursor| is_after_cursor(status, cursor))
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        statuses.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        statuses.truncate(limit);

        statuses.into_iter().map(Arc::new).collect()
    }

    /// Get public timeline
    ///
    /// Returns all public statuses in cache.
    pub async fn get_public_timeline(
        &self,
        limit: usize,
        max_cursor: Option<&TimelineCursorKey>,
        min_cursor: Option<&TimelineCursorKey>,
    ) -> Vec<Arc<CachedStatus>> {
        if limit == 0 {
            return Vec::new();
        }

        let now_ms = Utc::now().timestamp_millis();
        let mut state = self.state.write().await;
        self.prune_expired_locked(&mut state, now_ms);

        let mut statuses: Vec<CachedStatus> = state
            .entries_by_id
            .values()
            .map(|entry| &entry.status)
            .filter(|status| status.visibility == "public")
            .filter(|status| {
                max_cursor
                    .map(|cursor| is_before_cursor(status, cursor))
                    .unwrap_or(true)
            })
            .filter(|status| {
                min_cursor
                    .map(|cursor| is_after_cursor(status, cursor))
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        statuses.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        statuses.truncate(limit);

        statuses.into_iter().map(Arc::new).collect()
    }
}

// =============================================================================
// Cached Profile
// =============================================================================

/// Cached user profile
///
/// Full profile data for followees and followers.
#[derive(Debug, Clone)]
pub struct CachedProfile {
    /// Account address (user@domain)
    pub address: String,
    /// ActivityPub actor URI
    pub uri: String,
    pub display_name: Option<String>,
    pub note: Option<String>,
    pub profile_fields_json: Option<String>,
    pub locked: bool,
    pub bot: bool,
    pub discoverable: bool,
    pub indexable: bool,
    pub avatar_url: Option<String>,
    pub header_url: Option<String>,
    /// RSA public key for signature verification
    pub public_key_pem: String,
    /// Inbox URI for activity delivery
    pub inbox_uri: String,
    /// Outbox URI for fetching posts
    pub outbox_uri: Option<String>,
    pub followers_count: Option<u64>,
    pub following_count: Option<u64>,
    /// When this profile was last fetched
    pub fetched_at: DateTime<Utc>,
}

impl From<super::RemoteProfile> for CachedProfile {
    fn from(value: super::RemoteProfile) -> Self {
        Self {
            address: value.address,
            uri: value.uri,
            display_name: value.display_name,
            note: value.note,
            profile_fields_json: value.profile_fields_json,
            locked: value.locked,
            bot: value.bot,
            discoverable: value.discoverable,
            indexable: value.indexable,
            avatar_url: value.avatar_url,
            header_url: value.header_url,
            public_key_pem: value.public_key_pem,
            inbox_uri: value.inbox_uri,
            outbox_uri: value.outbox_uri,
            followers_count: value
                .followers_count
                .and_then(|count| u64::try_from(count).ok()),
            following_count: value
                .following_count
                .and_then(|count| u64::try_from(count).ok()),
            fetched_at: value.fetched_at,
        }
    }
}

impl From<&CachedProfile> for super::RemoteProfile {
    fn from(value: &CachedProfile) -> Self {
        Self {
            address: value.address.clone(),
            uri: value.uri.clone(),
            display_name: value.display_name.clone(),
            note: value.note.clone(),
            profile_fields_json: value.profile_fields_json.clone(),
            locked: value.locked,
            bot: value.bot,
            discoverable: value.discoverable,
            indexable: value.indexable,
            avatar_url: value.avatar_url.clone(),
            header_url: value.header_url.clone(),
            public_key_pem: value.public_key_pem.clone(),
            inbox_uri: value.inbox_uri.clone(),
            outbox_uri: value.outbox_uri.clone(),
            followers_count: value
                .followers_count
                .and_then(|count| i64::try_from(count).ok()),
            following_count: value
                .following_count
                .and_then(|count| i64::try_from(count).ok()),
            fetched_at: value.fetched_at,
            created_at: value.fetched_at,
            updated_at: value.fetched_at,
        }
    }
}

#[derive(Default)]
struct ProfileCacheState {
    profiles_by_address: HashMap<String, CachedProfile>,
    addresses_by_uri: HashMap<String, HashSet<String>>,
}

// =============================================================================
// Profile Cache
// =============================================================================

/// Profile cache for followees and followers
///
/// Populated on startup by fetching from follow addresses in DB.
/// Updated when Update activities are received.
pub struct ProfileCache {
    state: RwLock<ProfileCacheState>,
    ttl_ms: i64,
    last_prune_at_ms: AtomicI64,
}

impl ProfileCache {
    /// Create new profile cache
    pub async fn new(ttl_seconds: u64) -> Result<Self, AppError> {
        Ok(Self {
            state: RwLock::new(ProfileCacheState::default()),
            ttl_ms: ttl_seconds_to_millis(ttl_seconds),
            last_prune_at_ms: AtomicI64::new(0),
        })
    }

    fn remove_profile_locked(state: &mut ProfileCacheState, address: &str) -> bool {
        let Some(removed) = state.profiles_by_address.remove(address) else {
            return false;
        };

        if let Some(addresses) = state.addresses_by_uri.get_mut(&removed.uri) {
            addresses.remove(address);
            if addresses.is_empty() {
                state.addresses_by_uri.remove(&removed.uri);
            }
        }

        true
    }

    fn insert_profile_locked(state: &mut ProfileCacheState, profile: CachedProfile) {
        if let Some(previous) = state.profiles_by_address.get(&profile.address)
            && previous.uri != profile.uri
            && let Some(addresses) = state.addresses_by_uri.get_mut(&previous.uri)
        {
            addresses.remove(&profile.address);
            if addresses.is_empty() {
                state.addresses_by_uri.remove(&previous.uri);
            }
        }

        let uri = profile.uri.clone();
        let address = profile.address.clone();
        state.profiles_by_address.insert(address.clone(), profile);
        state
            .addresses_by_uri
            .entry(uri)
            .or_default()
            .insert(address);
    }

    fn prune_expired_locked(&self, state: &mut ProfileCacheState, now_ms: i64) {
        let cutoff = now_ms.saturating_sub(self.ttl_ms);
        let stale_addresses: Vec<String> = state
            .profiles_by_address
            .iter()
            .filter(|(_, profile)| profile.fetched_at.timestamp_millis() < cutoff)
            .map(|(address, _)| address.clone())
            .collect();
        for address in stale_addresses {
            Self::remove_profile_locked(state, &address);
        }
    }

    async fn prune_expired_if_needed(&self) {
        let now_ms = Utc::now().timestamp_millis();
        let last_prune = self.last_prune_at_ms.load(Ordering::Relaxed);
        if last_prune > 0 && now_ms.saturating_sub(last_prune) < PROFILE_PRUNE_INTERVAL_MS {
            return;
        }

        let mut state = self.state.write().await;
        self.prune_expired_locked(&mut state, now_ms);
        self.last_prune_at_ms.store(now_ms, Ordering::Relaxed);
    }

    fn update_size_metric(&self, count: usize) {
        use crate::metrics::CACHE_SIZE;
        CACHE_SIZE
            .with_label_values(&["profile"])
            .set(count.min(i64::MAX as usize) as i64);
    }

    async fn get_profiles_by_uri(&self, actor_uri: &str) -> Vec<CachedProfile> {
        let cutoff = Utc::now().timestamp_millis() - self.ttl_ms;
        let state = self.state.read().await;
        let Some(addresses) = state.addresses_by_uri.get(actor_uri) else {
            return Vec::new();
        };
        let mut profiles: Vec<CachedProfile> = addresses
            .iter()
            .filter_map(|address| state.profiles_by_address.get(address))
            .filter(|profile| profile.fetched_at.timestamp_millis() >= cutoff)
            .cloned()
            .collect();
        profiles.sort_by(|a, b| b.fetched_at.cmp(&a.fetched_at));
        profiles
    }

    /// Initialize cache from follow addresses
    ///
    /// Fetches profiles for all followees and followers in parallel.
    /// Called on application startup.
    ///
    /// # Arguments
    /// * `addresses` - List of addresses (user@domain) to fetch
    /// * `http_client` - HTTP client for fetching
    pub async fn initialize_from_addresses(
        &self,
        addresses: &[String],
        http_client: &reqwest::Client,
    ) {
        // Fetch profiles in parallel (max 10 concurrent)
        use futures::stream::{self, StreamExt};

        let mut unique_addresses: Vec<String> = addresses
            .iter()
            .map(|address| address.trim())
            .filter(|address| !address.is_empty())
            .map(ToString::to_string)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let cutoff = Utc::now().timestamp_millis() - self.ttl_ms;
        {
            let state = self.state.read().await;
            unique_addresses.retain(|address| {
                !state
                    .profiles_by_address
                    .get(address)
                    .is_some_and(|profile| profile.fetched_at.timestamp_millis() >= cutoff)
            });
        }

        stream::iter(unique_addresses)
            .map(|address| async move {
                let actor_uri = match discover_actor_uri(http_client, &address).await {
                    Ok(actor_uri) => actor_uri,
                    Err(error) => {
                        tracing::warn!(address = %address, %error, "Failed to discover actor URI for profile cache");
                        return;
                    }
                };

                let actor_document = match fetch_actor_document(http_client, &actor_uri).await {
                    Ok(actor_document) => actor_document,
                    Err(error) => {
                        tracing::warn!(address = %address, actor_uri = %actor_uri, %error, "Failed to fetch actor document for profile cache");
                        return;
                    }
                };

                let Some(profile) =
                    build_cached_profile_from_actor(&address, &actor_uri, &actor_document)
                else {
                    tracing::warn!(
                        address = %address,
                        actor_uri = %actor_uri,
                        "Failed to build cached profile from actor document"
                    );
                    return;
                };

                self.insert(profile).await;
            })
            .buffer_unordered(10)
            .collect::<Vec<_>>()
            .await;
    }

    /// Get profile by address
    pub async fn get(&self, address: &str) -> Option<Arc<CachedProfile>> {
        self.prune_expired_if_needed().await;

        let cutoff = Utc::now().timestamp_millis() - self.ttl_ms;
        let value = self
            .state
            .read()
            .await
            .profiles_by_address
            .get(address)
            .filter(|profile| profile.fetched_at.timestamp_millis() >= cutoff)
            .cloned()
            .map(Arc::new);

        use crate::metrics::{CACHE_HITS_TOTAL, CACHE_MISSES_TOTAL};
        if value.is_some() {
            CACHE_HITS_TOTAL.with_label_values(&["profile"]).inc();
        } else {
            CACHE_MISSES_TOTAL.with_label_values(&["profile"]).inc();
        }

        value
    }
    /// Get profile by actor URI
    pub async fn get_by_uri(&self, actor_uri: &str) -> Option<Arc<CachedProfile>> {
        self.prune_expired_if_needed().await;

        let cutoff = Utc::now().timestamp_millis() - self.ttl_ms;
        let state = self.state.read().await;
        let addresses = state.addresses_by_uri.get(actor_uri)?;
        let profile = addresses
            .iter()
            .filter_map(|address| state.profiles_by_address.get(address))
            .filter(|profile| profile.fetched_at.timestamp_millis() >= cutoff)
            .max_by(|a, b| a.fetched_at.cmp(&b.fetched_at))
            .cloned();

        let value = profile.map(Arc::new);
        use crate::metrics::{CACHE_HITS_TOTAL, CACHE_MISSES_TOTAL};
        if value.is_some() {
            CACHE_HITS_TOTAL.with_label_values(&["profile"]).inc();
        } else {
            CACHE_MISSES_TOTAL.with_label_values(&["profile"]).inc();
        }
        value
    }

    /// Insert or update profile
    pub async fn insert(&self, profile: CachedProfile) {
        let mut state = self.state.write().await;
        Self::insert_profile_locked(&mut state, profile);
        let size = state.profiles_by_address.len();
        drop(state);

        self.prune_expired_if_needed().await;
        self.update_size_metric(size);
    }

    /// Update profile from ActivityPub Update activity
    ///
    /// Called when receiving Update activity for a known actor.
    pub async fn update_from_activity(&self, actor_uri: &str, update_data: serde_json::Value) {
        let actor_object = update_data
            .get("object")
            .unwrap_or(&update_data)
            .as_object()
            .cloned();
        let Some(actor_object) = actor_object else {
            return;
        };

        if let Some(id) = actor_object.get("id").and_then(|value| value.as_str())
            && id != actor_uri
        {
            tracing::warn!(
                actor_uri = %actor_uri,
                object_id = %id,
                "Ignoring Update activity due to mismatched actor object id"
            );
            return;
        }

        let actor_value = serde_json::Value::Object(actor_object.clone());
        let existing_profiles = self.get_profiles_by_uri(actor_uri).await;

        if existing_profiles.is_empty() {
            if let Some(address) = actor_address_from_document(actor_uri, &actor_value)
                && let Some(profile) =
                    build_cached_profile_from_actor(&address, actor_uri, &actor_value)
            {
                self.insert(profile).await;
            }
            return;
        }

        for mut updated in existing_profiles {
            if actor_object.contains_key("name") {
                updated.display_name = actor_object
                    .get("name")
                    .and_then(|value| value.as_str())
                    .map(ToString::to_string);
            }

            if actor_object.contains_key("summary") || actor_object.contains_key("note") {
                updated.note = actor_object
                    .get("summary")
                    .or_else(|| actor_object.get("note"))
                    .and_then(|value| value.as_str())
                    .map(ToString::to_string);
            }

            if actor_object.contains_key("attachment") {
                updated.profile_fields_json = crate::profile_fields::serialize_profile_fields(
                    &crate::profile_fields::extract_profile_fields_from_actor(
                        &serde_json::Value::Object(actor_object.clone()),
                    ),
                )
                .ok()
                .flatten();
            }
            if actor_object.contains_key("manuallyApprovesFollowers") {
                updated.locked = actor_object
                    .get("manuallyApprovesFollowers")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
            }
            if actor_object.contains_key("bot") {
                updated.bot = actor_object
                    .get("bot")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
            }
            if actor_object.contains_key("discoverable") {
                updated.discoverable = actor_object
                    .get("discoverable")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true);
            }
            if actor_object.contains_key("indexable") {
                updated.indexable = actor_object
                    .get("indexable")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true);
            }

            if actor_object.contains_key("icon") {
                updated.avatar_url = actor_object.get("icon").and_then(extract_url);
            }
            if actor_object.contains_key("image") {
                updated.header_url = actor_object.get("image").and_then(extract_url);
            }

            if let Some(public_key_pem) = extract_public_key_pem(&actor_value) {
                updated.public_key_pem = public_key_pem;
            }

            if let Some(inbox_uri) = actor_object.get("inbox").and_then(|value| value.as_str())
                && url::Url::parse(inbox_uri).is_ok()
            {
                updated.inbox_uri = inbox_uri.to_string();
            }

            if actor_object.contains_key("outbox") {
                updated.outbox_uri = actor_object
                    .get("outbox")
                    .and_then(|value| value.as_str())
                    .map(ToString::to_string);
            }
            if actor_object.contains_key("followersCount") {
                updated.followers_count = actor_object
                    .get("followersCount")
                    .and_then(|value| value.as_u64());
            }
            if actor_object.contains_key("followingCount") {
                updated.following_count = actor_object
                    .get("followingCount")
                    .and_then(|value| value.as_u64());
            }

            updated.fetched_at = Utc::now();
            self.insert(updated).await;
        }
    }

    /// Get public key for signature verification
    ///
    /// # Arguments
    /// * `address` - Account address (user@domain)
    ///
    /// # Returns
    /// PEM-encoded public key or None if not cached
    pub async fn get_public_key(&self, address: &str) -> Option<String> {
        self.get(address).await.map(|p| p.public_key_pem.clone())
    }

    /// Get inbox URI for activity delivery
    pub async fn get_inbox(&self, address: &str) -> Option<String> {
        self.get(address).await.map(|p| p.inbox_uri.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn sample_status(id: &str, created_at: DateTime<Utc>) -> CachedStatus {
        CachedStatus {
            id: id.to_string(),
            uri: format!("https://example.com/status/{id}"),
            content: format!("content-{id}"),
            account_address: "alice@example.com".to_string(),
            created_at,
            visibility: "public".to_string(),
            attachments: Vec::new(),
            reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
        }
    }

    fn sample_profile(address: &str, fetched_at: DateTime<Utc>) -> CachedProfile {
        CachedProfile {
            address: address.to_string(),
            uri: format!("https://example.com/users/{address}"),
            display_name: Some("Alice".to_string()),
            note: Some("note".to_string()),
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            avatar_url: None,
            header_url: None,
            public_key_pem: "pem".to_string(),
            inbox_uri: "https://example.com/inbox".to_string(),
            outbox_uri: Some("https://example.com/outbox".to_string()),
            followers_count: Some(1),
            following_count: Some(2),
            fetched_at,
        }
    }

    #[test]
    fn profile_ttl_conversion_is_bounded() {
        assert_eq!(ttl_seconds_to_millis(1), 1000);
        let max_ttl_seconds = (i64::MAX as u64) / 1000;
        let bounded = ttl_seconds_to_millis(u64::MAX);
        assert!(bounded > 0);
        assert_eq!(bounded, ttl_seconds_to_millis(max_ttl_seconds));
        if max_ttl_seconds < u64::MAX {
            assert_eq!(bounded, ttl_seconds_to_millis(max_ttl_seconds + 1));
        }
    }

    #[tokio::test]
    async fn timeline_insert_and_get() {
        let cache = TimelineCache::new(16).await.expect("cache init");
        let status = sample_status("s1", Utc::now());

        cache.insert(status.clone()).await;
        let fetched = cache.get("s1").await.expect("status should exist");

        assert_eq!(fetched.id, status.id);
        assert_eq!(fetched.uri, status.uri);
        assert_eq!(fetched.content, status.content);
    }

    #[tokio::test]
    async fn timeline_evicts_oldest_when_over_capacity() {
        let cache = TimelineCache::new(2).await.expect("cache init");
        let now = Utc::now();

        cache
            .insert(sample_status("s1", now - Duration::seconds(3)))
            .await;
        cache
            .insert(sample_status("s2", now - Duration::seconds(2)))
            .await;
        cache
            .insert(sample_status("s3", now - Duration::seconds(1)))
            .await;

        assert!(
            cache.get("s1").await.is_none(),
            "oldest entry should be evicted"
        );
        assert!(cache.get("s2").await.is_some());
        assert!(cache.get("s3").await.is_some());
    }

    #[tokio::test]
    async fn timeline_ttl_removes_expired_entries() {
        let cache = TimelineCache::new(16).await.expect("cache init");
        cache.insert(sample_status("expired", Utc::now())).await;
        let expired_inserted_at =
            Utc::now().timestamp_millis() - Duration::days(8).num_milliseconds();
        {
            let mut state = cache.state.write().await;
            let entry = state
                .entries_by_id
                .get_mut("expired")
                .expect("cache entry should exist");
            entry.inserted_at_ms = expired_inserted_at;
        }

        assert!(
            cache.get("expired").await.is_none(),
            "entries older than 7 days should expire"
        );
    }

    #[tokio::test]
    async fn timeline_supports_concurrent_inserts() {
        let cache = Arc::new(TimelineCache::new(128).await.expect("cache init"));
        let now = Utc::now();

        let mut tasks = Vec::new();
        for idx in 0..32 {
            let cache = Arc::clone(&cache);
            tasks.push(tokio::spawn(async move {
                let id = format!("status-{idx}");
                cache
                    .insert(sample_status(&id, now + Duration::milliseconds(idx as i64)))
                    .await;
            }));
        }

        for task in tasks {
            task.await.expect("join");
        }

        assert!(cache.get("status-0").await.is_some());
        assert!(cache.get("status-31").await.is_some());
    }

    #[tokio::test]
    async fn profile_ttl_prunes_expired_entries() {
        let cache = ProfileCache::new(1).await.expect("cache init");
        let profile = sample_profile("alice@example.com", Utc::now() - Duration::seconds(120));

        cache.insert(profile).await;
        assert!(cache.get("alice@example.com").await.is_none());
    }

    #[tokio::test]
    async fn profile_get_by_uri_returns_latest_entry() {
        let cache = ProfileCache::new(60).await.expect("cache init");
        let mut profile = sample_profile("alice@example.com", Utc::now());
        profile.uri = "https://example.com/users/alice".to_string();

        cache.insert(profile.clone()).await;

        let fetched = cache
            .get_by_uri("https://example.com/users/alice")
            .await
            .expect("profile should exist");
        assert_eq!(fetched.address, "alice@example.com");
        assert_eq!(fetched.uri, profile.uri);
    }

    #[tokio::test]
    async fn profile_initialize_from_addresses_fetches_webfinger_and_actor() {
        use axum::{Json, Router, extract::Query, routing::get};
        use serde::Deserialize;
        use tokio::net::TcpListener;

        #[derive(Deserialize)]
        struct WebFingerQuery {
            resource: String,
        }

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("server address");
        let actor_uri = format!("http://{addr}/users/alice");
        let inbox_uri = format!("http://{addr}/users/alice/inbox");

        let actor_uri_for_webfinger = actor_uri.clone();
        let app = Router::new()
            .route(
                "/.well-known/webfinger",
                get(move |Query(query): Query<WebFingerQuery>| {
                    let actor_uri = actor_uri_for_webfinger.clone();
                    async move {
                        assert_eq!(query.resource, format!("acct:alice@{addr}"));
                        Json(serde_json::json!({
                            "subject": format!("acct:alice@{addr}"),
                            "links": [{
                                "rel": "self",
                                "type": "application/activity+json",
                                "href": actor_uri,
                            }]
                        }))
                    }
                }),
            )
            .route(
                "/users/alice",
                get(move || {
                    let inbox_uri = inbox_uri.clone();
                    async move {
                        Json(serde_json::json!({
                            "id": format!("http://{addr}/users/alice"),
                            "name": "Alice",
                            "summary": "<p>Hello</p>",
                            "inbox": inbox_uri,
                            "outbox": format!("http://{addr}/users/alice/outbox"),
                            "publicKey": {
                                "id": format!("http://{addr}/users/alice#main-key"),
                                "publicKeyPem": "test-public-key"
                            },
                            "followersCount": 12,
                            "followingCount": 34
                        }))
                    }
                }),
            );

        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test server");
        });

        let cache = ProfileCache::new(60).await.expect("cache init");
        let http_client = reqwest::Client::new();
        cache
            .initialize_from_addresses(&[format!("alice@{addr}")], &http_client)
            .await;

        let profile = cache.get(&format!("alice@{addr}")).await.expect("profile");
        assert_eq!(profile.uri, actor_uri);
        assert_eq!(
            profile.inbox_uri,
            format!("http://{addr}/users/alice/inbox")
        );
        assert_eq!(profile.display_name.as_deref(), Some("Alice"));
        assert_eq!(profile.public_key_pem, "test-public-key");
        assert_eq!(profile.followers_count, Some(12));
        assert_eq!(profile.following_count, Some(34));
    }

    #[tokio::test]
    async fn profile_update_from_activity_applies_profile_fields() {
        let cache = ProfileCache::new(60).await.expect("cache init");
        let mut profile = sample_profile("alice@example.com", Utc::now());
        profile.uri = "https://remote.example/users/alice".to_string();
        profile.inbox_uri = "https://remote.example/inbox-old".to_string();
        profile.public_key_pem = "old-public-key".to_string();
        cache.insert(profile).await;

        cache
            .update_from_activity(
                "https://remote.example/users/alice",
                serde_json::json!({
                    "type": "Update",
                    "object": {
                        "id": "https://remote.example/users/alice",
                        "name": "Alice Updated",
                        "summary": "updated summary",
                        "icon": { "url": "https://cdn.example/avatar.png" },
                        "image": { "url": "https://cdn.example/header.png" },
                        "publicKey": {
                            "publicKeyPem": "new-public-key"
                        },
                        "inbox": "https://remote.example/inbox-new",
                        "outbox": "https://remote.example/outbox-new",
                        "followersCount": 99,
                        "followingCount": 77
                    }
                }),
            )
            .await;

        let updated = cache
            .get("alice@example.com")
            .await
            .expect("updated profile");
        assert_eq!(updated.display_name.as_deref(), Some("Alice Updated"));
        assert_eq!(updated.note.as_deref(), Some("updated summary"));
        assert_eq!(
            updated.avatar_url.as_deref(),
            Some("https://cdn.example/avatar.png")
        );
        assert_eq!(
            updated.header_url.as_deref(),
            Some("https://cdn.example/header.png")
        );
        assert_eq!(updated.public_key_pem, "new-public-key");
        assert_eq!(updated.inbox_uri, "https://remote.example/inbox-new");
        assert_eq!(
            updated.outbox_uri.as_deref(),
            Some("https://remote.example/outbox-new")
        );
        assert_eq!(updated.followers_count, Some(99));
        assert_eq!(updated.following_count, Some(77));
    }

    #[tokio::test]
    async fn profile_update_from_activity_ignores_mismatched_actor_id() {
        let cache = ProfileCache::new(60).await.expect("cache init");
        let mut profile = sample_profile("alice@example.com", Utc::now());
        profile.uri = "https://remote.example/users/alice".to_string();
        profile.display_name = Some("Alice Before".to_string());
        profile.inbox_uri = "https://remote.example/inbox-old".to_string();
        cache.insert(profile).await;

        cache
            .update_from_activity(
                "https://remote.example/users/alice",
                serde_json::json!({
                    "type": "Update",
                    "object": {
                        "id": "https://attacker.example/users/mallory",
                        "name": "Alice After",
                        "inbox": "https://attacker.example/inbox"
                    }
                }),
            )
            .await;

        let unchanged = cache
            .get("alice@example.com")
            .await
            .expect("profile should exist");
        assert_eq!(unchanged.display_name.as_deref(), Some("Alice Before"));
        assert_eq!(unchanged.inbox_uri, "https://remote.example/inbox-old");
        assert_eq!(unchanged.uri, "https://remote.example/users/alice");
    }

    #[tokio::test]
    async fn profile_update_from_activity_updates_all_rows_for_same_actor_uri() {
        let cache = ProfileCache::new(60).await.expect("cache init");
        let actor_uri = "https://remote.example/users/alice";

        let mut primary = sample_profile("alice@remote.example", Utc::now());
        primary.uri = actor_uri.to_string();
        primary.display_name = Some("Before".to_string());
        primary.inbox_uri = "https://remote.example/inbox-old".to_string();
        cache.insert(primary).await;

        let mut alias = sample_profile(actor_uri, Utc::now());
        alias.uri = actor_uri.to_string();
        alias.display_name = Some("Before".to_string());
        alias.inbox_uri = "https://remote.example/inbox-old".to_string();
        cache.insert(alias).await;

        cache
            .update_from_activity(
                actor_uri,
                serde_json::json!({
                    "type": "Update",
                    "object": {
                        "id": actor_uri,
                        "name": "After",
                        "inbox": "https://remote.example/inbox-new"
                    }
                }),
            )
            .await;

        let updated_primary = cache
            .get("alice@remote.example")
            .await
            .expect("primary row");
        let updated_alias = cache.get(actor_uri).await.expect("alias row");
        assert_eq!(updated_primary.display_name.as_deref(), Some("After"));
        assert_eq!(
            updated_primary.inbox_uri,
            "https://remote.example/inbox-new"
        );
        assert_eq!(updated_alias.display_name.as_deref(), Some("After"));
        assert_eq!(updated_alias.inbox_uri, "https://remote.example/inbox-new");
    }
}
fn is_before_cursor(status: &CachedStatus, cursor: &TimelineCursorKey) -> bool {
    status.created_at < cursor.created_at
        || (status.created_at == cursor.created_at && status.id < cursor.id)
}

fn is_after_cursor(status: &CachedStatus, cursor: &TimelineCursorKey) -> bool {
    status.created_at > cursor.created_at
        || (status.created_at == cursor.created_at && status.id > cursor.id)
}
