//! GitHub OAuth authentication
//!
//! Handles:
//! - GitHub OAuth flow
//! - Session management
//! - Authentication middleware

mod middleware;
mod oauth;
pub mod session;

pub use middleware::{CurrentUser, require_auth};
pub use oauth::auth_router;
pub use session::{Session, create_session_token, verify_session_token};
