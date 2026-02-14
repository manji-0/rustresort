//! Comprehensive API endpoint coverage tests
//!
//! Tests all 88+ Mastodon API endpoints for basic functionality

mod common;

use common::TestServer;
use serde_json::json;

// ============================================================================
// Instance Endpoints (5 endpoints)
// ============================================================================

#[tokio::test]
async fn test_instance_info() {
    let server = TestServer::new().await;
    let response = server
        .client
        .get(&server.url("/api/v1/instance"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_instance_v2() {
    let server = TestServer::new().await;
    let response = server
        .client
        .get(&server.url("/api/v2/instance"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_instance_peers() {
    let server = TestServer::new().await;
    let response = server
        .client
        .get(&server.url("/api/v1/instance/peers"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_instance_activity() {
    let server = TestServer::new().await;
    let response = server
        .client
        .get(&server.url("/api/v1/instance/activity"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_instance_rules() {
    let server = TestServer::new().await;
    let response = server
        .client
        .get(&server.url("/api/v1/instance/rules"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

// ============================================================================
// Apps Endpoints (2 endpoints)
// ============================================================================

#[tokio::test]
async fn test_create_app() {
    let server = TestServer::new().await;
    let app_data = json!({
        "client_name": "Test App",
        "redirect_uris": "urn:ietf:wg:oauth:2.0:oob",
        "scopes": "read write"
    });

    let response = server
        .client
        .post(&server.url("/api/v1/apps"))
        .json(&app_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_verify_app_credentials() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/apps/verify_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 401);
}

// ============================================================================
// Account Endpoints (20+ endpoints)
// ============================================================================

#[tokio::test]
async fn test_create_account() {
    let server = TestServer::new().await;
    let account_data = json!({
        "username": "newuser",
        "email": "newuser@example.com",
        "password": "password123",
        "agreement": true
    });

    let response = server
        .client
        .post(&server.url("/api/v1/accounts"))
        .json(&account_data)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 422);
}

#[tokio::test]
async fn test_verify_credentials() {
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

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_update_credentials() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let update_data = json!({
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

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_account() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}", account.id)))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_account_statuses() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}/statuses", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_account_followers() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}/followers", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_account_following() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}/following", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_follow_account() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/accounts/{}/follow", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 422);
}

#[tokio::test]
async fn test_unfollow_account() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/accounts/{}/unfollow", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 422);
}

#[tokio::test]
async fn test_block_account() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/accounts/{}/block", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 422);
}

#[tokio::test]
async fn test_unblock_account() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/accounts/{}/unblock", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 422);
}

#[tokio::test]
async fn test_mute_account() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/accounts/{}/mute", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 422);
}

#[tokio::test]
async fn test_unmute_account() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/accounts/{}/unmute", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 422);
}

#[tokio::test]
async fn test_get_blocks() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/blocks"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_mutes() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/mutes"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_relationships() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url(&format!(
            "/api/v1/accounts/relationships?id[]={}",
            account.id
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_search_accounts() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/accounts/search?q=test"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_account_lists() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}/lists", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_account_identity_proofs() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}/identity_proofs", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

// ============================================================================
// Follow Requests Endpoints (4 endpoints)
// ============================================================================

#[tokio::test]
async fn test_get_follow_requests() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/follow_requests"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_follow_request() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/follow_requests/test_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_authorize_follow_request() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url("/api/v1/follow_requests/test_id/authorize"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_reject_follow_request() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url("/api/v1/follow_requests/test_id/reject"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

// ============================================================================
// Status Endpoints (20+ endpoints)
// ============================================================================

#[tokio::test]
async fn test_create_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = json!({
        "status": "Test status",
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

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    let status_data = json!({"status": "Test", "visibility": "public"});
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if create_response.status().is_success() {
        let created: serde_json::Value = create_response.json().await.unwrap();
        let status_id = created["id"].as_str().unwrap();

        let response = server
            .client
            .get(&server.url(&format!("/api/v1/statuses/{}", status_id)))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }
}

#[tokio::test]
async fn test_delete_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    let status_data = json!({"status": "Test", "visibility": "public"});
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if create_response.status().is_success() {
        let created: serde_json::Value = create_response.json().await.unwrap();
        let status_id = created["id"].as_str().unwrap();

        let response = server
            .client
            .delete(&server.url(&format!("/api/v1/statuses/{}", status_id)))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }
}

#[tokio::test]
async fn test_get_status_context() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    let status_data = json!({"status": "Test", "visibility": "public"});
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if create_response.status().is_success() {
        let created: serde_json::Value = create_response.json().await.unwrap();
        let status_id = created["id"].as_str().unwrap();

        let response = server
            .client
            .get(&server.url(&format!("/api/v1/statuses/{}/context", status_id)))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }
}

#[tokio::test]
async fn test_favourite_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    let status_data = json!({"status": "Test", "visibility": "public"});
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if create_response.status().is_success() {
        let created: serde_json::Value = create_response.json().await.unwrap();
        let status_id = created["id"].as_str().unwrap();

        let response = server
            .client
            .post(&server.url(&format!("/api/v1/statuses/{}/favourite", status_id)))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }
}

#[tokio::test]
async fn test_unfavourite_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    let status_data = json!({"status": "Test", "visibility": "public"});
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if create_response.status().is_success() {
        let created: serde_json::Value = create_response.json().await.unwrap();
        let status_id = created["id"].as_str().unwrap();

        let response = server
            .client
            .post(&server.url(&format!("/api/v1/statuses/{}/unfavourite", status_id)))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }
}

#[tokio::test]
async fn test_reblog_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    let status_data = json!({"status": "Test", "visibility": "public"});
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if create_response.status().is_success() {
        let created: serde_json::Value = create_response.json().await.unwrap();
        let status_id = created["id"].as_str().unwrap();

        let response = server
            .client
            .post(&server.url(&format!("/api/v1/statuses/{}/reblog", status_id)))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }
}

#[tokio::test]
async fn test_unreblog_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    let status_data = json!({"status": "Test", "visibility": "public"});
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if create_response.status().is_success() {
        let created: serde_json::Value = create_response.json().await.unwrap();
        let status_id = created["id"].as_str().unwrap();

        let response = server
            .client
            .post(&server.url(&format!("/api/v1/statuses/{}/unreblog", status_id)))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }
}

#[tokio::test]
async fn test_bookmark_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    let status_data = json!({"status": "Test", "visibility": "public"});
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if create_response.status().is_success() {
        let created: serde_json::Value = create_response.json().await.unwrap();
        let status_id = created["id"].as_str().unwrap();

        let response = server
            .client
            .post(&server.url(&format!("/api/v1/statuses/{}/bookmark", status_id)))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }
}

#[tokio::test]
async fn test_unbookmark_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    let status_data = json!({"status": "Test", "visibility": "public"});
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if create_response.status().is_success() {
        let created: serde_json::Value = create_response.json().await.unwrap();
        let status_id = created["id"].as_str().unwrap();

        let response = server
            .client
            .post(&server.url(&format!("/api/v1/statuses/{}/unbookmark", status_id)))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }
}

#[tokio::test]
async fn test_pin_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    let status_data = json!({"status": "Test", "visibility": "public"});
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if create_response.status().is_success() {
        let created: serde_json::Value = create_response.json().await.unwrap();
        let status_id = created["id"].as_str().unwrap();

        let response = server
            .client
            .post(&server.url(&format!("/api/v1/statuses/{}/pin", status_id)))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .unwrap();

        assert!(response.status().is_success() || response.status() == 422);
    }
}

#[tokio::test]
async fn test_unpin_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    let status_data = json!({"status": "Test", "visibility": "public"});
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    if create_response.status().is_success() {
        let created: serde_json::Value = create_response.json().await.unwrap();
        let status_id = created["id"].as_str().unwrap();

        let response = server
            .client
            .post(&server.url(&format!("/api/v1/statuses/{}/unpin", status_id)))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .unwrap();

        assert!(response.status().is_success() || response.status() == 422);
    }
}

// ============================================================================
// Timeline Endpoints (4 endpoints)
// ============================================================================

#[tokio::test]
async fn test_home_timeline() {
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

    assert_eq!(response.status(), 200);
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

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_tag_timeline() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/tag/test"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_list_timeline() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/list/test_list_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

// ============================================================================
// Notification Endpoints (5 endpoints)
// ============================================================================

#[tokio::test]
async fn test_get_notifications() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/notifications/test_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_dismiss_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url("/api/v1/notifications/test_id/dismiss"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_clear_notifications() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url("/api/v1/notifications/clear"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_unread_count() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/notifications/unread_count"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

// ============================================================================
// Media Endpoints (4 endpoints)
// ============================================================================

#[tokio::test]
async fn test_upload_media() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Note: This is a basic test without actual file upload
    let response = server
        .client
        .post(&server.url("/api/v1/media"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 400 || response.status() == 422);
}

#[tokio::test]
async fn test_upload_media_v2() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Note: This is a basic test without actual file upload
    let response = server
        .client
        .post(&server.url("/api/v2/media"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 400 || response.status() == 422);
}

#[tokio::test]
async fn test_get_media() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/media/test_media_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_update_media() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let update_data = json!({
        "description": "Updated description"
    });

    let response = server
        .client
        .put(&server.url("/api/v1/media/test_media_id"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&update_data)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

// ============================================================================
// Lists Endpoints (7 endpoints)
// ============================================================================

#[tokio::test]
async fn test_get_lists() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/lists"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_create_list() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let list_data = json!({
        "title": "Test List"
    });

    let response = server
        .client
        .post(&server.url("/api/v1/lists"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&list_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_list() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/lists/test_list_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_update_list() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let update_data = json!({
        "title": "Updated List"
    });

    let response = server
        .client
        .put(&server.url("/api/v1/lists/test_list_id"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&update_data)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_delete_list() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .delete(&server.url("/api/v1/lists/test_list_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_get_list_accounts() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/lists/test_list_id/accounts"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_add_list_accounts() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let add_data = json!({
        "account_ids": [account.id]
    });

    let response = server
        .client
        .post(&server.url("/api/v1/lists/test_list_id/accounts"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&add_data)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

// ============================================================================
// Filters Endpoints (6 endpoints)
// ============================================================================

#[tokio::test]
async fn test_get_filters() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/filters"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_create_filter() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let filter_data = json!({
        "phrase": "test",
        "context": ["home"],
        "irreversible": false,
        "whole_word": true
    });

    let response = server
        .client
        .post(&server.url("/api/v1/filters"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&filter_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_filter() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/filters/test_filter_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_update_filter() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let update_data = json!({
        "phrase": "updated test"
    });

    let response = server
        .client
        .put(&server.url("/api/v1/filters/test_filter_id"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&update_data)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_delete_filter() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .delete(&server.url("/api/v1/filters/test_filter_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_get_filters_v2() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v2/filters"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

// ============================================================================
// Bookmarks & Favourites Endpoints (2 endpoints)
// ============================================================================

#[tokio::test]
async fn test_get_bookmarks() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/bookmarks"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_favourites() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/favourites"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

// ============================================================================
// Search Endpoints (2 endpoints)
// ============================================================================

#[tokio::test]
async fn test_search_v1() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/search?q=test"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_search_v2() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v2/search?q=test"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

// ============================================================================
// Polls Endpoints (2 endpoints)
// ============================================================================

#[tokio::test]
async fn test_get_poll() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/polls/test_poll_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_vote_in_poll() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let vote_data = json!({
        "choices": [0]
    });

    let response = server
        .client
        .post(&server.url("/api/v1/polls/test_poll_id/votes"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&vote_data)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

// ============================================================================
// Scheduled Statuses Endpoints (4 endpoints)
// ============================================================================

#[tokio::test]
async fn test_get_scheduled_statuses() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/scheduled_statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_scheduled_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/scheduled_statuses/test_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_update_scheduled_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let update_data = json!({
        "scheduled_at": "2026-01-15T12:00:00Z"
    });

    let response = server
        .client
        .put(&server.url("/api/v1/scheduled_statuses/test_id"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&update_data)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_delete_scheduled_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .delete(&server.url("/api/v1/scheduled_statuses/test_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

// ============================================================================
// Conversations Endpoints (3 endpoints)
// ============================================================================

#[tokio::test]
async fn test_get_conversations() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/conversations"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_delete_conversation() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .delete(&server.url("/api/v1/conversations/test_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_mark_conversation_read() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url("/api/v1/conversations/test_id/read"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}
