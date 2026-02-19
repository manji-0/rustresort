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
pub use delivery::{ActivityDelivery, DeliveryResult};
pub use key_cache::{CacheStats, PublicKeyCache};
pub use rate_limit::{RateLimitStats, RateLimiter, extract_domain};
pub use signature::{
    fetch_public_key, key_id_matches_actor, parse_signature_header, sign_request, verify_signature,
};
pub use webfinger::{
    WebFingerResponse, WebFingerResult, generate_webfinger_response, resolve_webfinger,
};
