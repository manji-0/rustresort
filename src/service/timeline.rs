//! Timeline service
//!
//! Handles timeline retrieval from database and cache-backed metadata.

use std::{collections::HashSet, future::Future, sync::Arc};

use crate::data::{Database, ProfileCache, Status, TimelineCache};
use crate::error::AppError;

/// Timeline service
pub struct TimelineService {
    db: Arc<Database>,
    timeline_cache: Arc<TimelineCache>,
    profile_cache: Arc<ProfileCache>,
}

const TIMELINE_MUTE_OVERFETCH_MULTIPLIER: usize = 3;
const TIMELINE_MUTE_OVERFETCH_MAX_LIMIT: usize = 200;

impl TimelineService {
    /// Create new timeline service
    pub fn new(
        db: Arc<Database>,
        timeline_cache: Arc<TimelineCache>,
        profile_cache: Arc<ProfileCache>,
    ) -> Self {
        Self {
            db,
            timeline_cache,
            profile_cache,
        }
    }

    /// Get home timeline
    ///
    /// Returns local statuses for the single-user instance.
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
        let min_id = min_id.map(str::to_string);
        let statuses = self
            .collect_visible_statuses(limit, max_id.map(str::to_string), |fetch_limit, cursor| {
                let min_id = min_id.clone();
                async move {
                    self.db
                        .get_local_statuses_in_window(
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

    /// Get public timeline
    ///
    /// Returns local public statuses for the single-user instance.
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
    ) -> Result<Vec<TimelineItem>, AppError> {
        // Single-user instance currently stores local statuses only,
        // so local_only doesn't change query behavior yet.
        let _ = local_only;
        let statuses = self
            .collect_visible_statuses(
                limit,
                max_id.map(str::to_string),
                |fetch_limit, cursor| async move {
                    self.db
                        .get_local_public_statuses(fetch_limit, cursor.as_deref())
                        .await
                },
            )
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
        list_id: &str,
        local_account_address: &str,
        local_account_id: &str,
        limit: usize,
        max_id: Option<&str>,
        min_id: Option<&str>,
    ) -> Result<Vec<TimelineItem>, AppError> {
        let list_id = list_id.to_string();
        let local_account_address = local_account_address.to_string();
        let local_account_id = local_account_id.to_string();
        let min_id = min_id.map(str::to_string);
        let statuses = self
            .collect_visible_statuses(limit, max_id.map(str::to_string), |fetch_limit, cursor| {
                let list_id = list_id.clone();
                let local_account_address = local_account_address.clone();
                let local_account_id = local_account_id.clone();
                let min_id = min_id.clone();
                async move {
                    self.db
                        .get_list_timeline_statuses_in_window(
                            &list_id,
                            &local_account_address,
                            &local_account_id,
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
        _account_address: &str,
        _limit: usize,
        _max_id: Option<&str>,
        _only_media: bool,
        _exclude_replies: bool,
    ) -> Result<Vec<TimelineItem>, AppError> {
        Err(AppError::NotImplemented(
            "account timeline is not implemented yet".to_string(),
        ))
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
