//! Mastodon API compatible endpoints
//!
//! Implements subset of Mastodon API for client app compatibility.
//! See: https://docs.joinmastodon.org/api/

use axum::{
    Router,
    extract::FromRef,
    middleware,
    routing::{MethodRouter, delete, get, post, put},
};

use crate::{
    AccountApiState, AdminApiState, AppsApiState, AuthState, ConversationsApiState,
    FiltersApiState, InstanceApiState, ListsApiState, MediaApiState, PollsApiState, PushApiState,
    ScheduledStatusesApiState, SearchApiState, StatusApiState, StreamingApiState, TimelineApiState,
};

pub mod accounts;
pub mod admin;
pub mod apps;
pub mod bookmarks;
pub mod conversations;
pub(crate) mod federation_delivery;
pub mod filters;
pub mod instance;
pub mod lists;
pub mod markers;
pub mod media;
pub mod notifications;
pub mod polls;
pub mod push;
pub mod scheduled_statuses;
pub mod search;
pub mod statuses;
pub mod streaming;
pub mod timelines;

const SESSION_ONLY: &[&str] = &[];
const READ_ACCOUNTS: &[&str] = &["read:accounts"];
const WRITE_ACCOUNTS: &[&str] = &["write:accounts"];
const FOLLOW: &[&str] = &["follow"];
const READ_STATUSES: &[&str] = &["read:statuses"];
const WRITE_STATUSES: &[&str] = &["write:statuses"];
const WRITE_FAVOURITES: &[&str] = &["write:favourites"];
const READ_NOTIFICATIONS: &[&str] = &["read:notifications"];
const READ_USER_STREAM: &[&str] = &["read:statuses", "read:notifications"];
const WRITE_NOTIFICATIONS: &[&str] = &["write:notifications"];
const WRITE_MEDIA: &[&str] = &["write:media"];
const READ_LISTS: &[&str] = &["read:lists"];
const WRITE_LISTS: &[&str] = &["write:lists"];
const READ_FILTERS: &[&str] = &["read:filters"];
const WRITE_FILTERS: &[&str] = &["write:filters"];
const READ_SEARCH: &[&str] = &["read:search"];

/// Create Mastodon API router
///
/// Routes are split into public and authenticated endpoints.
pub fn mastodon_api_router<S>(auth_state: AuthState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AccountApiState: FromRef<S>,
    AdminApiState: FromRef<S>,
    AppsApiState: FromRef<S>,
    ConversationsApiState: FromRef<S>,
    FiltersApiState: FromRef<S>,
    InstanceApiState: FromRef<S>,
    ListsApiState: FromRef<S>,
    MediaApiState: FromRef<S>,
    PollsApiState: FromRef<S>,
    PushApiState: FromRef<S>,
    ScheduledStatusesApiState: FromRef<S>,
    SearchApiState: FromRef<S>,
    StatusApiState: FromRef<S>,
    StreamingApiState: FromRef<S>,
    TimelineApiState: FromRef<S>,
{
    let scoped = |router: MethodRouter<S>, scopes: &'static [&'static str]| {
        let auth_state = auth_state.clone();
        router.route_layer(middleware::from_fn(
            move |jar: axum_extra::extract::CookieJar,
                  request: axum::http::Request<axum::body::Body>,
                  next: axum::middleware::Next| {
                let auth_state = auth_state.clone();
                async move {
                    crate::auth::require_auth_scopes_with_policy(
                        auth_state,
                        crate::auth::ScopePolicy::Any(scopes),
                        jar,
                        request,
                        next,
                    )
                    .await
                }
            },
        ))
    };
    let scoped_all = |router: MethodRouter<S>, scopes: &'static [&'static str]| {
        let auth_state = auth_state.clone();
        router.route_layer(middleware::from_fn(
            move |jar: axum_extra::extract::CookieJar,
                  request: axum::http::Request<axum::body::Body>,
                  next: axum::middleware::Next| {
                let auth_state = auth_state.clone();
                async move {
                    crate::auth::require_auth_scopes_with_policy(
                        auth_state,
                        crate::auth::ScopePolicy::All(scopes),
                        jar,
                        request,
                        next,
                    )
                    .await
                }
            },
        ))
    };
    let app_auth = |router: MethodRouter<S>| {
        let auth_state = auth_state.clone();
        router.route_layer(middleware::from_fn(
            move |request: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| {
                let auth_state = auth_state.clone();
                async move {
                    crate::auth::require_app_auth(axum::extract::State(auth_state), request, next)
                        .await
                }
            },
        ))
    };

    // Public endpoints (no authentication required)
    let public_routes = Router::new()
        // Instance information is public
        .route("/v1/instance", get(instance::instance))
        .route("/v1/instance/peers", get(instance::instance_peers))
        .route("/v1/instance/activity", get(instance::instance_activity))
        .route("/v1/instance/rules", get(instance::instance_rules))
        .route("/v2/instance", get(instance::instance_v2))
        .route("/v1/apps", post(apps::create_app))
        .route("/v1/custom_emojis", get(instance::custom_emojis))
        // Public timelines
        .route("/v1/timelines/public", get(timelines::public_timeline))
        // Public account and status views
        .route("/v1/accounts/:id", get(accounts::get_account))
        .route("/v1/accounts/:id/statuses", get(accounts::account_statuses))
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
        // Accounts - authenticated operations
        .route(
            "/v1/apps/verify_credentials",
            app_auth(get(apps::verify_app_credentials)),
        )
        .route(
            "/v1/accounts/verify_credentials",
            scoped(get(accounts::verify_credentials), READ_ACCOUNTS),
        )
        .route(
            "/v1/preferences",
            scoped(get(accounts::preferences), READ_ACCOUNTS),
        )
        .route(
            "/v1/accounts/update_credentials",
            scoped(
                axum::routing::patch(accounts::update_credentials),
                WRITE_ACCOUNTS,
            ),
        )
        .route(
            "/v1/accounts/lookup",
            scoped(get(accounts::lookup_account), READ_ACCOUNTS),
        )
        .route(
            "/v1/accounts/:id/followers",
            scoped(get(accounts::get_account_followers), READ_ACCOUNTS),
        )
        .route(
            "/v1/accounts/:id/following",
            scoped(get(accounts::get_account_following), READ_ACCOUNTS),
        )
        .route(
            "/v1/accounts/:id/follow",
            scoped(post(accounts::follow_account), FOLLOW),
        )
        .route(
            "/v1/accounts/:id/unfollow",
            scoped(post(accounts::unfollow_account), FOLLOW),
        )
        .route(
            "/v1/accounts/relationships",
            scoped(get(accounts::get_relationships), READ_ACCOUNTS),
        )
        .route(
            "/v1/accounts/search",
            scoped(get(accounts::search_accounts), READ_ACCOUNTS),
        )
        .route(
            "/v1/accounts/:id/lists",
            scoped(get(accounts::get_account_lists), READ_ACCOUNTS),
        )
        .route(
            "/v1/accounts/:id/identity_proofs",
            scoped(get(accounts::get_account_identity_proofs), READ_ACCOUNTS),
        )
        .route(
            "/v1/accounts/:id/block",
            scoped(post(accounts::block_account), WRITE_ACCOUNTS),
        )
        .route(
            "/v1/accounts/:id/unblock",
            scoped(post(accounts::unblock_account), WRITE_ACCOUNTS),
        )
        .route(
            "/v1/accounts/:id/mute",
            scoped(post(accounts::mute_account), WRITE_ACCOUNTS),
        )
        .route(
            "/v1/accounts/:id/unmute",
            scoped(post(accounts::unmute_account), WRITE_ACCOUNTS),
        )
        // Blocks & Mutes
        .route(
            "/v1/blocks",
            scoped(get(accounts::get_blocks), READ_ACCOUNTS),
        )
        .route("/v1/mutes", scoped(get(accounts::get_mutes), READ_ACCOUNTS))
        // Follow Requests
        .route(
            "/v1/follow_requests",
            scoped(get(accounts::get_follow_requests), READ_ACCOUNTS),
        )
        .route(
            "/v1/follow_requests/:id",
            scoped(get(accounts::get_follow_request), READ_ACCOUNTS),
        )
        .route(
            "/v1/follow_requests/:id/authorize",
            scoped(post(accounts::authorize_follow_request), FOLLOW),
        )
        .route(
            "/v1/follow_requests/:id/reject",
            scoped(post(accounts::reject_follow_request), FOLLOW),
        )
        // Statuses - write operations require auth
        .route(
            "/v1/statuses",
            scoped(post(statuses::create_status), WRITE_STATUSES),
        )
        .route(
            "/v1/statuses/:id",
            scoped(delete(statuses::delete_status), WRITE_STATUSES),
        )
        .route(
            "/v1/statuses/:id/source",
            scoped(get(statuses::get_status_source), READ_STATUSES),
        )
        .route(
            "/v1/statuses/:id/favourite",
            scoped(post(statuses::favourite_status), WRITE_FAVOURITES),
        )
        .route(
            "/v1/statuses/:id/unfavourite",
            scoped(post(statuses::unfavourite_status), WRITE_FAVOURITES),
        )
        .route(
            "/v1/statuses/:id/reblog",
            scoped(post(statuses::reblog_status), WRITE_STATUSES),
        )
        .route(
            "/v1/statuses/:id/unreblog",
            scoped(post(statuses::unreblog_status), WRITE_STATUSES),
        )
        .route(
            "/v1/statuses/:id/bookmark",
            scoped(post(statuses::bookmark_status), WRITE_STATUSES),
        )
        .route(
            "/v1/statuses/:id/unbookmark",
            scoped(post(statuses::unbookmark_status), WRITE_STATUSES),
        )
        .route(
            "/v1/statuses/:id",
            scoped(put(statuses::update_status), WRITE_STATUSES),
        )
        .route(
            "/v1/statuses/:id/history",
            scoped(get(statuses::get_status_history), READ_STATUSES),
        )
        .route(
            "/v1/statuses/:id/pin",
            scoped(post(statuses::pin_status), WRITE_STATUSES),
        )
        .route(
            "/v1/statuses/:id/unpin",
            scoped(post(statuses::unpin_status), WRITE_STATUSES),
        )
        .route(
            "/v1/statuses/:id/mute",
            scoped(post(statuses::mute_status), WRITE_STATUSES),
        )
        .route(
            "/v1/statuses/:id/unmute",
            scoped(post(statuses::unmute_status), WRITE_STATUSES),
        )
        // Timelines - require auth (except public which is in public_routes)
        .route(
            "/v1/timelines/home",
            scoped(get(timelines::home_timeline), READ_STATUSES),
        )
        .route(
            "/v1/timelines/tag/:hashtag",
            scoped(get(timelines::tag_timeline), READ_STATUSES),
        )
        .route(
            "/v1/timelines/list/:list_id",
            scoped(get(timelines::list_timeline), READ_STATUSES),
        )
        // Notifications
        .route(
            "/v1/notifications",
            scoped(get(notifications::get_notifications), READ_NOTIFICATIONS),
        )
        .route(
            "/v2/notifications",
            scoped(get(notifications::get_notifications_v2), READ_NOTIFICATIONS),
        )
        .route(
            "/v1/notifications/:id",
            scoped(get(notifications::get_notification), READ_NOTIFICATIONS),
        )
        .route(
            "/v1/notifications/:id/dismiss",
            scoped(
                post(notifications::dismiss_notification),
                WRITE_NOTIFICATIONS,
            ),
        )
        .route(
            "/v1/notifications/clear",
            scoped(
                post(notifications::clear_notifications),
                WRITE_NOTIFICATIONS,
            ),
        )
        .route(
            "/v1/notifications/unread_count",
            scoped(get(notifications::get_unread_count), READ_NOTIFICATIONS),
        )
        .route(
            "/v1/push/subscription",
            scoped_all(
                get(push::get_subscription)
                    .post(push::create_subscription)
                    .put(push::update_subscription)
                    .delete(push::delete_subscription),
                READ_NOTIFICATIONS,
            ),
        )
        .route(
            "/v1/markers",
            scoped(get(markers::get_markers), READ_STATUSES),
        )
        .route(
            "/v1/markers",
            scoped(post(markers::save_markers), WRITE_STATUSES),
        )
        // Media
        .route("/v1/media", scoped(post(media::upload_media), WRITE_MEDIA))
        .route(
            "/v2/media",
            scoped(post(media::upload_media_v2), WRITE_MEDIA),
        )
        .route(
            "/v1/media/:id",
            scoped(get(media::get_media), READ_STATUSES),
        )
        .route(
            "/v1/media/:id",
            scoped(put(media::update_media), WRITE_MEDIA),
        )
        // Lists
        .route("/v1/lists", scoped(get(lists::get_lists), READ_LISTS))
        .route("/v1/lists/:id", scoped(get(lists::get_list), READ_LISTS))
        .route("/v1/lists", scoped(post(lists::create_list), WRITE_LISTS))
        .route(
            "/v1/lists/:id",
            scoped(put(lists::update_list), WRITE_LISTS),
        )
        .route(
            "/v1/lists/:id",
            scoped(delete(lists::delete_list), WRITE_LISTS),
        )
        .route(
            "/v1/lists/:id/accounts",
            scoped(get(lists::get_list_accounts), READ_LISTS),
        )
        .route(
            "/v1/lists/:id/accounts",
            scoped(post(lists::add_list_accounts), WRITE_LISTS),
        )
        .route(
            "/v1/lists/:id/accounts",
            scoped(delete(lists::delete_list_accounts), WRITE_LISTS),
        )
        // Filters
        .route(
            "/v1/filters",
            scoped(get(filters::get_filters), READ_FILTERS),
        )
        .route(
            "/v1/filters/:id",
            scoped(get(filters::get_filter), READ_FILTERS),
        )
        .route(
            "/v1/filters",
            scoped(post(filters::create_filter), WRITE_FILTERS),
        )
        .route(
            "/v1/filters/:id",
            scoped(put(filters::update_filter), WRITE_FILTERS),
        )
        .route(
            "/v1/filters/:id",
            scoped(delete(filters::delete_filter), WRITE_FILTERS),
        )
        .route(
            "/v2/filters",
            scoped(get(filters::get_filters_v2), READ_FILTERS),
        )
        // Bookmarks / Favourites
        .route(
            "/v1/bookmarks",
            scoped(get(bookmarks::get_bookmarks), READ_STATUSES),
        )
        .route(
            "/v1/favourites",
            scoped(get(bookmarks::get_favourites), READ_STATUSES),
        )
        // Search
        .route("/v1/search", scoped(get(search::search_v1), READ_SEARCH))
        .route("/v2/search", scoped(get(search::search_v2), READ_SEARCH))
        // Polls
        .route("/v1/polls/:id", scoped(get(polls::get_poll), READ_STATUSES))
        .route(
            "/v1/polls/:id/votes",
            scoped(post(polls::vote_in_poll), WRITE_STATUSES),
        )
        // Scheduled Statuses
        .route(
            "/v1/scheduled_statuses",
            scoped(
                get(scheduled_statuses::get_scheduled_statuses),
                READ_STATUSES,
            ),
        )
        .route(
            "/v1/scheduled_statuses/:id",
            scoped(get(scheduled_statuses::get_scheduled_status), READ_STATUSES),
        )
        .route(
            "/v1/scheduled_statuses/:id",
            scoped(
                put(scheduled_statuses::update_scheduled_status),
                WRITE_STATUSES,
            ),
        )
        .route(
            "/v1/scheduled_statuses/:id",
            scoped(
                delete(scheduled_statuses::delete_scheduled_status),
                WRITE_STATUSES,
            ),
        )
        // Conversations
        .route(
            "/v1/conversations",
            scoped(get(conversations::get_conversations), READ_STATUSES),
        )
        .route(
            "/v1/conversations/:id",
            scoped(delete(conversations::delete_conversation), WRITE_STATUSES),
        )
        .route(
            "/v1/conversations/:id/read",
            scoped(post(conversations::mark_conversation_read), WRITE_STATUSES),
        )
        // Streaming API
        .route(
            "/v1/streaming/health",
            scoped(get(streaming::streaming_health), READ_STATUSES),
        )
        .route(
            "/v1/streaming",
            scoped(get(streaming::stream_root), SESSION_ONLY),
        )
        .route(
            "/v1/streaming/user",
            scoped_all(get(streaming::stream_user), READ_USER_STREAM),
        )
        .route(
            "/v1/streaming/public",
            scoped(get(streaming::stream_public), READ_STATUSES),
        )
        .route(
            "/v1/streaming/public/local",
            scoped(get(streaming::stream_public_local), READ_STATUSES),
        )
        .route(
            "/v1/streaming/hashtag",
            scoped(get(streaming::stream_hashtag), READ_STATUSES),
        )
        .route(
            "/v1/streaming/list",
            scoped(get(streaming::stream_list), READ_STATUSES),
        )
        .route(
            "/v1/streaming/direct",
            scoped(get(streaming::stream_direct), READ_STATUSES),
        )
        // Admin API
        .route(
            "/v1/admin/accounts",
            scoped(get(admin::list_accounts), SESSION_ONLY),
        )
        .route(
            "/v1/admin/accounts/:id",
            scoped(get(admin::get_account), SESSION_ONLY),
        )
        .route(
            "/v1/admin/accounts/:id/action",
            scoped(post(admin::account_action), SESSION_ONLY),
        )
        .route(
            "/v1/admin/reports",
            scoped(get(admin::list_reports), SESSION_ONLY),
        )
        .route(
            "/v1/admin/domain_blocks",
            scoped(get(admin::list_domain_blocks_v1), SESSION_ONLY),
        )
        .route(
            "/v1/admin/domain_blocks",
            scoped(post(admin::create_domain_block_v1), SESSION_ONLY),
        )
        .route(
            "/v1/admin/domain_blocks/:id",
            scoped(delete(admin::delete_domain_block_v1), SESSION_ONLY),
        );

    // Merge public and authenticated routes
    // CurrentUser extractor reads session populated by require_auth middleware.
    public_routes.merge(authenticated_routes)
}
