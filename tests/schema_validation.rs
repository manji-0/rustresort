//! Schema validation tests for Mastodon API
//!
//! These tests validate that RustResort's API responses conform to the
//! Mastodon API schema as defined by GoToSocial's swagger.yaml

mod common;

use common::TestServer;
use common::schema_validator::{load_test_schema, validate_against_schema};
use serde_json::Value;

#[tokio::test]
async fn test_account_schema_verify_credentials() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/accounts/verify_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        let schema = load_test_schema("account");

        match validate_against_schema(&json, &schema) {
            Ok(_) => println!("✓ Account schema validation passed"),
            Err(errors) => {
                eprintln!("✗ Account schema validation failed:");
                for error in &errors {
                    eprintln!("  - {}", error);
                }
                eprintln!(
                    "\nActual response:\n{}",
                    serde_json::to_string_pretty(&json).unwrap()
                );
                panic!("Schema validation failed with {} errors", errors.len());
            }
        }
    } else {
        panic!("Request failed with status: {}", response.status());
    }
}

#[tokio::test]
async fn test_account_schema_get_account() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}", account.id)))
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        let schema = load_test_schema("account");

        match validate_against_schema(&json, &schema) {
            Ok(_) => println!("✓ Account schema validation passed"),
            Err(errors) => {
                eprintln!("✗ Account schema validation failed:");
                for error in &errors {
                    eprintln!("  - {}", error);
                }
                eprintln!(
                    "\nActual response:\n{}",
                    serde_json::to_string_pretty(&json).unwrap()
                );
                panic!("Schema validation failed with {} errors", errors.len());
            }
        }
    }
}

#[tokio::test]
async fn test_status_schema_create() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "Test status for schema validation",
        "visibility": "public"
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        let schema = load_test_schema("status");

        match validate_against_schema(&json, &schema) {
            Ok(_) => println!("✓ Status schema validation passed"),
            Err(errors) => {
                eprintln!("✗ Status schema validation failed:");
                for error in &errors {
                    eprintln!("  - {}", error);
                }
                eprintln!(
                    "\nActual response:\n{}",
                    serde_json::to_string_pretty(&json).unwrap()
                );
                panic!("Schema validation failed with {} errors", errors.len());
            }
        }
    } else {
        panic!("Request failed with status: {}", response.status());
    }
}

#[tokio::test]
async fn test_status_schema_get() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    let status_data = serde_json::json!({
        "status": "Test status",
        "visibility": "public"
    });

    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if create_response.status().is_success() {
        let created_status: Value = create_response.json().await.unwrap();
        let status_id = created_status["id"].as_str().unwrap();

        // Get the status
        let response = server
            .client
            .get(&server.url(&format!("/api/v1/statuses/{}", status_id)))
            .send()
            .await
            .unwrap();

        if response.status().is_success() {
            let json: Value = response.json().await.unwrap();
            let schema = load_test_schema("status");

            match validate_against_schema(&json, &schema) {
                Ok(_) => println!("✓ Status schema validation passed"),
                Err(errors) => {
                    eprintln!("✗ Status schema validation failed:");
                    for error in &errors {
                        eprintln!("  - {}", error);
                    }
                    eprintln!(
                        "\nActual response:\n{}",
                        serde_json::to_string_pretty(&json).unwrap()
                    );
                    panic!("Schema validation failed with {} errors", errors.len());
                }
            }
        }
    }
}

#[tokio::test]
async fn test_instance_schema() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/api/v1/instance"))
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        let schema = load_test_schema("instance");

        match validate_against_schema(&json, &schema) {
            Ok(_) => println!("✓ Instance schema validation passed"),
            Err(errors) => {
                eprintln!("✗ Instance schema validation failed:");
                for error in &errors {
                    eprintln!("  - {}", error);
                }
                eprintln!(
                    "\nActual response:\n{}",
                    serde_json::to_string_pretty(&json).unwrap()
                );
                panic!("Schema validation failed with {} errors", errors.len());
            }
        }
    } else {
        panic!("Request failed with status: {}", response.status());
    }
}

#[tokio::test]
async fn test_timeline_schema() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create some statuses
    for i in 0..3 {
        let status_data = serde_json::json!({
            "status": format!("Timeline test status {}", i),
            "visibility": "public"
        });

        server
            .client
            .post(&server.url("/api/v1/statuses"))
            .header("Authorization", format!("Bearer {}", token))
            .json(&status_data)
            .send()
            .await
            .unwrap();
    }

    // Get home timeline
    let response = server
        .client
        .get(&server.url("/api/v1/timelines/home"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();

        // Timeline should be an array
        assert!(json.is_array(), "Timeline response should be an array");

        let statuses = json.as_array().unwrap();
        let schema = load_test_schema("status");

        // Validate each status in the timeline
        for (idx, status) in statuses.iter().enumerate() {
            match validate_against_schema(status, &schema) {
                Ok(_) => println!("✓ Status {} schema validation passed", idx),
                Err(errors) => {
                    eprintln!("✗ Status {} schema validation failed:", idx);
                    for error in &errors {
                        eprintln!("  - {}", error);
                    }
                    eprintln!(
                        "\nActual response:\n{}",
                        serde_json::to_string_pretty(status).unwrap()
                    );
                    panic!(
                        "Schema validation failed for status {} with {} errors",
                        idx,
                        errors.len()
                    );
                }
            }
        }
    } else {
        panic!("Request failed with status: {}", response.status());
    }
}

#[tokio::test]
async fn test_status_with_media_schema() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Note: This test assumes media upload is implemented
    // For now, we'll test a status without media but with other fields
    let status_data = serde_json::json!({
        "status": "Test status with all fields",
        "visibility": "public",
        "sensitive": true,
        "spoiler_text": "Test warning",
        "language": "en"
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        let schema = load_test_schema("status");

        match validate_against_schema(&json, &schema) {
            Ok(_) => {
                println!("✓ Status with extended fields schema validation passed");

                // Verify specific fields
                assert_eq!(json["sensitive"], true, "sensitive field should be true");
                assert_eq!(
                    json["spoiler_text"], "Test warning",
                    "spoiler_text should match"
                );
            }
            Err(errors) => {
                eprintln!("✗ Status schema validation failed:");
                for error in &errors {
                    eprintln!("  - {}", error);
                }
                eprintln!(
                    "\nActual response:\n{}",
                    serde_json::to_string_pretty(&json).unwrap()
                );
                panic!("Schema validation failed with {} errors", errors.len());
            }
        }
    } else {
        panic!("Request failed with status: {}", response.status());
    }
}
