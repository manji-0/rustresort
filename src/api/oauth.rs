//! OAuth endpoints

use axum::{
    Router,
    extract::FromRef,
    middleware,
    routing::{get, post},
};
use std::sync::Arc;

use super::mastodon::apps::{authorize, create_token, revoke_token};
use crate::AppsApiState;
use crate::auth::require_session_auth;
use crate::config::AppConfig;

/// Create OAuth router
///
/// These routes do NOT require authentication (they provide authentication).
pub fn oauth_router<S>(config: Arc<AppConfig>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AppsApiState: FromRef<S>,
{
    let authorize_routes = Router::new()
        .route("/authorize", get(authorize))
        .route_layer(middleware::from_fn_with_state(
            config.clone(),
            require_session_auth,
        ));

    Router::new()
        .merge(authorize_routes)
        .route("/token", post(create_token))
        .route("/revoke", post(revoke_token))
}
