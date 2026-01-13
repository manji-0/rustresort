# RustResort E2E Tests

This directory contains End-to-End (E2E) tests for RustResort's major scenarios.

## 📁 Directory Structure

```
tests/
├── common/
│   └── mod.rs          # Common test utilities (TestServer helper)
├── e2e_health.rs       # Health check & basic server functionality
├── e2e_wellknown.rs    # .well-known endpoints (WebFinger, NodeInfo)
├── e2e_account.rs      # Account operations (get, update, follow)
├── e2e_status.rs       # Status operations (post, delete, favourite, boost)
├── e2e_timeline.rs     # Timelines (home, public, local, hashtag)
└── e2e_activitypub.rs  # ActivityPub integration (Actor, Inbox, Outbox)
```

## 🚀 Running Tests

### Run All E2E Tests
```bash
cargo test --tests
```

### Run Specific Test Suite
```bash
# Health check tests
cargo test --test e2e_health

# Account-related tests
cargo test --test e2e_account

# Status-related tests
cargo test --test e2e_status

# Timeline-related tests
cargo test --test e2e_timeline

# ActivityPub-related tests
cargo test --test e2e_activitypub

# .well-known-related tests
cargo test --test e2e_wellknown
```

### Run Specific Test Cases
```bash
# Run by test name
cargo test test_health_check
cargo test test_create_status_with_auth

# Run by pattern matching
cargo test timeline
cargo test account
```

### Run with Verbose Output
```bash
# Show standard output
cargo test --test e2e_health -- --nocapture

# Show test names too
cargo test --test e2e_health -- --nocapture --show-output
```

## 🧪 Test Suite Details

### 1. Health Tests (`e2e_health.rs`)
Basic server functionality tests

- ✅ Health check endpoint
- ✅ Server startup confirmation
- ✅ CORS header verification
- ✅ Unknown routes return 404

### 2. WellKnown Tests (`e2e_wellknown.rs`)
Tests for .well-known endpoints required for Fediverse integration

- ✅ WebFinger endpoint
- ✅ NodeInfo discovery
- ✅ host-meta endpoint
- ✅ WebFinger with account

### 3. Account Tests (`e2e_account.rs`)
Account management functionality tests

- ⚠️ Verify credentials without auth (401 error)
- ✅ Verify credentials with auth
- ✅ Get account by ID
- ✅ Update account information
- ✅ List account statuses
- ✅ List followers
- ✅ List following

### 4. Status Tests (`e2e_status.rs`)
Status (post) management functionality tests

- ⚠️ Create status without auth (401 error)
- ✅ Create status with auth
- ✅ Get status
- ✅ Delete status
- ✅ Favourite status
- ✅ Boost (reblog) status
- ✅ Get status context

### 5. Timeline Tests (`e2e_timeline.rs`)
Timeline display functionality tests

- ⚠️ Home timeline without auth (401 error)
- ✅ Home timeline with auth
- ✅ Public timeline
- ✅ Local timeline
- ✅ Pagination
- ✅ Hashtag timeline
- ✅ max_id parameter
- ✅ since_id parameter

### 6. ActivityPub Tests (`e2e_activitypub.rs`)
ActivityPub integration functionality tests

- ✅ Actor endpoint
- ✅ Inbox endpoint
- ✅ Outbox endpoint
- ✅ Followers collection
- ✅ Following collection
- ✅ Status Activity representation
- ✅ Shared Inbox
- ✅ Content negotiation

## 🛠️ TestServer Helper

Common test utilities implemented in `tests/common/mod.rs`.

### Main Features

```rust
use common::TestServer;

#[tokio::test]
async fn my_test() {
    // Start test server
    let server = TestServer::new().await;
    
    // Send HTTP request
    let response = server.client
        .get(&server.url("/health"))
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
}
```

### Available Methods

- `TestServer::new()` - Create a new test server instance
- `server.url(path)` - Generate complete URL
- `server.create_test_account()` - Create test account
- `server.create_test_token()` - Create test auth token
- `server.state` - Access to AppState
- `server.client` - HTTP client

### Characteristics

- **Independence**: Each test uses an isolated server instance
- **Temporary DB**: New SQLite database created per test
- **Auto-port**: OS automatically assigns free port
- **Auto-cleanup**: Resources released automatically after test

## 📊 Current Test Status

**Total Tests**: 39  
**Passing**: 35 (89.7%)  
**Failing**: 3 (7.7%)  

### Failing Tests

The following 3 tests fail because authentication middleware implementation is incomplete:

1. `test_verify_credentials_without_auth` - Returns 404 instead of 401
2. `test_create_status_without_auth` - Returns 404 instead of 401
3. `test_home_timeline_without_auth` - Returns 404 instead of 401

These will be resolved naturally as implementation progresses.

## 🔧 Adding Tests

### Adding New Test Cases

Add to existing test file:

```rust
#[tokio::test]
async fn test_my_new_feature() {
    let server = TestServer::new().await;
    
    // Test logic
    let response = server.client
        .get(&server.url("/api/v1/my_endpoint"))
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
}
```

### Adding New Test Suite

1. Create `tests/e2e_myfeature.rs`
2. Add `mod common;`
3. Implement test cases

```rust
mod common;

use common::TestServer;

#[tokio::test]
async fn test_my_feature() {
    let server = TestServer::new().await;
    // Test logic
}
```

## 📈 Best Practices

### Test Independence
- Each test is independent of other tests
- No dependency on test execution order
- No shared state

### Test Data
- Create new data for each test
- Avoid hardcoded IDs
- Use temporary databases

### Assertions
- Clear assertion messages
- Test multiple conditions individually
- Cover edge cases

### Performance
- Avoid unnecessary waits
- Keep tests parallelizable
- Share heavy setup

## 🐛 Troubleshooting

### Tests Timing Out
```bash
# Extend timeout duration
RUST_TEST_TIMEOUT=60 cargo test
```

### Port Already in Use
TestServer automatically uses free ports, so this issue typically doesn't occur.

### Database Errors
Check permissions on temporary directory.

### Parallel Execution Issues
```bash
# Run sequentially
cargo test -- --test-threads=1
```

## 📚 Related Documentation

- [E2E Test Report](../docs/E2E_TEST_REPORT.md) - Detailed test execution report
- [DEVELOPMENT.md](../docs/DEVELOPMENT.md) - Development guide
- [API.md](../docs/API.md) - API specification
- [ROADMAP.md](../docs/ROADMAP.md) - Implementation roadmap

## 🎯 Future Plans

### Short-term
- [ ] Fix authentication middleware
- [ ] Add OAuth2 flow tests
- [ ] Add media upload tests

### Mid-term
- [ ] Add HTTP Signatures tests
- [ ] Add Activity delivery tests
- [ ] Add performance tests

### Long-term
- [ ] Build integration test environment
- [ ] Integrate with CI/CD pipeline
- [ ] Auto-generate coverage reports
