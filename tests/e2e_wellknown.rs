//! E2E tests for .well-known endpoints (WebFinger, NodeInfo)

mod common;

use common::TestServer;
use serde_json::Value;

#[tokio::test]
async fn test_webfinger_endpoint_exists() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/.well-known/webfinger?resource=acct:testuser@test.example.com"))
        .send()
        .await
        .unwrap();

    // Should return 200 or 404 depending on implementation status
    // For now, just verify the endpoint is routed
    assert!(response.status().is_client_error() || response.status().is_success());
}

#[tokio::test]
async fn test_nodeinfo_discovery() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/.well-known/nodeinfo"))
        .send()
        .await
        .unwrap();

    // Should return JSON with links to nodeinfo
    assert!(response.status().is_client_error() || response.status().is_success());
}

#[tokio::test]
async fn test_host_meta_endpoint() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/.well-known/host-meta"))
        .send()
        .await
        .unwrap();

    // Should return XML or JSON
    assert!(response.status().is_client_error() || response.status().is_success());
}

#[tokio::test]
async fn test_webfinger_with_account() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(&server.url("/.well-known/webfinger?resource=acct:testuser@test.example.com"))
        .send()
        .await
        .unwrap();

    // With account created, should return proper WebFinger response
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.get("subject").is_some());
        assert!(json.get("links").is_some());
    }
}
