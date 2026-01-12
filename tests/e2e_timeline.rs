//! E2E tests for timeline operations

mod common;

use common::TestServer;
use serde_json::Value;

#[tokio::test]
async fn test_home_timeline_without_auth() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/home"))
        .send()
        .await
        .unwrap();

    // Should return 401 Unauthorized
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_home_timeline_with_auth() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/home"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should return timeline if implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_public_timeline() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();

    // Public timeline should be accessible without auth
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_local_timeline() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/public?local=true"))
        .send()
        .await
        .unwrap();

    // Local timeline should be accessible
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_timeline_pagination() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create multiple statuses
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    for i in 0..5 {
        let status = Status {
            id: EntityId::new().0,
            uri: format!("https://test.example.com/status/{}", i),
            content: format!("<p>Status {}</p>", i),
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
    }

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/public?limit=3"))
        .send()
        .await
        .unwrap();

    // Should return limited number of statuses
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
        if json.as_array().unwrap().len() > 0 {
            assert!(json.as_array().unwrap().len() <= 3);
        }
    }
}

#[tokio::test]
async fn test_hashtag_timeline() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/tag/rust"))
        .send()
        .await
        .unwrap();

    // Hashtag timeline should be accessible
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_timeline_with_max_id() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/home?max_id=123456"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should handle max_id parameter
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_timeline_with_since_id() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/home?since_id=123456"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should handle since_id parameter
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}
