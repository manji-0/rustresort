//! Timeline service
//!
//! Handles timeline retrieval from cache and database.

use std::sync::Arc;

use crate::data::{Database, ProfileCache, Status, TimelineCache};
use crate::error::AppError;

/// Timeline service
pub struct TimelineService {
    db: Arc<Database>,
    timeline_cache: Arc<TimelineCache>,
    profile_cache: Arc<ProfileCache>,
}

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
    /// Returns statuses from followed accounts (from cache)
    /// plus own statuses (from database).
    ///
    /// # Arguments
    /// * `limit` - Maximum results (default 20, max 40)
    /// * `max_id` - Return statuses older than this ID
    /// * `min_id` - Return statuses newer than this ID
    ///
    /// # Returns
    /// Merged and sorted list of statuses
    pub async fn home_timeline(
        &self,
        _limit: usize,
        _max_id: Option<&str>,
        _min_id: Option<&str>,
    ) -> Result<Vec<TimelineItem>, AppError> {
        // TODO:
        // 1. Get follow addresses from DB
        // 2. Get cached statuses from followees
        // 3. Get own local statuses from DB
        // 4. Merge and sort by created_at desc
        // 5. Apply pagination
        todo!()
    }

    /// Get public timeline
    ///
    /// Returns all public statuses from cache.
    ///
    /// # Arguments
    /// * `local_only` - If true, only return local statuses
    /// * `limit` - Maximum results
    /// * `max_id` - Pagination cursor
    pub async fn public_timeline(
        &self,
        _local_only: bool,
        _limit: usize,
        _max_id: Option<&str>,
    ) -> Result<Vec<TimelineItem>, AppError> {
        // TODO:
        // 1. Get from cache (filtered by visibility=public)
        // 2. If local_only, also include local from DB
        // 3. Sort and limit
        todo!()
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
        // TODO:
        // 1. If local account, get from DB
        // 2. If remote, get from cache
        // 3. Apply filters
        todo!()
    }

    /// Get favourites timeline
    ///
    /// Returns statuses the user has favourited.
    pub async fn favourites_timeline(
        &self,
        _limit: usize,
        _max_id: Option<&str>,
    ) -> Result<Vec<TimelineItem>, AppError> {
        // TODO:
        // 1. Get favourited status IDs from DB
        // 2. Get statuses from DB (persisted)
        todo!()
    }

    /// Get bookmarks timeline
    pub async fn bookmarks_timeline(
        &self,
        _limit: usize,
        _max_id: Option<&str>,
    ) -> Result<Vec<TimelineItem>, AppError> {
        // TODO: Similar to favourites
        todo!()
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
