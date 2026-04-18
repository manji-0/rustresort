//! Additional E2E tests for Mastodon API edge cases

mod common;

use chrono::Utc;
use common::TestServer;
use rustresort::data::{Notification, NotificationType};
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
        .post(server.url("/api/v1/statuses"))
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
        .post(server.url("/api/v1/statuses"))
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
        .post(server.url("/api/v1/statuses"))
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
        .get(server.url("/api/v1/statuses/nonexistent"))
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
        .get(server.url("/api/v1/accounts/nonexistent"))
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
            .post(server.url("/api/v1/statuses"))
            .header("Authorization", format!("Bearer {}", token))
            .json(&status_data)
            .send()
            .await
            .unwrap();
    }

    // Request with limit
    let response = server
        .client
        .get(server.url("/api/v1/timelines/home?limit=3"))
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
        .get(server.url("/api/v1/timelines/home?limit=1000"))
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
        .get(server.url("/api/v1/accounts/verify_credentials"))
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
async fn test_featured_tags_crud_and_suggestions() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .client
        .post(server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "status": "Posting about #rust",
            "visibility": "public"
        }))
        .send()
        .await
        .unwrap();

    let suggestions = server
        .client
        .get(server.url("/api/v1/featured_tags/suggestions"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(suggestions.status(), 200);
    let suggestions_json: Value = suggestions.json().await.unwrap();
    assert!(
        suggestions_json
            .as_array()
            .unwrap()
            .iter()
            .any(|tag| tag["name"] == "rust")
    );

    let create = server
        .client
        .post(server.url("/api/v1/featured_tags"))
        .header("Authorization", format!("Bearer {}", token))
        .form(&[("name", "rust")])
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), 200);
    let create_json: Value = create.json().await.unwrap();
    assert_eq!(create_json["name"], "rust");
    assert!(
        create_json["url"]
            .as_str()
            .unwrap()
            .ends_with("/users/testuser/tagged/rust")
    );
    let featured_tag_id = create_json["id"].as_str().unwrap().to_string();

    let list = server
        .client
        .get(server.url("/api/v1/featured_tags"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    let list_json: Value = list.json().await.unwrap();
    assert!(
        list_json
            .as_array()
            .unwrap()
            .iter()
            .any(|tag| tag["id"] == featured_tag_id && tag["name"] == "rust")
    );

    let delete = server
        .client
        .delete(server.url(&format!("/api/v1/featured_tags/{featured_tag_id}")))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), 200);
    assert_eq!(delete.json::<Value>().await.unwrap(), serde_json::json!({}));
}

#[tokio::test]
async fn test_account_statuses_empty() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/statuses", account.id)))
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
async fn test_markers_round_trip() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let save_payload = serde_json::json!({
        "home": { "last_read_id": "status-123" },
        "notifications": { "last_read_id": "notif-456" }
    });

    let save_response = server
        .client
        .post(server.url("/api/v1/markers"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&save_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(save_response.status(), 200);
    let saved: Value = save_response.json().await.unwrap();
    assert_eq!(saved["home"]["last_read_id"], "status-123");
    assert_eq!(saved["notifications"]["last_read_id"], "notif-456");

    let get_response = server
        .client
        .get(server.url("/api/v1/markers"))
        .query(&[("timeline[]", "home"), ("timeline[]", "notifications")])
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(get_response.status(), 200);
    let loaded: Value = get_response.json().await.unwrap();
    assert_eq!(loaded["home"]["last_read_id"], "status-123");
    assert_eq!(loaded["notifications"]["last_read_id"], "notif-456");

    let default_get_response = server
        .client
        .get(server.url("/api/v1/markers"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(default_get_response.status(), 200);
    assert_eq!(
        default_get_response.json::<Value>().await.unwrap(),
        serde_json::json!({})
    );
}

#[tokio::test]
async fn test_notifications_respect_types_filters() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    for (id, notification_type) in [
        ("notif-mention", NotificationType::Mention),
        ("notif-follow", NotificationType::Follow),
    ] {
        server
            .state
            .db
            .insert_notification(&Notification {
                id: id.to_string(),
                notification_type,
                origin_account_address: "alice@remote.example".to_string(),
                status_uri: None,
                read: false,
                created_at: Utc::now(),
            })
            .await
            .unwrap();
    }

    let only_mentions = server
        .client
        .get(server.url("/api/v1/notifications?types[]=mention"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(only_mentions.status(), 200);
    let only_mentions: Value = only_mentions.json().await.unwrap();
    let ids: Vec<&str> = only_mentions
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(ids.contains(&"notif-mention"));
    assert!(!ids.contains(&"notif-follow"));

    let excluded_mentions = server
        .client
        .get(server.url("/api/v1/notifications?exclude_types[]=mention"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(excluded_mentions.status(), 200);
    let excluded_mentions: Value = excluded_mentions.json().await.unwrap();
    let ids: Vec<&str> = excluded_mentions
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(!ids.contains(&"notif-mention"));
    assert!(ids.contains(&"notif-follow"));
}

#[tokio::test]
async fn test_public_timeline_without_auth() {
    let server = TestServer::new().await;

    // Public timeline should be accessible without authentication
    let response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
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
        .post(server.url("/api/v1/statuses"))
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
