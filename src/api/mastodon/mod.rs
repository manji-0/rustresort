//! Mastodon API compatible endpoints
//!
//! Implements subset of Mastodon API for client app compatibility.
//! See: https://docs.joinmastodon.org/api/

use axum::{
    Router,
    routing::{delete, get, post, put},
};

use crate::AppState;

pub mod accounts;
pub mod admin;
pub mod apps;
pub mod bookmarks;
pub mod conversations;
pub mod filters;
pub mod instance;
pub mod lists;
pub mod media;
pub mod notifications;
pub mod polls;
pub mod scheduled_statuses;
pub mod search;
pub mod statuses;
pub mod streaming;
pub mod timelines;

/// Create Mastodon API router
///
/// Routes are split into public and authenticated endpoints.
pub fn mastodon_api_router() -> Router<AppState> {
    // Public endpoints (no authentication required)
    let public_routes = Router::new()
        // Instance information is public
        .route("/v1/instance", get(instance::instance))
        .route("/v1/instance/peers", get(instance::instance_peers))
        .route("/v1/instance/activity", get(instance::instance_activity))
        .route("/v1/instance/rules", get(instance::instance_rules))
        .route("/v2/instance", get(instance::instance_v2))
        // App registration is public
        .route("/v1/apps", post(apps::create_app))
        // Account creation is public
        .route("/v1/accounts", post(accounts::create_account))
        // Public timelines
        .route("/v1/timelines/public", get(timelines::public_timeline))
        // Public account and status views
        .route("/v1/accounts/:id", get(accounts::get_account))
        .route("/v1/statuses/:id", get(statuses::get_status))
        .route(
            "/v1/statuses/:id/context",
            get(statuses::get_status_context),
        )
        .route(
            "/v1/statuses/:id/reblogged_by",
            get(statuses::get_reblogged_by),
        )
        .route(
            "/v1/statuses/:id/favourited_by",
            get(statuses::get_favourited_by),
        );

    // Authenticated endpoints (require valid token)
    let authenticated_routes = Router::new()
        // Apps - verify credentials requires auth
        .route(
            "/v1/apps/verify_credentials",
            get(apps::verify_app_credentials),
        )
        // Accounts - authenticated operations
        .route(
            "/v1/accounts/verify_credentials",
            get(accounts::verify_credentials),
        )
        .route(
            "/v1/accounts/update_credentials",
            axum::routing::patch(accounts::update_credentials),
        )
        .route("/v1/accounts/:id/statuses", get(accounts::account_statuses))
        .route(
            "/v1/accounts/:id/followers",
            get(accounts::get_account_followers),
        )
        .route(
            "/v1/accounts/:id/following",
            get(accounts::get_account_following),
        )
        .route("/v1/accounts/:id/follow", post(accounts::follow_account))
        .route(
            "/v1/accounts/:id/unfollow",
            post(accounts::unfollow_account),
        )
        .route(
            "/v1/accounts/relationships",
            get(accounts::get_relationships),
        )
        .route("/v1/accounts/search", get(accounts::search_accounts))
        .route("/v1/accounts/:id/lists", get(accounts::get_account_lists))
        .route(
            "/v1/accounts/:id/identity_proofs",
            get(accounts::get_account_identity_proofs),
        )
        .route("/v1/accounts/:id/block", post(accounts::block_account))
        .route("/v1/accounts/:id/unblock", post(accounts::unblock_account))
        .route("/v1/accounts/:id/mute", post(accounts::mute_account))
        .route("/v1/accounts/:id/unmute", post(accounts::unmute_account))
        // Blocks & Mutes
        .route("/v1/blocks", get(accounts::get_blocks))
        .route("/v1/mutes", get(accounts::get_mutes))
        // Follow Requests
        .route("/v1/follow_requests", get(accounts::get_follow_requests))
        .route("/v1/follow_requests/:id", get(accounts::get_follow_request))
        .route(
            "/v1/follow_requests/:id/authorize",
            post(accounts::authorize_follow_request),
        )
        .route(
            "/v1/follow_requests/:id/reject",
            post(accounts::reject_follow_request),
        )
        // Statuses - write operations require auth
        .route("/v1/statuses", post(statuses::create_status))
        .route("/v1/statuses/:id", delete(statuses::delete_status))
        .route("/v1/statuses/:id/source", get(statuses::get_status_source))
        .route(
            "/v1/statuses/:id/favourite",
            post(statuses::favourite_status),
        )
        .route(
            "/v1/statuses/:id/unfavourite",
            post(statuses::unfavourite_status),
        )
        .route("/v1/statuses/:id/reblog", post(statuses::reblog_status))
        .route("/v1/statuses/:id/unreblog", post(statuses::unreblog_status))
        .route("/v1/statuses/:id/bookmark", post(statuses::bookmark_status))
        .route(
            "/v1/statuses/:id/unbookmark",
            post(statuses::unbookmark_status),
        )
        .route("/v1/statuses/:id", put(statuses::update_status))
        .route(
            "/v1/statuses/:id/history",
            get(statuses::get_status_history),
        )
        .route("/v1/statuses/:id/pin", post(statuses::pin_status))
        .route("/v1/statuses/:id/unpin", post(statuses::unpin_status))
        .route("/v1/statuses/:id/mute", post(statuses::mute_status))
        .route("/v1/statuses/:id/unmute", post(statuses::unmute_status))
        // Timelines - require auth (except public which is in public_routes)
        .route("/v1/timelines/home", get(timelines::home_timeline))
        .route("/v1/timelines/tag/:hashtag", get(timelines::tag_timeline))
        .route("/v1/timelines/list/:list_id", get(timelines::list_timeline))
        // Notifications
        .route("/v1/notifications", get(notifications::get_notifications))
        .route(
            "/v1/notifications/:id",
            get(notifications::get_notification),
        )
        .route(
            "/v1/notifications/:id/dismiss",
            post(notifications::dismiss_notification),
        )
        .route(
            "/v1/notifications/clear",
            post(notifications::clear_notifications),
        )
        .route(
            "/v1/notifications/unread_count",
            get(notifications::get_unread_count),
        )
        // Media
        .route("/v1/media", post(media::upload_media))
        .route("/v2/media", post(media::upload_media_v2))
        .route("/v1/media/:id", get(media::get_media))
        .route("/v1/media/:id", put(media::update_media))
        // Lists
        .route("/v1/lists", get(lists::get_lists))
        .route("/v1/lists/:id", get(lists::get_list))
        .route("/v1/lists", post(lists::create_list))
        .route("/v1/lists/:id", put(lists::update_list))
        .route("/v1/lists/:id", delete(lists::delete_list))
        .route("/v1/lists/:id/accounts", get(lists::get_list_accounts))
        .route("/v1/lists/:id/accounts", post(lists::add_list_accounts))
        .route(
            "/v1/lists/:id/accounts",
            delete(lists::delete_list_accounts),
        )
        // Filters
        .route("/v1/filters", get(filters::get_filters))
        .route("/v1/filters/:id", get(filters::get_filter))
        .route("/v1/filters", post(filters::create_filter))
        .route("/v1/filters/:id", put(filters::update_filter))
        .route("/v1/filters/:id", delete(filters::delete_filter))
        .route("/v2/filters", get(filters::get_filters_v2))
        // Bookmarks / Favourites
        .route("/v1/bookmarks", get(bookmarks::get_bookmarks))
        .route("/v1/favourites", get(bookmarks::get_favourites))
        // Search
        .route("/v1/search", get(search::search_v1))
        .route("/v2/search", get(search::search_v2))
        // Polls
        .route("/v1/polls/:id", get(polls::get_poll))
        .route("/v1/polls/:id/votes", post(polls::vote_in_poll))
        // Scheduled Statuses
        .route(
            "/v1/scheduled_statuses",
            get(scheduled_statuses::get_scheduled_statuses),
        )
        .route(
            "/v1/scheduled_statuses/:id",
            get(scheduled_statuses::get_scheduled_status),
        )
        .route(
            "/v1/scheduled_statuses/:id",
            put(scheduled_statuses::update_scheduled_status),
        )
        .route(
            "/v1/scheduled_statuses/:id",
            delete(scheduled_statuses::delete_scheduled_status),
        )
        // Conversations
        .route("/v1/conversations", get(conversations::get_conversations))
        .route(
            "/v1/conversations/:id",
            delete(conversations::delete_conversation),
        )
        .route(
            "/v1/conversations/:id/read",
            post(conversations::mark_conversation_read),
        )
        // Streaming API
        .route("/v1/streaming/health", get(streaming::streaming_health))
        .route("/v1/streaming/user", get(streaming::stream_user))
        .route("/v1/streaming/public", get(streaming::stream_public))
        .route(
            "/v1/streaming/public/local",
            get(streaming::stream_public_local),
        )
        .route("/v1/streaming/hashtag", get(streaming::stream_hashtag))
        .route("/v1/streaming/list", get(streaming::stream_list))
        .route("/v1/streaming/direct", get(streaming::stream_direct))
        // Admin API
        .route("/v1/admin/accounts", get(admin::list_accounts))
        .route("/v1/admin/accounts/:id", get(admin::get_account))
        .route("/v1/admin/accounts/:id/action", post(admin::account_action))
        .route("/v1/admin/reports", get(admin::list_reports))
        .route("/v1/admin/domain_blocks", get(admin::list_domain_blocks_v1))
        .route(
            "/v1/admin/domain_blocks",
            post(admin::create_domain_block_v1),
        )
        .route(
            "/v1/admin/domain_blocks/:id",
            delete(admin::delete_domain_block_v1),
        );

    // Merge public and authenticated routes
    // Note: Authentication is enforced by using CurrentUser extractor in handlers
    public_routes.merge(authenticated_routes)
}
