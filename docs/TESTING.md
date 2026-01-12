# Testing Specification

## Overview

RustResort implements comprehensive testing at multiple levels to ensure reliability and API compliance.

## Test Structure

```
tests/
├── common/
│   ├── mod.rs              # Shared test utilities
│   └── schema_validator.rs # JSON schema validation
├── schemas/                # Mastodon API JSON schemas
│   ├── account.json
│   ├── status.json
│   ├── instance.json
│   └── ...
├── e2e_*.rs               # End-to-end tests
└── schema_validation.rs   # Schema compliance tests
```

## Test Categories

### 1. Unit Tests

Located within source files using `#[cfg(test)]` modules.

**Coverage:**
- Data models and conversions
- Federation signature generation/verification
- Rate limiting logic
- Cache operations
- Public key caching

**Example:**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit() {
        // Test rate limiting logic
    }
}
```

### 2. Integration Tests

Located in `tests/` directory, testing complete request/response cycles.

**Test Files:**
- `e2e_health.rs` - Health check endpoints
- `e2e_account.rs` - Account operations
- `e2e_status.rs` - Status creation and retrieval
- `e2e_timeline.rs` - Timeline endpoints
- `e2e_wellknown.rs` - WebFinger and NodeInfo
- `e2e_activitypub.rs` - ActivityPub endpoints
- `e2e_mastodon_api.rs` - Mastodon API compliance
- `e2e_federation_scenarios.rs` - Federation workflows
- `e2e_api_coverage.rs` - API endpoint coverage

### 3. Schema Validation Tests

Validates API responses against Mastodon API schemas extracted from GoToSocial.

**Implementation:**
```rust
#[tokio::test]
async fn test_account_schema() {
    let app = spawn_app().await;
    let response = app.get_account("test_user").await;
    
    validate_schema("account", &response).expect("Schema validation failed");
}
```

**Validated Schemas:**
- Account
- Status
- Instance
- Notification
- Media Attachment
- Poll
- Conversation
- Filter (v1 and v2)
- List
- Relationship
- Scheduled Status
- Tag
- Card
- Application
- Emoji
- Context

### 4. Federation Tests

Tests ActivityPub federation scenarios including:

**Scenarios:**
- Follow/Accept workflow
- Activity delivery
- HTTP signature verification
- Remote actor fetching
- WebFinger resolution
- Inbox processing
- Outbox generation

**Example:**
```rust
#[tokio::test]
async fn test_follow_accept_workflow() {
    // 1. Remote actor sends Follow activity
    // 2. Local server processes Follow
    // 3. Local server sends Accept activity
    // 4. Verify follower relationship created
}
```

## Test Utilities

### Common Test Helpers

Located in `tests/common/mod.rs`:

```rust
/// Spawn test application instance
pub async fn spawn_app() -> TestApp {
    // Initialize test database
    // Start test server
    // Return test client
}

/// Create test user
pub async fn create_test_user(app: &TestApp, username: &str) -> User {
    // Create user in database
    // Generate OAuth token
}

/// Create test status
pub async fn create_test_status(app: &TestApp, user: &User, content: &str) -> Status {
    // Create status via API
}
```

### Schema Validator

Located in `tests/common/schema_validator.rs`:

```rust
/// Validate JSON against schema
pub fn validate_schema(schema_name: &str, json: &Value) -> Result<(), ValidationError> {
    let schema = load_schema(schema_name)?;
    let validator = JSONSchema::compile(&schema)?;
    validator.validate(json)?;
    Ok(())
}
```

## Running Tests

### All Tests
```bash
cargo test
```

### Specific Test Suite
```bash
cargo test e2e_account
cargo test schema_validation
cargo test federation
```

### With Output
```bash
cargo test -- --nocapture
```

### Single Test
```bash
cargo test test_account_schema -- --exact
```

## Test Database

Tests use an in-memory SQLite database that is:
- Created fresh for each test
- Isolated between tests
- Automatically cleaned up

**Configuration:**
```rust
async fn setup_test_db() -> Database {
    let db = Database::new(":memory:").await.unwrap();
    db.run_migrations().await.unwrap();
    db
}
```

## Continuous Integration

Tests run automatically on:
- Pull requests
- Main branch commits
- Release tags

**CI Configuration:**
```yaml
test:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v2
    - uses: actions-rs/toolchain@v1
    - run: cargo test --all-features
```

## Test Coverage

Current coverage targets:
- Unit tests: >80%
- Integration tests: All API endpoints
- Schema validation: All Mastodon API objects
- Federation: Core ActivityPub workflows

## Ignored Tests

Some tests are marked with `#[ignore]` for features not yet implemented:

```rust
#[tokio::test]
#[ignore = "Streaming API not implemented"]
async fn test_streaming_api() {
    // Test implementation
}
```

Run ignored tests:
```bash
cargo test -- --ignored
```

## Performance Tests

Located in `benches/` (future implementation):
- API endpoint latency
- Database query performance
- Federation delivery throughput
- Cache hit rates

## Best Practices

1. **Test Isolation**: Each test should be independent
2. **Clear Names**: Test names should describe what they test
3. **Arrange-Act-Assert**: Follow AAA pattern
4. **Error Messages**: Provide helpful assertion messages
5. **Cleanup**: Tests should clean up resources
6. **Deterministic**: Tests should not rely on timing or randomness

## Schema Extraction

Schemas are extracted from GoToSocial's `swagger.yaml`:

```bash
python scripts/extract_schemas.py
```

This generates JSON schemas in `tests/schemas/` directory.

## Future Enhancements

- [ ] Property-based testing with `proptest`
- [ ] Mutation testing
- [ ] Load testing with `criterion`
- [ ] Contract testing for federation
- [ ] Visual regression testing for UI (if applicable)

## Related Documentation

- [DEVELOPMENT.md](DEVELOPMENT.md) - Development setup
- [API.md](API.md) - API specifications
- [FEDERATION.md](FEDERATION.md) - Federation specifications
