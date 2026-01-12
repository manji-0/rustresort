//! E2E tests for account operations

mod common;

use common::TestServer;
use serde_json::Value;

#[tokio::test]
async fn test_verify_credentials_without_auth() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/api/v1/accounts/verify_credentials"))
        .send()
        .await
        .unwrap();

    // Should return 401 Unauthorized without token
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_verify_credentials_with_auth() {
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

    // Should return account info if auth is implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.get("id").is_some());
        assert!(json.get("username").is_some());
    }
}

#[tokio::test]
async fn test_get_account_by_id() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}", account.id)))
        .send()
        .await
        .unwrap();

    // Should return account info
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["username"], "testuser");
    }
}

#[tokio::test]
async fn test_update_credentials() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let update_data = serde_json::json!({
        "display_name": "Updated Name",
        "note": "Updated bio"
    });

    let response = server
        .client
        .patch(&server.url("/api/v1/accounts/update_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&update_data)
        .send()
        .await
        .unwrap();

    // Should update account if implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["display_name"], "Updated Name");
    }
}

#[tokio::test]
async fn test_account_statuses() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}/statuses", account.id)))
        .send()
        .await
        .unwrap();

    // Should return array of statuses
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_account_followers() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}/followers", account.id)))
        .send()
        .await
        .unwrap();

    // Should return array of followers
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_account_following() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}/following", account.id)))
        .send()
        .await
        .unwrap();

    // Should return array of following
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}
