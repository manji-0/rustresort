//! Data layer module
//!
//! Handles all data persistence and caching:
//! - SQLite database operations
//! - Timeline cache (volatile)
//! - Profile cache (volatile)

mod cache;
mod database;
mod models;

pub use cache::{ProfileCache, TimelineCache};
pub use database::Database;
pub use models::*;

#[cfg(test)]
mod database_test;
