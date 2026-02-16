//! OAuth endpoints

use axum::{
    Router, middleware,
    routing::{get, post},
};

use super::mastodon::apps::{authorize, create_token, revoke_token};
use crate::AppState;
use crate::auth::require_session_auth;

/// Create OAuth router
///
/// These routes do NOT require authentication (they provide authentication).
pub fn oauth_router(state: AppState) -> Router<AppState> {
    let authorize_routes = Router::new()
        .route("/authorize", get(authorize))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_session_auth,
        ));

    Router::new()
        .merge(authorize_routes)
        .route("/token", post(create_token))
        .route("/revoke", post(revoke_token))
}
