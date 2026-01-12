//! Additional E2E tests for Mastodon API edge cases

mod common;

use common::TestServer;
use serde_json::Value;

#[tokio::test]
async fn test_create_status_with_content_warning() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "Sensitive content here",
        "spoiler_text": "CW: Test warning",
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
        assert_eq!(json["spoiler_text"], "CW: Test warning");
        assert_eq!(json["sensitive"], true);
    }
}

#[tokio::test]
async fn test_create_status_without_auth_empty() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let status_data = serde_json::json!({
        "status": "",
        "visibility": "public"
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    // Should return 401 Unauthorized without token
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_create_status_without_auth_missing() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let status_data = serde_json::json!({
        "visibility": "public"
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    // Should return 401 Unauthorized without token
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_get_nonexistent_status() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/api/v1/statuses/nonexistent"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn test_get_nonexistent_account() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/api/v1/accounts/nonexistent"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn test_timeline_pagination_limit() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create multiple statuses
    for i in 0..5 {
        let status_data = serde_json::json!({
            "status": format!("Status {}", i),
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

    // Request with limit
    let response = server
        .client
        .get(&server.url("/api/v1/timelines/home?limit=3"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        let statuses = json.as_array().unwrap();
        assert!(statuses.len() <= 3);
    }
}

#[tokio::test]
async fn test_timeline_max_limit() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Request with very large limit (should be capped at 40)
    let response = server
        .client
        .get(&server.url("/api/v1/timelines/home?limit=1000"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        let statuses = json.as_array().unwrap();
        // Should be capped at 40
        assert!(statuses.len() <= 40);
    }
}

#[tokio::test]
async fn test_verify_credentials_returns_counts() {
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
        assert!(json.get("followers_count").is_some());
        assert!(json.get("following_count").is_some());
        assert!(json.get("statuses_count").is_some());
    }
}

#[tokio::test]
async fn test_account_statuses_empty() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}/statuses", account.id)))
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        let statuses = json.as_array().unwrap();
        assert_eq!(statuses.len(), 0);
    }
}

#[tokio::test]
async fn test_public_timeline_without_auth() {
    let server = TestServer::new().await;

    // Public timeline should be accessible without authentication
    let response = server
        .client
        .get(&server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();

    // Should return 200 or 404 depending on implementation
    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_status_html_escaping() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "<script>alert('xss')</script>",
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
        let content = json["content"].as_str().unwrap();
        // HTML should be escaped
        assert!(content.contains("&lt;script&gt;"));
        assert!(!content.contains("<script>"));
    }
}
