//! In-memory caches backed by Turso in-memory databases.
//!
//! These caches are volatile and cleared on restart.

use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use turso::{Builder, Connection, Value};

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

fn map_turso_error(context: &str, error: turso::Error) -> AppError {
    AppError::Internal(anyhow::anyhow!("{context}: {error}"))
}

fn deserialize_attachments(json: &str) -> Vec<CachedAttachment> {
    serde_json::from_str::<Vec<CachedAttachment>>(json).unwrap_or_default()
}

fn to_datetime(created_at_ms: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(created_at_ms).unwrap_or_else(Utc::now)
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

// =============================================================================
// Timeline Cache
// =============================================================================

/// Timeline cache (volatile, max 2000 items)
///
/// Stores recent statuses from followees.
pub struct TimelineCache {
    /// Hold database for lifetime management.
    _db: turso::Database,
    conn: Connection,
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
        let max_items = max_items.max(1);

        let db = Builder::new_local(":memory:")
            .build()
            .await
            .map_err(|e| map_turso_error("failed to create timeline cache database", e))?;
        let conn = db
            .connect()
            .map_err(|e| map_turso_error("failed to connect timeline cache database", e))?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS timeline_statuses (
                id TEXT PRIMARY KEY,
                uri TEXT NOT NULL UNIQUE,
                content TEXT NOT NULL,
                account_address TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                visibility TEXT NOT NULL,
                attachments_json TEXT NOT NULL,
                reply_to_uri TEXT,
                boost_of_uri TEXT,
                inserted_at_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_timeline_statuses_created_at
                ON timeline_statuses(created_at_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_timeline_statuses_account
                ON timeline_statuses(account_address, created_at_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_timeline_statuses_visibility
                ON timeline_statuses(visibility, created_at_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_timeline_statuses_uri
                ON timeline_statuses(uri);
            "#,
        )
        .await
        .map_err(|e| map_turso_error("failed to initialize timeline cache schema", e))?;

        Ok(Self {
            _db: db,
            conn,
            ttl_ms: TIMELINE_TTL_MS,
            max_items,
        })
    }

    async fn prune_expired(&self) -> Result<(), turso::Error> {
        let cutoff = Utc::now().timestamp_millis() - self.ttl_ms;
        self.conn
            .execute(
                "DELETE FROM timeline_statuses WHERE inserted_at_ms < ?1",
                [cutoff],
            )
            .await?;
        Ok(())
    }

    async fn update_size_metric(&self) -> Result<(), turso::Error> {
        let mut rows = self
            .conn
            .query("SELECT COUNT(*) FROM timeline_statuses", ())
            .await?;
        let count = if let Some(row) = rows.next().await? {
            row.get::<i64>(0)?
        } else {
            0
        };

        use crate::metrics::CACHE_SIZE;
        CACHE_SIZE.with_label_values(&["timeline"]).set(count);
        Ok(())
    }

    fn parse_status_row(row: &turso::Row) -> Result<CachedStatus, turso::Error> {
        let attachments_json: String = row.get(6)?;
        Ok(CachedStatus {
            id: row.get(0)?,
            uri: row.get(1)?,
            content: row.get(2)?,
            account_address: row.get(3)?,
            created_at: to_datetime(row.get(4)?),
            visibility: row.get(5)?,
            attachments: deserialize_attachments(&attachments_json),
            reply_to_uri: row.get(7)?,
            boost_of_uri: row.get(8)?,
        })
    }

    /// Insert status into cache
    ///
    /// Automatically evicts oldest items when capacity is reached.
    pub async fn insert(&self, status: CachedStatus) {
        let attachments_json = match serde_json::to_string(&status.attachments) {
            Ok(json) => json,
            Err(error) => {
                tracing::warn!(%error, "Failed to serialize timeline cache attachments");
                return;
            }
        };

        let created_at_ms = status.created_at.timestamp_millis();
        let inserted_at_ms = Utc::now().timestamp_millis();

        let upsert_result = self
            .conn
            .execute(
                r#"
                INSERT INTO timeline_statuses (
                    id, uri, content, account_address, created_at_ms, visibility,
                    attachments_json, reply_to_uri, boost_of_uri, inserted_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                ON CONFLICT(id) DO UPDATE SET
                    uri = excluded.uri,
                    content = excluded.content,
                    account_address = excluded.account_address,
                    created_at_ms = excluded.created_at_ms,
                    visibility = excluded.visibility,
                    attachments_json = excluded.attachments_json,
                    reply_to_uri = excluded.reply_to_uri,
                    boost_of_uri = excluded.boost_of_uri,
                    inserted_at_ms = excluded.inserted_at_ms
                "#,
                (
                    status.id,
                    status.uri,
                    status.content,
                    status.account_address,
                    created_at_ms,
                    status.visibility,
                    attachments_json,
                    status.reply_to_uri,
                    status.boost_of_uri,
                    inserted_at_ms,
                ),
            )
            .await;

        if let Err(error) = upsert_result {
            tracing::warn!(%error, "Failed to upsert timeline cache entry");
            return;
        }

        if let Err(error) = self.prune_expired().await {
            tracing::warn!(%error, "Failed to prune expired timeline cache entries");
        }

        if let Err(error) = self
            .conn
            .execute(
                r#"
                DELETE FROM timeline_statuses
                WHERE id IN (
                    SELECT id
                    FROM timeline_statuses
                    ORDER BY created_at_ms DESC, id DESC
                    LIMIT -1 OFFSET ?1
                )
                "#,
                [self.max_items as i64],
            )
            .await
        {
            tracing::warn!(%error, "Failed to enforce timeline cache size limit");
        }

        if let Err(error) = self.update_size_metric().await {
            tracing::warn!(%error, "Failed to update timeline cache metrics");
        }
    }

    /// Get status by ID
    pub async fn get(&self, id: &str) -> Option<Arc<CachedStatus>> {
        if let Err(error) = self.prune_expired().await {
            tracing::warn!(%error, "Failed to prune expired timeline cache entries");
        }

        let result = self
            .conn
            .query(
                r#"
                SELECT id, uri, content, account_address, created_at_ms, visibility,
                       attachments_json, reply_to_uri, boost_of_uri
                FROM timeline_statuses
                WHERE id = ?1
                LIMIT 1
                "#,
                [id],
            )
            .await;

        let mut rows = match result {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "Failed to fetch timeline cache entry by id");
                return None;
            }
        };

        let value = match rows.next().await {
            Ok(Some(row)) => match Self::parse_status_row(&row) {
                Ok(status) => Some(Arc::new(status)),
                Err(error) => {
                    tracing::warn!(%error, "Failed to decode timeline cache entry");
                    None
                }
            },
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(%error, "Failed to iterate timeline cache rows");
                None
            }
        };

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
        if let Err(error) = self.prune_expired().await {
            tracing::warn!(%error, "Failed to prune expired timeline cache entries");
        }

        let result = self
            .conn
            .query(
                r#"
                SELECT id, uri, content, account_address, created_at_ms, visibility,
                       attachments_json, reply_to_uri, boost_of_uri
                FROM timeline_statuses
                WHERE uri = ?1
                LIMIT 1
                "#,
                [uri],
            )
            .await;

        let mut rows = match result {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "Failed to fetch timeline cache entry by uri");
                return None;
            }
        };

        match rows.next().await {
            Ok(Some(row)) => match Self::parse_status_row(&row) {
                Ok(status) => Some(Arc::new(status)),
                Err(error) => {
                    tracing::warn!(%error, "Failed to decode timeline cache entry");
                    None
                }
            },
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(%error, "Failed to iterate timeline cache rows");
                None
            }
        }
    }

    /// Remove status from cache
    pub async fn remove(&self, id: &str) {
        if let Err(error) = self
            .conn
            .execute("DELETE FROM timeline_statuses WHERE id = ?1", [id])
            .await
        {
            tracing::warn!(%error, "Failed to remove timeline cache entry by id");
            return;
        }

        if let Err(error) = self.update_size_metric().await {
            tracing::warn!(%error, "Failed to update timeline cache metrics");
        }
    }

    /// Remove status from cache by ActivityPub URI.
    pub async fn remove_by_uri(&self, uri: &str) {
        if let Err(error) = self
            .conn
            .execute("DELETE FROM timeline_statuses WHERE uri = ?1", [uri])
            .await
        {
            tracing::warn!(%error, "Failed to remove timeline cache entry by uri");
            return;
        }

        if let Err(error) = self.update_size_metric().await {
            tracing::warn!(%error, "Failed to update timeline cache metrics");
        }
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
        max_id: Option<&str>,
    ) -> Vec<Arc<CachedStatus>> {
        if followee_addresses.is_empty() {
            return Vec::new();
        }

        if let Err(error) = self.prune_expired().await {
            tracing::warn!(%error, "Failed to prune expired timeline cache entries");
        }

        let mut sql = String::from(
            r#"
            SELECT id, uri, content, account_address, created_at_ms, visibility,
                   attachments_json, reply_to_uri, boost_of_uri
            FROM timeline_statuses
            WHERE account_address IN (
            "#,
        );
        let placeholders = vec!["?"; followee_addresses.len()].join(", ");
        sql.push_str(&placeholders);
        sql.push(')');

        let mut params: Vec<Value> = followee_addresses
            .iter()
            .cloned()
            .map(Value::from)
            .collect();

        if let Some(max_id) = max_id {
            sql.push_str(" AND id < ?");
            params.push(Value::from(max_id.to_string()));
        }

        sql.push_str(" ORDER BY created_at_ms DESC LIMIT ?");
        params.push(Value::from(limit as i64));

        let mut rows = match self.conn.query(&sql, params).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "Failed to fetch timeline cache home timeline");
                return Vec::new();
            }
        };

        let mut statuses = Vec::new();
        loop {
            match rows.next().await {
                Ok(Some(row)) => match Self::parse_status_row(&row) {
                    Ok(status) => statuses.push(Arc::new(status)),
                    Err(error) => tracing::warn!(%error, "Failed to decode timeline cache entry"),
                },
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(%error, "Failed to iterate home timeline rows");
                    break;
                }
            }
        }

        statuses
    }

    /// Get public timeline
    ///
    /// Returns all public statuses in cache.
    pub async fn get_public_timeline(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Vec<Arc<CachedStatus>> {
        if let Err(error) = self.prune_expired().await {
            tracing::warn!(%error, "Failed to prune expired timeline cache entries");
        }

        let (sql, params): (&str, Vec<Value>) = if let Some(max_id) = max_id {
            (
                r#"
                SELECT id, uri, content, account_address, created_at_ms, visibility,
                       attachments_json, reply_to_uri, boost_of_uri
                FROM timeline_statuses
                WHERE visibility = 'public' AND id < ?1
                ORDER BY created_at_ms DESC
                LIMIT ?2
                "#,
                vec![Value::from(max_id.to_string()), Value::from(limit as i64)],
            )
        } else {
            (
                r#"
                SELECT id, uri, content, account_address, created_at_ms, visibility,
                       attachments_json, reply_to_uri, boost_of_uri
                FROM timeline_statuses
                WHERE visibility = 'public'
                ORDER BY created_at_ms DESC
                LIMIT ?1
                "#,
                vec![Value::from(limit as i64)],
            )
        };

        let mut rows = match self.conn.query(sql, params).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "Failed to fetch timeline cache public timeline");
                return Vec::new();
            }
        };

        let mut statuses = Vec::new();
        loop {
            match rows.next().await {
                Ok(Some(row)) => match Self::parse_status_row(&row) {
                    Ok(status) => statuses.push(Arc::new(status)),
                    Err(error) => tracing::warn!(%error, "Failed to decode timeline cache entry"),
                },
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(%error, "Failed to iterate public timeline rows");
                    break;
                }
            }
        }

        statuses
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

// =============================================================================
// Profile Cache
// =============================================================================

/// Profile cache for followees and followers
///
/// Populated on startup by fetching from follow addresses in DB.
/// Updated when Update activities are received.
pub struct ProfileCache {
    /// Hold database for lifetime management.
    _db: turso::Database,
    conn: Connection,
    ttl_ms: i64,
    last_prune_at_ms: AtomicI64,
}

impl ProfileCache {
    /// Create new profile cache
    pub async fn new(ttl_seconds: u64) -> Result<Self, AppError> {
        let db = Builder::new_local(":memory:")
            .build()
            .await
            .map_err(|e| map_turso_error("failed to create profile cache database", e))?;
        let conn = db
            .connect()
            .map_err(|e| map_turso_error("failed to connect profile cache database", e))?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS profiles (
                address TEXT PRIMARY KEY,
                uri TEXT NOT NULL,
                display_name TEXT,
                note TEXT,
                avatar_url TEXT,
                header_url TEXT,
                public_key_pem TEXT NOT NULL,
                inbox_uri TEXT NOT NULL,
                outbox_uri TEXT,
                followers_count INTEGER,
                following_count INTEGER,
                fetched_at_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_profiles_uri ON profiles(uri);
            CREATE INDEX IF NOT EXISTS idx_profiles_fetched_at ON profiles(fetched_at_ms);
            "#,
        )
        .await
        .map_err(|e| map_turso_error("failed to initialize profile cache schema", e))?;

        Ok(Self {
            _db: db,
            conn,
            ttl_ms: ttl_seconds_to_millis(ttl_seconds),
            last_prune_at_ms: AtomicI64::new(0),
        })
    }

    async fn prune_expired(&self) -> Result<(), turso::Error> {
        let cutoff = Utc::now().timestamp_millis() - self.ttl_ms;
        self.conn
            .execute("DELETE FROM profiles WHERE fetched_at_ms < ?1", [cutoff])
            .await?;
        Ok(())
    }

    async fn prune_expired_if_needed(&self) -> Result<(), turso::Error> {
        let now = Utc::now().timestamp_millis();
        let last_prune = self.last_prune_at_ms.load(Ordering::Relaxed);
        if last_prune > 0 && now.saturating_sub(last_prune) < PROFILE_PRUNE_INTERVAL_MS {
            return Ok(());
        }

        self.prune_expired().await?;
        self.last_prune_at_ms.store(now, Ordering::Relaxed);
        Ok(())
    }

    async fn update_size_metric(&self) -> Result<(), turso::Error> {
        let mut rows = self.conn.query("SELECT COUNT(*) FROM profiles", ()).await?;
        let count = if let Some(row) = rows.next().await? {
            row.get::<i64>(0)?
        } else {
            0
        };
        use crate::metrics::CACHE_SIZE;
        CACHE_SIZE.with_label_values(&["profile"]).set(count);
        Ok(())
    }

    fn parse_profile_row(row: &turso::Row) -> Result<CachedProfile, turso::Error> {
        let followers_count: Option<i64> = row.get(9)?;
        let following_count: Option<i64> = row.get(10)?;

        Ok(CachedProfile {
            address: row.get(0)?,
            uri: row.get(1)?,
            display_name: row.get(2)?,
            note: row.get(3)?,
            avatar_url: row.get(4)?,
            header_url: row.get(5)?,
            public_key_pem: row.get(6)?,
            inbox_uri: row.get(7)?,
            outbox_uri: row.get(8)?,
            followers_count: followers_count.map(|v| v as u64),
            following_count: following_count.map(|v| v as u64),
            fetched_at: to_datetime(row.get(11)?),
        })
    }

    async fn get_profiles_by_uri(
        &self,
        actor_uri: &str,
    ) -> Result<Vec<CachedProfile>, turso::Error> {
        let cutoff = Utc::now().timestamp_millis() - self.ttl_ms;
        let mut rows = self
            .conn
            .query(
                r#"
                SELECT
                    address, uri, display_name, note, avatar_url, header_url, public_key_pem,
                    inbox_uri, outbox_uri, followers_count, following_count, fetched_at_ms
                FROM profiles
                WHERE uri = ?1
                  AND fetched_at_ms >= ?2
                ORDER BY fetched_at_ms DESC
                "#,
                (actor_uri, cutoff),
            )
            .await?;

        let mut profiles = Vec::new();
        while let Some(row) = rows.next().await? {
            profiles.push(Self::parse_profile_row(&row)?);
        }

        Ok(profiles)
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

        let unique_addresses: Vec<String> = addresses
            .iter()
            .map(|address| address.trim())
            .filter(|address| !address.is_empty())
            .map(ToString::to_string)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

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
        if let Err(error) = self.prune_expired_if_needed().await {
            tracing::warn!(%error, "Failed to prune profile cache");
        }

        let cutoff = Utc::now().timestamp_millis() - self.ttl_ms;
        let result = self
            .conn
            .query(
                r#"
                SELECT
                    address, uri, display_name, note, avatar_url, header_url, public_key_pem,
                    inbox_uri, outbox_uri, followers_count, following_count, fetched_at_ms
                FROM profiles
                WHERE address = ?1
                  AND fetched_at_ms >= ?2
                LIMIT 1
                "#,
                (address, cutoff),
            )
            .await;

        let mut rows = match result {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "Failed to fetch profile cache entry");
                return None;
            }
        };

        let value = match rows.next().await {
            Ok(Some(row)) => match Self::parse_profile_row(&row) {
                Ok(profile) => Some(Arc::new(profile)),
                Err(error) => {
                    tracing::warn!(%error, "Failed to decode profile cache entry");
                    None
                }
            },
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(%error, "Failed to iterate profile cache rows");
                None
            }
        };

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
        if let Err(error) = self.prune_expired_if_needed().await {
            tracing::warn!(%error, "Failed to prune profile cache");
        }

        let cutoff = Utc::now().timestamp_millis() - self.ttl_ms;
        let result = self
            .conn
            .query(
                r#"
                SELECT
                    address, uri, display_name, note, avatar_url, header_url, public_key_pem,
                    inbox_uri, outbox_uri, followers_count, following_count, fetched_at_ms
                FROM profiles
                WHERE uri = ?1
                  AND fetched_at_ms >= ?2
                ORDER BY fetched_at_ms DESC
                LIMIT 1
                "#,
                (actor_uri, cutoff),
            )
            .await;

        let mut rows = match result {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "Failed to fetch profile cache entry by actor URI");
                return None;
            }
        };

        let value = match rows.next().await {
            Ok(Some(row)) => match Self::parse_profile_row(&row) {
                Ok(profile) => Some(Arc::new(profile)),
                Err(error) => {
                    tracing::warn!(%error, "Failed to decode profile cache entry");
                    None
                }
            },
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(%error, "Failed to iterate profile cache rows");
                None
            }
        };

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
        let upsert_result = self
            .conn
            .execute(
                r#"
                INSERT INTO profiles (
                    address, uri, display_name, note, avatar_url, header_url, public_key_pem,
                    inbox_uri, outbox_uri, followers_count, following_count, fetched_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                ON CONFLICT(address) DO UPDATE SET
                    uri = excluded.uri,
                    display_name = excluded.display_name,
                    note = excluded.note,
                    avatar_url = excluded.avatar_url,
                    header_url = excluded.header_url,
                    public_key_pem = excluded.public_key_pem,
                    inbox_uri = excluded.inbox_uri,
                    outbox_uri = excluded.outbox_uri,
                    followers_count = excluded.followers_count,
                    following_count = excluded.following_count,
                    fetched_at_ms = excluded.fetched_at_ms
                "#,
                (
                    profile.address,
                    profile.uri,
                    profile.display_name,
                    profile.note,
                    profile.avatar_url,
                    profile.header_url,
                    profile.public_key_pem,
                    profile.inbox_uri,
                    profile.outbox_uri,
                    profile.followers_count.map(|v| v as i64),
                    profile.following_count.map(|v| v as i64),
                    profile.fetched_at.timestamp_millis(),
                ),
            )
            .await;

        if let Err(error) = upsert_result {
            tracing::warn!(%error, "Failed to upsert profile cache entry");
            return;
        }

        if let Err(error) = self.prune_expired_if_needed().await {
            tracing::warn!(%error, "Failed to prune profile cache");
        }

        if let Err(error) = self.update_size_metric().await {
            tracing::warn!(%error, "Failed to update profile cache metrics");
        }
    }

    /// Update profile from ActivityPub Update activity
    ///
    /// Called when receiving Update activity for a known actor.
    pub async fn update_from_activity(&self, actor_uri: &str, update_data: serde_json::Value) {
        let existing_profiles = match self.get_profiles_by_uri(actor_uri).await {
            Ok(existing_profiles) => existing_profiles,
            Err(error) => {
                tracing::warn!(%error, actor_uri = %actor_uri, "Failed to query profile cache by actor URI");
                return;
            }
        };

        if existing_profiles.is_empty() {
            return;
        }

        let actor_object = update_data
            .get("object")
            .unwrap_or(&update_data)
            .as_object()
            .cloned();
        let Some(actor_object) = actor_object else {
            return;
        };

        if let Some(id) = actor_object.get("id").and_then(|value| value.as_str()) {
            if id != actor_uri {
                tracing::warn!(
                    actor_uri = %actor_uri,
                    object_id = %id,
                    "Ignoring Update activity due to mismatched actor object id"
                );
                return;
            }
        }

        let actor_value = serde_json::Value::Object(actor_object.clone());

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

            if actor_object.contains_key("icon") {
                updated.avatar_url = actor_object.get("icon").and_then(extract_url);
            }
            if actor_object.contains_key("image") {
                updated.header_url = actor_object.get("image").and_then(extract_url);
            }

            if let Some(public_key_pem) = extract_public_key_pem(&actor_value) {
                updated.public_key_pem = public_key_pem;
            }

            if let Some(inbox_uri) = actor_object.get("inbox").and_then(|value| value.as_str()) {
                if url::Url::parse(inbox_uri).is_ok() {
                    updated.inbox_uri = inbox_uri.to_string();
                }
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
        }
    }

    fn sample_profile(address: &str, fetched_at: DateTime<Utc>) -> CachedProfile {
        CachedProfile {
            address: address.to_string(),
            uri: format!("https://example.com/users/{address}"),
            display_name: Some("Alice".to_string()),
            note: Some("note".to_string()),
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
        let bounded = ttl_seconds_to_millis(u64::MAX);
        assert!(bounded > 0);
        assert!(bounded <= i64::MAX);
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
        cache
            .conn
            .execute(
                "UPDATE timeline_statuses SET inserted_at_ms = ?1 WHERE id = ?2",
                (expired_inserted_at, "expired"),
            )
            .await
            .expect("set old inserted_at");

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
