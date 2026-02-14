//! ActivityPub federation module
//!
//! Handles:
//! - Activity processing (inbox)
//! - Activity delivery (outbox)
//! - HTTP Signatures
//! - WebFinger
//! - Actor fetching
//! - Public key caching
//! - Rate limiting

mod activity;
mod delivery;
mod key_cache;
mod rate_limit;
mod signature;
mod webfinger;

pub use activity::{ActivityProcessor, ActivityType};
pub use delivery::ActivityDelivery;
pub use key_cache::{CacheStats, PublicKeyCache};
pub use rate_limit::{RateLimitStats, RateLimiter, extract_domain};
pub use signature::{
    extract_actor_domain, extract_signature_key_id, fetch_public_key, key_id_matches_actor,
    sign_request, verify_signature,
};
pub use webfinger::{WebFingerResult, resolve_webfinger};
