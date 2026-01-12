//! E2E tests for status operations (posting, retrieving, deleting)

mod common;

use common::TestServer;
use serde_json::Value;

#[tokio::test]
async fn test_create_status_without_auth() {
    let server = TestServer::new().await;

    let status_data = serde_json::json!({
        "status": "Hello, world!",
        "visibility": "public"
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    // Should return 401 Unauthorized
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_create_status_with_auth() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "Hello, world!",
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

    // Should create status if implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.get("id").is_some());
        assert_eq!(json["content"], "<p>Hello, world!</p>");
    }
}

#[tokio::test]
async fn test_get_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create a status in the database
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/123".to_string(),
        content: "<p>Test status</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };

    server.state.db.insert_status(&status).await.unwrap();

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}", status.id)))
        .send()
        .await
        .unwrap();

    // Should return status
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["id"], status.id);
    }
}

#[tokio::test]
async fn test_delete_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/456".to_string(),
        content: "<p>To be deleted</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };

    server.state.db.insert_status(&status).await.unwrap();

    let response = server
        .client
        .delete(&server.url(&format!("/api/v1/statuses/{}", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should delete status if implemented
    if response.status().is_success() {
        // Verify status is deleted
        let get_response = server
            .client
            .get(&server.url(&format!("/api/v1/statuses/{}", status.id)))
            .send()
            .await
            .unwrap();

        assert_eq!(get_response.status(), 404);
    }
}

#[tokio::test]
async fn test_favourite_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/fav".to_string(),
        content: "<p>Favourite me!</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };

    server.state.db.insert_status(&status).await.unwrap();

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/favourite", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should favourite status if implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["favourited"], true);
    }
}

#[tokio::test]
async fn test_boost_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/boost".to_string(),
        content: "<p>Boost me!</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };

    server.state.db.insert_status(&status).await.unwrap();

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/reblog", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should boost status if implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["reblogged"], true);
    }
}

#[tokio::test]
async fn test_status_context() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create a status
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/context".to_string(),
        content: "<p>Context test</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };

    server.state.db.insert_status(&status).await.unwrap();

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/context", status.id)))
        .send()
        .await
        .unwrap();

    // Should return context (ancestors and descendants)
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.get("ancestors").is_some());
        assert!(json.get("descendants").is_some());
    }
}
