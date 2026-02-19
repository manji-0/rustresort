//! Data layer module
//!
//! Handles all data persistence and caching:
//! - SQLite database operations
//! - Timeline cache (volatile)
//! - Profile cache (volatile)

mod cache;
mod database;
mod models;
mod sync;

pub use cache::{CachedAttachment, CachedProfile, CachedStatus, ProfileCache, TimelineCache};
pub use database::{Database, TursoSyncOptions};
pub use models::*;
pub use sync::{sync_to_d1, validate_d1_sync_environment};

#[cfg(test)]
mod database_test;
