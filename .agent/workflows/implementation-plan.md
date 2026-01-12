---
description: RustResort Implementation Plan
---

# RustResort Implementation Plan

Based on the documentation (ARCHITECTURE.md, STORAGE_STRATEGY.md, CLOUDFLARE.md, DEVELOPMENT.md), this plan outlines the implementation steps.

## Phase 1: Foundation (Current Focus)

### Step 1: Error Handling Implementation
- [ ] Complete `error.rs` - implement `IntoResponse` for `AppError`
- [ ] Add JSON error responses
- [ ] Add proper HTTP status code mapping

### Step 2: Configuration Loading
- [ ] Implement `config::AppConfig::load()`
- [ ] Support TOML file loading (default.toml, local.toml)
- [ ] Support environment variable overrides
- [ ] Add configuration validation

### Step 3: Data Layer - Models
- [ ] Define core data models in `data/models.rs`:
  - Account
  - Status
  - MediaAttachment
  - Notification
  - Follow/Follower
  - CachedStatus
  - CachedProfile

### Step 4: Data Layer - Database
- [ ] Implement `data/database.rs`:
  - SQLite connection pool setup
  - Basic CRUD operations for each model
  - Migration support
- [ ] Create initial migration files

### Step 5: Data Layer - Cache
- [ ] Implement `data/cache.rs`:
  - TimelineCache (moka-based, max 2000 items)
  - ProfileCache (moka-based)
  - Cache initialization from DB

### Step 6: Storage Layer
- [ ] Implement `storage/media.rs`:
  - R2 client setup
  - Media upload
  - Public URL generation
- [ ] Implement `storage/backup.rs`:
  - SQLite backup to R2
  - Backup scheduling
  - Retention management

### Step 7: Main Application
- [ ] Implement `main.rs`:
  - Tracing/logging setup
  - AppState initialization
  - Axum router setup
  - Server startup
  - Background task spawning

### Step 8: Basic API Endpoints
- [ ] Implement health check endpoint
- [ ] Implement instance info endpoint
- [ ] Implement WebFinger endpoint

## Phase 2: Testing

### Unit Tests
- [ ] `error.rs` tests
- [ ] `config.rs` tests
- [ ] `data/models.rs` tests
- [ ] `data/database.rs` tests
- [ ] `data/cache.rs` tests
- [ ] `storage/media.rs` tests
- [ ] `storage/backup.rs` tests

### Integration Tests
- [ ] Database integration tests
- [ ] R2 storage integration tests (with MinIO)
- [ ] Configuration loading tests

### E2E Tests
- [ ] Server startup test
- [ ] Health check endpoint test
- [ ] Instance info endpoint test
- [ ] WebFinger endpoint test

## Testing Strategy

### Unit Tests
- Located in the same file as the implementation (`#[cfg(test)] mod tests`)
- Use `tokio::test` for async tests
- Mock external dependencies where appropriate

### Integration Tests
- Located in `tests/integration/`
- Use real SQLite database (in-memory or temp file)
- Use MinIO for R2 testing
- Test cross-module interactions

### E2E Tests
- Located in `tests/e2e/`
- Start full server
- Make HTTP requests
- Verify responses

## Priority Order

1. Error handling (foundation for everything)
2. Configuration (needed for all components)
3. Data models (core domain)
4. Database layer (persistence)
5. Cache layer (performance)
6. Storage layer (media handling)
7. Main application (wiring everything together)
8. Basic API endpoints (verification)
9. Tests (quality assurance)

## Success Criteria

- [ ] All `todo!()` placeholders removed from Phase 1 modules
- [ ] `cargo build` succeeds
- [ ] `cargo test` passes with >70% coverage
- [ ] Server starts successfully
- [ ] Health check endpoint returns 200
- [ ] Configuration loads from file and env vars
- [ ] Database migrations run successfully
- [ ] R2 media upload works (with MinIO in dev)
