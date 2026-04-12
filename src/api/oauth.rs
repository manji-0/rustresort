//! OAuth endpoints.

use axum::{
    Router,
    extract::FromRef,
    routing::{get, post},
};

use super::mastodon::apps::{authorize, create_token, revoke_token};
use crate::AppsApiState;

pub fn oauth_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AppsApiState: FromRef<S>,
{
    Router::new()
        .route("/oauth/authorize", get(authorize))
        .route("/oauth/token", post(create_token))
        .route("/oauth/revoke", post(revoke_token))
}
