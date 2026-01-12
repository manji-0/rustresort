# API Schema Validation Tests

This directory contains JSON Schema definitions and validation tests for RustResort's Mastodon API implementation.

## Overview

These tests validate that RustResort's API responses conform to the Mastodon API specification, using GoToSocial's `swagger.yaml` as the reference implementation.

## Schema Files

JSON Schema definitions for Mastodon API objects:

- `account.json` - Account object
- `status.json` - Status/Post object
- `instance.json` - Instance information
- `relationship.json` - Account relationship
- `poll.json` - Poll object

## Running Tests

```bash
# Run all schema validation tests
cargo test --test schema_validation

# Run with output
cargo test --test schema_validation -- --nocapture

# Run specific test
cargo test --test schema_validation test_account_schema_get_account
```

## Test Coverage

### ✅ Currently Tested

- Account endpoints (GET /api/v1/accounts/:id)
- Instance endpoint (GET /api/v1/instance)
- Status endpoints (GET /api/v1/statuses/:id)

### 🔄 Partially Tested (Auth Issues)

- Account verification (GET /api/v1/accounts/verify_credentials)
- Status creation (POST /api/v1/statuses)
- Timeline endpoints (GET /api/v1/timelines/*)

### 📋 To Be Added

- Relationships
- Notifications
- Polls
- Filters
- Lists
- Follow requests
- Blocks and Mutes
- Scheduled statuses
- Conversations
- Search

## Adding New Schemas

1. Extract schema from `gotosocial/docs/api/swagger.yaml`
2. Convert to JSON Schema format
3. Save as `tests/schemas/{object_name}.json`
4. Add test in `tests/schema_validation.rs`

Example:

```rust
#[tokio::test]
async fn test_my_endpoint_schema() {
    let server = TestServer::new().await;
    let response = server.client.get(&server.url("/api/v1/my_endpoint")).send().await.unwrap();
    
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        let schema = load_test_schema("my_object");
        
        match validate_against_schema(&json, &schema) {
            Ok(_) => println!("✓ Schema validation passed"),
            Err(errors) => {
                eprintln!("✗ Schema validation failed:");
                for error in &errors {
                    eprintln!("  - {}", error);
                }
                panic!("Schema validation failed");
            }
        }
    }
}
```

## Schema Validation Framework

The validation framework is in `tests/common/schema_validator.rs`:

```rust
// Load a schema by name
let schema = load_test_schema("account");

// Validate JSON data against schema
let result = validate_against_schema(&json_data, &schema);
```

## Reference

- **GoToSocial Swagger**: `../../gotosocial/docs/api/swagger.yaml`
- **Mastodon API Docs**: https://docs.joinmastodon.org/api/
- **JSON Schema Spec**: https://json-schema.org/draft-07/schema

## Troubleshooting

### Authentication Errors (401)

If tests fail with 401 Unauthorized:

1. Check test token generation in `tests/common/mod.rs`
2. Verify session secret matches between test and app
3. Ensure account exists before creating token
4. Check middleware configuration in test router

### Schema Validation Errors

If schema validation fails:

1. Check the actual response with `--nocapture` flag
2. Compare against GoToSocial's swagger definition
3. Update schema file if RustResort's implementation differs intentionally
4. Fix API response if it's a bug

### Missing Fields

If required fields are missing:

1. Check database query returns all necessary fields
2. Verify serialization includes all fields
3. Update schema if field is optional in practice

## Contributing

When adding new API endpoints:

1. Add corresponding JSON Schema
2. Add schema validation test
3. Run tests to verify compliance
4. Update this README with new coverage
