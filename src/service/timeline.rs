//! Timeline service
//!
//! Handles timeline retrieval from database and cache-backed metadata.

use std::{collections::HashSet, future::Future, sync::Arc};

use crate::data::{
    CachedStatus, Follow, ListTimelineQuery, PersistedReason, ProfileCache, Status,
    StatusVisibility, TimelineCache, TimelineRepository,
};
use crate::error::AppError;

/// Timeline service
pub struct TimelineService {
    db: Arc<dyn TimelineRepository>,
    timeline_cache: Arc<TimelineCache>,
    _profile_cache: Arc<ProfileCache>,
}

const TIMELINE_MUTE_OVERFETCH_MULTIPLIER: usize = 3;
const TIMELINE_MUTE_OVERFETCH_MAX_LIMIT: usize = 200;

impl TimelineService {
    /// Create new timeline service
    pub fn new<R>(
        db: Arc<R>,
        timeline_cache: Arc<TimelineCache>,
        profile_cache: Arc<ProfileCache>,
    ) -> Self
    where
        R: TimelineRepository + 'static,
    {
        Self {
            db,
            timeline_cache,
            _profile_cache: profile_cache,
        }
    }

    /// Get home timeline
    ///
    /// Returns local statuses plus cached followee statuses.
    ///
    /// # Arguments
    /// * `limit` - Maximum results (default 20, max 40)
    /// * `max_id` - Return statuses older than this ID
    /// * `min_id` - Return statuses newer than this ID
    ///
    /// # Returns
    /// Sorted list of statuses
    pub async fn home_timeline(
        &self,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<TimelineItem>, AppError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let fetch_limit = Self::overfetch_limit(limit);
        let local_statuses = self
            .db
            .get_local_statuses_in_window(fetch_limit, max_id, min_id)
            .await?;
        let follows = self.db.get_all_follows().await?;
        let followee_identities = Self::followee_cache_identities(&follows);
        let cached_statuses = self
            .timeline_cache
            .get_home_timeline(&followee_identities, fetch_limit, max_id)
            .await;
        let statuses = self
            .merge_statuses(local_statuses, cached_statuses, limit, min_id)
            .await?;
        self.build_timeline_items_with_interactions(statuses).await
    }

    /// Get public timeline
    ///
    /// Returns local public statuses and, unless `local_only`, cached remote public statuses.
    ///
    /// # Arguments
    /// * `local_only` - If true, only return local statuses
    /// * `limit` - Maximum results
    /// * `max_id` - Pagination cursor
    pub async fn public_timeline(
        &self,
        local_only: bool,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<TimelineItem>, AppError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let fetch_limit = Self::overfetch_limit(limit);
        let local_statuses = self
            .db
            .get_local_public_statuses(fetch_limit, max_id, min_id)
            .await?;
        let cached_statuses = if local_only {
            Vec::new()
        } else {
            self.timeline_cache
                .get_public_timeline(fetch_limit, max_id)
                .await
        };
        let statuses = self
            .merge_statuses(local_statuses, cached_statuses, limit, None)
            .await?;
        self.build_timeline_items_with_interactions(statuses).await
    }

    /// Get hashtag timeline.
    pub async fn tag_timeline(
        &self,
        hashtag: &str,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<TimelineItem>, AppError> {
        let hashtag = hashtag.to_string();
        let min_id = min_id.map(str::to_string);
        let statuses = self
            .collect_visible_statuses(limit, max_id.map(str::to_string), |fetch_limit, cursor| {
                let hashtag = hashtag.clone();
                let min_id = min_id.clone();
                async move {
                    self.db
                        .get_statuses_by_hashtag_in_window(
                            &hashtag,
                            fetch_limit,
                            cursor.as_deref(),
                            min_id.as_deref(),
                        )
                        .await
                }
            })
            .await?;
        self.build_timeline_items_with_interactions(statuses).await
    }

    /// Get list timeline.
    pub async fn list_timeline(
        &self,
        query: &ListTimelineQuery,
    ) -> Result<Vec<TimelineItem>, AppError> {
        let list_id = query.list_id.clone();
        let local_account_address = query.local_account_address.clone();
        let local_account_id = query.local_account_id.clone();
        let default_port = query.default_port;
        let min_id = query.min_id.clone();
        let statuses = self
            .collect_visible_statuses(query.limit, query.max_id.clone(), |fetch_limit, cursor| {
                let list_id = list_id.clone();
                let local_account_address = local_account_address.clone();
                let local_account_id = local_account_id.clone();
                let min_id = min_id.clone();
                async move {
                    self.db
                        .get_list_timeline_statuses_in_window(&ListTimelineQuery {
                            list_id,
                            local_account_address,
                            local_account_id,
                            default_port,
                            limit: fetch_limit,
                            max_id: cursor,
                            min_id,
                        })
                        .await
                }
            })
            .await?;
        self.build_timeline_items_with_interactions(statuses).await
    }

    /// Get account timeline
    ///
    /// Returns statuses from a specific account.
    ///
    /// # Arguments
    /// * `account_address` - Account address (user@domain)
    /// * `limit` - Maximum results
    /// * `max_id` - Pagination cursor
    /// * `only_media` - If true, only statuses with media
    /// * `exclude_replies` - If true, exclude replies
    pub async fn account_timeline(
        &self,
        account_address: Option<&str>,
        default_port: Option<u16>,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
        only_media: bool,
        exclude_replies: bool,
        exclude_reblogs: bool,
        only_pinned: bool,
    ) -> Result<Vec<TimelineItem>, AppError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let muted_thread_uris = self.db.get_muted_thread_uris().await?;
        let has_filters = only_media || exclude_replies || exclude_reblogs || only_pinned;
        let fetch_limit = if has_filters {
            limit
                .saturating_mul(5)
                .min(TIMELINE_MUTE_OVERFETCH_MAX_LIMIT)
        } else {
            Self::overfetch_limit(limit)
        };

        let mut filtered = Vec::with_capacity(limit);
        let mut page_max_id = max_id.map(str::to_string);

        loop {
            let statuses = match account_address {
                Some(account_address) => {
                    self.db
                        .get_statuses_by_account_address_in_window(
                            account_address,
                            default_port,
                            fetch_limit,
                            page_max_id.as_deref(),
                            min_id,
                        )
                        .await?
                }
                None => {
                    self.db
                        .get_local_statuses_in_window(fetch_limit, page_max_id.as_deref(), min_id)
                        .await?
                }
            };
            if statuses.is_empty() {
                break;
            }

            let batch_len = statuses.len();
            let last_status_id = statuses.last().map(|status| status.id.clone());
            let statuses = self
                .filter_muted_threads_with_uris(statuses, &muted_thread_uris)
                .await?;

            for status in statuses {
                if !matches!(
                    status.visibility,
                    StatusVisibility::Public | StatusVisibility::Unlisted
                ) {
                    continue;
                }
                if exclude_reblogs && status.boost_of_uri.is_some() {
                    continue;
                }
                if exclude_replies && status.in_reply_to_uri.is_some() {
                    continue;
                }

                let is_pinned = self.db.is_status_pinned(&status.id).await?;
                if only_pinned && !is_pinned {
                    continue;
                }
                if only_media && self.db.get_media_by_status(&status.id).await?.is_empty() {
                    continue;
                }

                filtered.push(status);
                if filtered.len() >= limit {
                    break;
                }
            }

            if filtered.len() >= limit {
                break;
            }
            if last_status_id.is_none() || batch_len < fetch_limit {
                break;
            }

            let Some(next_max_id) = last_status_id else {
                break;
            };
            if page_max_id.as_deref() == Some(next_max_id.as_str()) {
                break;
            }
            page_max_id = Some(next_max_id);
        }

        self.build_timeline_items_with_interactions(filtered).await
    }

    /// Get favourites timeline
    ///
    /// Returns statuses the user has favourited.
    pub async fn favourites_timeline(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<TimelineItem>, AppError> {
        let statuses = self
            .collect_visible_statuses(
                limit,
                max_id.map(str::to_string),
                |fetch_limit, cursor| async move {
                    self.db
                        .get_favourited_statuses(fetch_limit, cursor.as_deref())
                        .await
                },
            )
            .await?;
        let status_ids: Vec<String> = statuses.iter().map(|status| status.id.clone()).collect();
        let bookmarked_ids = self.db.get_bookmarked_status_ids_batch(&status_ids).await?;

        let mut items = Vec::with_capacity(statuses.len());
        for status in statuses {
            items.push(TimelineItem {
                account: Self::timeline_account_from_status(&status),
                bookmarked: bookmarked_ids.contains(&status.id),
                status,
                favourited: true,
                reblogged: false,
            });
        }

        Ok(items)
    }

    /// Get bookmarks timeline
    pub async fn bookmarks_timeline(
        &self,
        limit: usize,
        max_id: Option<&str>,
    ) -> Result<Vec<TimelineItem>, AppError> {
        let statuses = self
            .collect_visible_statuses(
                limit,
                max_id.map(str::to_string),
                |fetch_limit, cursor| async move {
                    self.db
                        .get_bookmarked_statuses(fetch_limit, cursor.as_deref())
                        .await
                },
            )
            .await?;
        let status_ids: Vec<String> = statuses.iter().map(|status| status.id.clone()).collect();
        let favourited_ids = self.db.get_favourited_status_ids_batch(&status_ids).await?;

        let mut items = Vec::with_capacity(statuses.len());
        for status in statuses {
            items.push(TimelineItem {
                account: Self::timeline_account_from_status(&status),
                favourited: favourited_ids.contains(&status.id),
                status,
                reblogged: false,
                bookmarked: true,
            });
        }

        Ok(items)
    }

    fn timeline_account_from_status(status: &Status) -> TimelineAccount {
        let default_address = if status.is_local {
            "local@local".to_string()
        } else {
            "remote@unknown".to_string()
        };
        let address = if status.account_address.is_empty() {
            default_address
        } else {
            status.account_address.clone()
        };
        let username = address
            .split('@')
            .next()
            .filter(|part| !part.is_empty())
            .unwrap_or("unknown")
            .to_string();

        TimelineAccount {
            address,
            username,
            display_name: None,
            avatar_url: None,
            is_local: status.is_local,
        }
    }

    async fn build_timeline_items_with_interactions(
        &self,
        statuses: Vec<Status>,
    ) -> Result<Vec<TimelineItem>, AppError> {
        let status_ids: Vec<String> = statuses.iter().map(|status| status.id.clone()).collect();
        let favourited_ids = self.db.get_favourited_status_ids_batch(&status_ids).await?;
        let bookmarked_ids = self.db.get_bookmarked_status_ids_batch(&status_ids).await?;

        Ok(statuses
            .into_iter()
            .map(|status| TimelineItem {
                account: Self::timeline_account_from_status(&status),
                favourited: favourited_ids.contains(&status.id),
                bookmarked: bookmarked_ids.contains(&status.id),
                status,
                reblogged: false,
            })
            .collect())
    }

    fn cached_status_to_status(cached: &CachedStatus) -> Status {
        Status {
            id: cached.id.clone(),
            uri: cached.uri.clone(),
            content: cached.content.clone(),
            content_warning: None,
            visibility: StatusVisibility::parse(&cached.visibility)
                .unwrap_or(StatusVisibility::Public),
            language: None,
            account_address: cached.account_address.clone(),
            is_local: false,
            in_reply_to_uri: cached.reply_to_uri.clone(),
            boost_of_uri: cached.boost_of_uri.clone(),
            quote_of_uri: cached.quote_of_uri.clone(),
            persisted_reason: PersistedReason::CacheOnly,
            created_at: cached.created_at,
            fetched_at: Some(cached.created_at),
        }
    }

    fn overfetch_limit(limit: usize) -> usize {
        limit
            .saturating_mul(TIMELINE_MUTE_OVERFETCH_MULTIPLIER)
            .max(limit)
            .min(TIMELINE_MUTE_OVERFETCH_MAX_LIMIT)
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

    fn format_authority_host(host: &str) -> String {
        if host.contains(':') {
            format!("[{}]", host)
        } else {
            host.to_string()
        }
    }

    fn cache_identity_from_actor_uri(actor_uri: &str) -> Option<String> {
        let parsed = url::Url::parse(actor_uri).ok()?;
        let host = parsed.host_str()?;
        let username = Self::extract_username_from_actor_path(parsed.path())?;
        let authority_host = Self::format_authority_host(&host.to_ascii_lowercase());
        let authority = match parsed.port() {
            Some(port) => format!("{}:{}", authority_host, port),
            None => authority_host,
        };
        Some(format!("{}@{}", username.to_ascii_lowercase(), authority))
    }

    fn followee_cache_identities(follows: &[Follow]) -> HashSet<String> {
        let mut identities = HashSet::new();

        for follow in follows {
            identities.insert(follow.target_address.clone());
            if let Some(actor_uri) = &follow.actor_uri {
                identities.insert(actor_uri.clone());
                if let Some(actor_address) = Self::cache_identity_from_actor_uri(actor_uri) {
                    identities.insert(actor_address);
                }
            }
        }

        identities
    }

    async fn merge_statuses(
        &self,
        db_statuses: Vec<Status>,
        cached_statuses: Vec<Arc<CachedStatus>>,
        limit: usize,
        min_id: Option<&str>,
    ) -> Result<Vec<Status>, AppError> {
        let mut merged = Vec::with_capacity(db_statuses.len() + cached_statuses.len());
        let mut seen_uris = HashSet::new();
        let mut seen_ids = HashSet::new();

        for status in db_statuses {
            seen_uris.insert(status.uri.clone());
            seen_ids.insert(status.id.clone());
            merged.push(status);
        }

        for cached in cached_statuses {
            let status = Self::cached_status_to_status(&cached);
            if min_id.is_some_and(|cursor| status.id.as_str() <= cursor) {
                continue;
            }
            if seen_uris.contains(&status.uri) || seen_ids.contains(&status.id) {
                continue;
            }
            seen_uris.insert(status.uri.clone());
            seen_ids.insert(status.id.clone());
            merged.push(status);
        }

        merged.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.id.cmp(&left.id))
        });

        let visible = self
            .filter_muted_threads_with_uris(merged, &self.db.get_muted_thread_uris().await?)
            .await?;
        Ok(visible.into_iter().take(limit).collect())
    }

    async fn collect_visible_statuses<F, Fut>(
        &self,
        limit: usize,
        initial_max_id: Option<String>,
        mut fetch_page: F,
    ) -> Result<Vec<Status>, AppError>
    where
        F: FnMut(usize, Option<String>) -> Fut,
        Fut: Future<Output = Result<Vec<Status>, AppError>>,
    {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let muted_thread_uris = self.db.get_muted_thread_uris().await?;
        if muted_thread_uris.is_empty() {
            return fetch_page(limit, initial_max_id).await;
        }

        let fetch_limit = limit
            .saturating_mul(TIMELINE_MUTE_OVERFETCH_MULTIPLIER)
            .max(limit)
            .min(TIMELINE_MUTE_OVERFETCH_MAX_LIMIT);
        let mut cursor = initial_max_id;
        let mut visible = Vec::with_capacity(limit);

        loop {
            let statuses = fetch_page(fetch_limit, cursor.clone()).await?;
            if statuses.is_empty() {
                break;
            }

            let fetched_count = statuses.len();
            cursor = statuses.last().map(|status| status.id.clone());

            let filtered = self
                .filter_muted_threads_with_uris(statuses, &muted_thread_uris)
                .await?;
            for status in filtered {
                visible.push(status);
                if visible.len() >= limit {
                    return Ok(visible);
                }
            }

            if fetched_count < fetch_limit || cursor.is_none() {
                break;
            }
        }

        Ok(visible)
    }

    async fn filter_muted_threads_with_uris(
        &self,
        statuses: Vec<Status>,
        muted_thread_uris: &HashSet<String>,
    ) -> Result<Vec<Status>, AppError> {
        if statuses.is_empty() {
            return Ok(statuses);
        }
        if muted_thread_uris.is_empty() {
            return Ok(statuses);
        }

        let mut visible = Vec::with_capacity(statuses.len());
        for status in statuses {
            let thread_uri = self.db.resolve_thread_root_uri(&status).await?;
            if !muted_thread_uris.contains(&thread_uri) {
                visible.push(status);
            }
        }

        Ok(visible)
    }
}

/// Timeline item for API response
///
/// Contains status data enriched with account info.
#[derive(Debug, Clone)]
pub struct TimelineItem {
    /// Status data
    pub status: Status,
    /// Account display info (from cache or constructed)
    pub account: TimelineAccount,
    /// Whether user has favourited this
    pub favourited: bool,
    /// Whether user has boosted this
    pub reblogged: bool,
    /// Whether user has bookmarked this
    pub bookmarked: bool,
}

/// Account info for timeline display
#[derive(Debug, Clone)]
pub struct TimelineAccount {
    pub address: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// true if this is the local account
    pub is_local: bool,
}
