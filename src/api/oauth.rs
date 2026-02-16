//! OAuth endpoints

use axum::{
    Router,
    routing::{get, post},
};

use super::mastodon::apps::{authorize, create_token, revoke_token};
use crate::AppState;

/// Create OAuth router
///
/// These routes do NOT require authentication (they provide authentication).
pub fn oauth_router() -> Router<AppState> {
    Router::new()
        .route("/authorize", get(authorize))
        .route("/token", post(create_token))
        .route("/revoke", post(revoke_token))
}
