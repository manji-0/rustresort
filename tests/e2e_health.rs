//! E2E tests for health check and basic server functionality

mod common;

use common::TestServer;

#[tokio::test]
async fn test_health_check() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/health"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = response.text().await.unwrap();
    assert_eq!(body, "OK");
}

#[tokio::test]
async fn test_server_starts_successfully() {
    let server = TestServer::new().await;

    // Verify server is accessible
    let response = server.client.get(&server.url("/health")).send().await;

    assert!(response.is_ok());
}

#[tokio::test]
async fn test_cors_headers() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/health"))
        .header("Origin", "https://test.example.com")
        .send()
        .await
        .unwrap();

    // CORS should allow the configured instance origin.
    assert!(
        response
            .headers()
            .contains_key("access-control-allow-origin")
    );
}

#[tokio::test]
async fn test_404_for_unknown_routes() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/unknown/route"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn test_metrics_requires_authentication() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/metrics"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_metrics_accepts_session_bearer_token() {
    let server = TestServer::new().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/metrics"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}
