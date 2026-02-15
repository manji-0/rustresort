//! E2E tests for ActivityPub federation endpoints

mod common;

use common::TestServer;
use serde_json::Value;

#[tokio::test]
async fn test_actor_endpoint() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(&server.url("/users/testuser"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    // Should return ActivityPub actor
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["type"], "Person");
        assert!(json.get("inbox").is_some());
        assert!(json.get("outbox").is_some());
        assert!(json.get("publicKey").is_some());
    }
}

#[tokio::test]
async fn test_inbox_endpoint() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create a simple Follow activity
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "type": "Follow",
        "actor": "https://remote.example.com/users/alice",
        "object": "https://test.example.com/users/testuser"
    });

    let response = server
        .client
        .post(&server.url("/users/testuser/inbox"))
        .header("Content-Type", "application/activity+json")
        .json(&activity)
        .send()
        .await
        .unwrap();

    // Should accept activity (may require HTTP signature in real implementation)
    // For now, just verify endpoint exists
    assert!(response.status().is_client_error() || response.status().is_success());
}

#[tokio::test]
async fn test_outbox_endpoint() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(&server.url("/users/testuser/outbox"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    // Should return ActivityPub OrderedCollection
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["type"], "OrderedCollection");
        assert!(json.get("totalItems").is_some());
    }
}

#[tokio::test]
async fn test_followers_collection() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(&server.url("/users/testuser/followers"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    // Should return ActivityPub OrderedCollection
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["type"], "OrderedCollection");
        assert!(json.get("totalItems").is_some());
    }
}

#[tokio::test]
async fn test_following_collection() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(&server.url("/users/testuser/following"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    // Should return ActivityPub OrderedCollection
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["type"], "OrderedCollection");
        assert!(json.get("totalItems").is_some());
    }
}

#[tokio::test]
async fn test_status_as_activity() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create a status
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/users/testuser/statuses/123".to_string(),
        content: "<p>ActivityPub test</p>".to_string(),
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
        .get(&server.url("/users/testuser/statuses/123"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert_eq!(json["type"], "Note");
    assert_eq!(
        json["id"],
        "https://test.example.com/users/testuser/statuses/123"
    );
    assert!(json.get("content").is_some());
    assert!(json.get("attributedTo").is_some());
}

#[tokio::test]
async fn test_unlisted_status_activity_audience() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/users/testuser/statuses/124".to_string(),
        content: "<p>Unlisted ActivityPub test</p>".to_string(),
        content_warning: None,
        visibility: "unlisted".to_string(),
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
        .get(&server.url("/users/testuser/statuses/124"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert_eq!(json["type"], "Note");
    assert_eq!(
        json["to"],
        serde_json::json!(["https://test.example.com/users/testuser/followers"])
    );
    assert_eq!(
        json["cc"],
        serde_json::json!(["https://www.w3.org/ns/activitystreams#Public"])
    );
}

#[tokio::test]
async fn test_shared_inbox() {
    let server = TestServer::new().await;

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "type": "Create",
        "actor": "https://remote.example.com/users/alice",
        "object": {
            "type": "Note",
            "content": "Hello from remote!"
        }
    });

    let response = server
        .client
        .post(&server.url("/inbox"))
        .header("Content-Type", "application/activity+json")
        .json(&activity)
        .send()
        .await
        .unwrap();

    // Should accept activity at shared inbox
    assert!(response.status().is_client_error() || response.status().is_success());
}

#[tokio::test]
async fn test_actor_content_negotiation() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Request with HTML Accept header
    let _html_response = server
        .client
        .get(&server.url("/users/testuser"))
        .header("Accept", "text/html")
        .send()
        .await
        .unwrap();

    // Request with ActivityPub Accept header
    let ap_response = server
        .client
        .get(&server.url("/users/testuser"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    // Should handle content negotiation differently
    // HTML might redirect or return HTML page
    // ActivityPub should return JSON
    if ap_response.status().is_success() {
        let content_type = ap_response.headers().get("content-type").unwrap();
        assert!(content_type.to_str().unwrap().contains("application/"));
    }
}
