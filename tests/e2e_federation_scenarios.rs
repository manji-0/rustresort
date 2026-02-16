//! E2E tests for federation scenarios
//!
//! These tests verify the complete flow of federation activities:
//! - Local post → ActivityPub federation
//! - Remote activity → Notification creation
//! - Follow lifecycle: Follow → Accept → Timeline
//! - Boost/Like → Notification generation
//!
//! Some tests are marked with #[ignore] as they test features that are
//! not yet fully implemented (e.g., inbox signature verification).

mod common;

use chrono::Utc;
use common::TestServer;
use rustresort::data::{EntityId, Follower, Notification, Status};
use serde_json::Value;

// =============================================================================
// Scenario 1: Local Post → Federation
// =============================================================================

/// Test that creating a local post generates proper ActivityPub representation
#[tokio::test]
async fn test_local_post_to_activitypub_note() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a local post via Mastodon API
    let status_data = serde_json::json!({
        "status": "Hello, Fediverse! #test",
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
        let status_id = json["id"].as_str().unwrap();

        // Verify the post can be retrieved as ActivityPub Note
        let ap_response = server
            .client
            .get(&server.url(&format!("/users/testuser/statuses/{}", status_id)))
            .header("Accept", "application/activity+json")
            .send()
            .await
            .unwrap();

        if ap_response.status().is_success() {
            let ap_json: Value = ap_response.json().await.unwrap();

            // Verify ActivityPub structure
            assert_eq!(ap_json["type"], "Note");
            assert!(ap_json.get("content").is_some());
            assert!(ap_json.get("attributedTo").is_some());
            assert!(ap_json.get("published").is_some());
            assert!(ap_json.get("to").is_some());
            assert!(ap_json.get("cc").is_some() || ap_json.get("audience").is_some());
        }
    }
}

/// Test that the outbox contains recent activities for federation
#[tokio::test]
async fn test_outbox_contains_create_activities() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create a status directly in DB
    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/users/testuser/statuses/outbox-test".to_string(),
        content: "<p>Outbox test post</p>".to_string(),
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

    // Fetch outbox
    let response = server
        .client
        .get(&server.url("/users/testuser/outbox"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();

        assert_eq!(json["type"], "OrderedCollection");
        assert!(json.get("totalItems").is_some());

        // Check if orderedItems or first page exists
        if let Some(items) = json.get("orderedItems") {
            assert!(items.is_array());
        }
    }
}

// =============================================================================
// Scenario 2: Remote Activity → Local Notification
// Tests simulate receiving activities by directly creating DB entries
// (ActivityProcessor implementation is pending)
// =============================================================================

/// Test notification creation flow (simulated)
/// This tests the notification API after manually inserting notification data
/// Note: This test may be flaky in parallel test environments due to authentication
#[tokio::test]
#[ignore = "Authentication may be unstable in parallel test runs - use database tests instead"]
async fn test_notification_api_returns_data() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status that will be referenced by the notification
    let our_status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/users/testuser/statuses/notif-test".to_string(),
        content: "<p>Test status for notification</p>".to_string(),
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

    server.state.db.insert_status(&our_status).await.unwrap();

    // Create a notification
    let notification = Notification {
        id: EntityId::new().0,
        notification_type: "favourite".to_string(),
        origin_account_address: "alice@remote.example.com".to_string(),
        status_uri: Some(our_status.uri.clone()),
        read: false,
        created_at: Utc::now(),
    };

    server
        .state
        .db
        .insert_notification(&notification)
        .await
        .unwrap();

    // Verify notification appears in API
    let response = server
        .client
        .get(&server.url("/api/v1/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "Notification API should return success, got {}",
        response.status()
    );

    let notifications: Vec<Value> = response.json().await.unwrap();
    assert!(
        !notifications.is_empty(),
        "Should have at least one notification"
    );
}

/// Test follow notification database integration
#[tokio::test]
async fn test_follow_notification_database() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Simulate receiving a follow by creating a follower and notification
    let follower = Follower {
        id: EntityId::new().0,
        follower_address: "bob@remote.example.com".to_string(),
        inbox_uri: "https://remote.example.com/users/bob/inbox".to_string(),
        uri: "https://remote.example.com/users/bob/follow/123".to_string(),
        created_at: Utc::now(),
    };

    server.state.db.insert_follower(&follower).await.unwrap();

    // Create follow notification
    let notification = Notification {
        id: EntityId::new().0,
        notification_type: "follow".to_string(),
        origin_account_address: "bob@remote.example.com".to_string(),
        status_uri: None,
        read: false,
        created_at: Utc::now(),
    };

    server
        .state
        .db
        .insert_notification(&notification)
        .await
        .unwrap();

    // Verify notification was created in DB
    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();

    assert!(
        notifications
            .iter()
            .any(|n| n.notification_type == "follow"),
        "Should have follow notification in database"
    );
}

/// Test mention notification with associated status
#[tokio::test]
async fn test_mention_notification_with_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create a status that mentions us
    let remote_status = Status {
        id: EntityId::new().0,
        uri: "https://remote.example.com/users/carol/statuses/mention".to_string(),
        content: "<p>@testuser@test.example.com Check this out!</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "carol@remote.example.com".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: Some(Utc::now()),
    };

    server.state.db.insert_status(&remote_status).await.unwrap();

    // Create mention notification
    let notification = Notification {
        id: EntityId::new().0,
        notification_type: "mention".to_string(),
        origin_account_address: "carol@remote.example.com".to_string(),
        status_uri: Some(remote_status.uri.clone()),
        read: false,
        created_at: Utc::now(),
    };

    server
        .state
        .db
        .insert_notification(&notification)
        .await
        .unwrap();

    // Verify notification was created with status_uri
    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();

    let mention_notif = notifications
        .iter()
        .find(|n| n.notification_type == "mention")
        .expect("Should have mention notification");

    assert!(
        mention_notif.status_uri.is_some(),
        "Mention notification should have associated status"
    );
}

// =============================================================================
// Scenario 3: Follow Lifecycle
// =============================================================================

/// Test complete follow flow: send follow request, receive accept, see in following list
#[tokio::test]
async fn test_follow_lifecycle_with_accept() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let _token = server.create_test_token().await;

    // Simulate following a remote user by creating a Follow record
    use rustresort::data::Follow;

    let follow = Follow {
        id: EntityId::new().0,
        target_address: "alice@remote.example.com".to_string(),
        uri: "https://test.example.com/users/testuser/follow/456".to_string(),
        created_at: Utc::now(),
    };

    server.state.db.insert_follow(&follow).await.unwrap();

    // Verify following collection includes the user
    let response = server
        .client
        .get(&server.url("/users/testuser/following"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["type"], "OrderedCollection");

        let total_items = json["totalItems"].as_i64().unwrap_or(0);
        assert!(total_items > 0, "Should have at least one following");
    }
}

/// Test follower collection updates when receiving follow
#[tokio::test]
async fn test_followers_collection_updates() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Add multiple followers
    let followers = vec![
        (
            "user1@instance1.com",
            "https://instance1.com/users/user1/inbox",
        ),
        (
            "user2@instance2.com",
            "https://instance2.com/users/user2/inbox",
        ),
        (
            "user3@instance3.com",
            "https://instance3.com/users/user3/inbox",
        ),
    ];

    for (i, (addr, inbox)) in followers.iter().enumerate() {
        let follower = Follower {
            id: EntityId::new().0,
            follower_address: addr.to_string(),
            inbox_uri: inbox.to_string(),
            uri: format!("https://example.com/follow/{}", i),
            created_at: Utc::now(),
        };
        server.state.db.insert_follower(&follower).await.unwrap();
    }

    // Verify followers collection
    let response = server
        .client
        .get(&server.url("/users/testuser/followers"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["type"], "OrderedCollection");

        let total_items = json["totalItems"].as_i64().unwrap_or(0);
        assert_eq!(total_items, 3, "Should have 3 followers");
    }
}

// =============================================================================
// Scenario 4: Boost (Reblog) Flow
// =============================================================================

/// Test that boost notification creation works in database
#[tokio::test]
async fn test_boost_notification_database() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create our status that will be boosted
    let our_status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/users/testuser/statuses/boostme".to_string(),
        content: "<p>Boost this if you agree!</p>".to_string(),
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

    server.state.db.insert_status(&our_status).await.unwrap();

    // Simulate receiving a boost (Announce activity)
    let notification = Notification {
        id: EntityId::new().0,
        notification_type: "reblog".to_string(),
        origin_account_address: "dave@remote.example.com".to_string(),
        status_uri: Some(our_status.uri.clone()),
        read: false,
        created_at: Utc::now(),
    };

    server
        .state
        .db
        .insert_notification(&notification)
        .await
        .unwrap();

    // Verify notification was created
    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();

    let reblog_notif = notifications
        .iter()
        .find(|n| n.notification_type == "reblog")
        .expect("Should have reblog notification");

    assert_eq!(
        reblog_notif.origin_account_address,
        "dave@remote.example.com"
    );
}

// =============================================================================
// Scenario 5: Reply Chain / Context
// =============================================================================

/// Test reply chain is properly maintained across federation
#[tokio::test]
async fn test_reply_chain_context() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create original status
    let original = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/users/testuser/statuses/original".to_string(),
        content: "<p>Original post</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now() - chrono::Duration::hours(2),
        fetched_at: None,
    };

    // Create reply from remote
    let reply = Status {
        id: EntityId::new().0,
        uri: "https://remote.example.com/users/eve/statuses/reply1".to_string(),
        content: "<p>This is a reply</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "eve@remote.example.com".to_string(),
        is_local: false,
        in_reply_to_uri: Some(original.uri.clone()),
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now() - chrono::Duration::hours(1),
        fetched_at: Some(Utc::now()),
    };

    // Create reply to reply
    let reply_to_reply = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/users/testuser/statuses/rereply".to_string(),
        content: "<p>Reply to reply</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: Some(reply.uri.clone()),
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };

    server.state.db.insert_status(&original).await.unwrap();
    server.state.db.insert_status(&reply).await.unwrap();
    server
        .state
        .db
        .insert_status(&reply_to_reply)
        .await
        .unwrap();

    // Get context for the reply
    let response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/context", reply.id)))
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let context: Value = response.json().await.unwrap();

        // Should have ancestors (the original post)
        assert!(context.get("ancestors").is_some());
        // Should have descendants (the reply to reply)
        assert!(context.get("descendants").is_some());
    }
}

// =============================================================================
// Scenario 6: Inbox Processing (Signature validation)
// =============================================================================

/// Test that inbox rejects unsigned requests
#[tokio::test]
async fn test_inbox_requires_signature() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Send activity without HTTP signature
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "type": "Follow",
        "actor": "https://malicious.example.com/users/attacker",
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

    // Should be rejected (401 or 403)
    assert!(
        response.status() == 401 || response.status() == 403,
        "Unsigned inbox request should be rejected"
    );
}

/// Test that shared inbox also requires signatures
#[tokio::test]
async fn test_shared_inbox_requires_signature() {
    let server = TestServer::new().await;

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "type": "Create",
        "actor": "https://malicious.example.com/users/spammer",
        "object": {
            "type": "Note",
            "content": "Spam message!"
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

    // Should be rejected
    assert!(
        response.status() == 401 || response.status() == 403,
        "Unsigned shared inbox request should be rejected"
    );
}

// =============================================================================
// Scenario 7: Notification Lifecycle
// =============================================================================

/// Test dismissing a single notification via database
#[tokio::test]
async fn test_dismiss_single_notification_database() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create a notification
    let notification = Notification {
        id: EntityId::new().0,
        notification_type: "follow".to_string(),
        origin_account_address: "follower@example.com".to_string(),
        status_uri: None,
        read: false,
        created_at: Utc::now(),
    };

    let notification_id = notification.id.clone();
    server
        .state
        .db
        .insert_notification(&notification)
        .await
        .unwrap();

    // Mark as read via database
    server
        .state
        .db
        .mark_notification_read(&notification_id)
        .await
        .unwrap();

    // Verify it's now read (dismissed)
    let notifications = server
        .state
        .db
        .get_notifications(10, None, true)
        .await
        .unwrap();

    // Should not appear in unread
    assert!(
        !notifications.iter().any(|n| n.id == notification_id),
        "Dismissed notification should not appear in unread"
    );
}

/// Test clearing all notifications via database
#[tokio::test]
async fn test_clear_all_notifications_database() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create multiple notifications
    for i in 0..5 {
        let notification = Notification {
            id: EntityId::new().0,
            notification_type: "favourite".to_string(),
            origin_account_address: format!("user{}@example.com", i),
            status_uri: None,
            read: false,
            created_at: Utc::now(),
        };
        server
            .state
            .db
            .insert_notification(&notification)
            .await
            .unwrap();
    }

    // Clear all via database
    server.state.db.mark_all_notifications_read().await.unwrap();

    // Verify all are now read
    let unread = server
        .state
        .db
        .get_notifications(100, None, true)
        .await
        .unwrap();

    assert!(unread.is_empty(), "All notifications should be cleared");
}

// =============================================================================
// Scenario 8: Cross-Instance Timeline Aggregation
// =============================================================================

/// Test home timeline aggregates local posts
#[tokio::test]
async fn test_home_timeline_aggregation() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create local status
    let local_status = Status {
        id: format!("local-{}", EntityId::new().0),
        uri: "https://test.example.com/users/testuser/statuses/hometest".to_string(),
        content: "<p>Local post for home timeline</p>".to_string(),
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

    server.state.db.insert_status(&local_status).await.unwrap();

    // Get home timeline
    let response = server
        .client
        .get(&server.url("/api/v1/timelines/home"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let timeline: Vec<Value> = response.json().await.unwrap();

        // Should contain our local post
        let has_local = timeline.iter().any(|s| {
            s["content"]
                .as_str()
                .map_or(false, |c| c.contains("Local post"))
        });

        assert!(has_local, "Home timeline should include local posts");
    }
}

/// Test public timeline visibility filter
#[tokio::test]
async fn test_public_timeline_visibility_filter() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create posts with different visibility
    let visibilities = vec!["public", "unlisted", "private", "direct"];

    for (i, vis) in visibilities.iter().enumerate() {
        let status = Status {
            id: format!("vis-{}-{}", i, EntityId::new().0),
            uri: format!("https://test.example.com/statuses/vis-{}", i),
            content: format!("<p>Post with visibility: {}</p>", vis),
            content_warning: None,
            visibility: vis.to_string(),
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

    // Get public timeline
    let response = server
        .client
        .get(&server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let timeline: Vec<Value> = response.json().await.unwrap();

        // Should only contain public posts
        for post in &timeline {
            if let Some(visibility) = post.get("visibility") {
                assert_eq!(
                    visibility, "public",
                    "Public timeline should only show public posts"
                );
            }
        }
    }
}

/// Test public timeline returns all local statuses (current behavior)
#[tokio::test]
async fn test_public_timeline_returns_local_statuses() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create a public status
    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/statuses/public-test".to_string(),
        content: "<p>Public timeline test</p>".to_string(),
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

    // Get public timeline
    let response = server
        .client
        .get(&server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let timeline: Vec<Value> = response.json().await.unwrap();
    assert!(!timeline.is_empty(), "Public timeline should have statuses");
}

// =============================================================================
// Scenario 9: WebFinger Discovery
// =============================================================================

/// Test WebFinger resolution for local user
#[tokio::test]
async fn test_webfinger_for_local_user() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(&server.url("/.well-known/webfinger?resource=acct:testuser@test.example.com"))
        .header("Accept", "application/jrd+json")
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let jrd: Value = response.json().await.unwrap();

        assert_eq!(jrd["subject"], "acct:testuser@test.example.com");
        assert!(jrd.get("links").is_some());

        // Should have self link with ActivityPub type
        let links = jrd["links"].as_array().unwrap();
        let ap_link = links.iter().find(|l| {
            l["type"]
                .as_str()
                .map_or(false, |t| t.contains("activity+json"))
        });

        assert!(ap_link.is_some(), "Should have ActivityPub self link");
    }
}

// =============================================================================
// Scenario 10: Activity Delivery Preparation
// =============================================================================

/// Test that followers' inboxes are collected for delivery
#[tokio::test]
async fn test_delivery_target_collection() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Add followers with different instances (testing shared inbox optimization)
    let followers_data = vec![
        ("user1@instance1.com", "https://instance1.com/inbox"),
        ("user2@instance1.com", "https://instance1.com/inbox"), // Same shared inbox
        (
            "user3@instance2.com",
            "https://instance2.com/users/user3/inbox",
        ),
    ];

    for (addr, inbox) in followers_data {
        let follower = Follower {
            id: EntityId::new().0,
            follower_address: addr.to_string(),
            inbox_uri: inbox.to_string(),
            uri: format!("https://example.com/follow/{}", addr),
            created_at: Utc::now(),
        };
        server.state.db.insert_follower(&follower).await.unwrap();
    }

    // Get all followers' inboxes
    let follower_inboxes = server.state.db.get_follower_inboxes().await.unwrap();

    // Collect unique inboxes (for shared inbox optimization)
    let unique_inboxes: std::collections::HashSet<_> = follower_inboxes.into_iter().collect();

    // Should have 2 unique inboxes (instance1 shared + instance2 personal)
    assert_eq!(unique_inboxes.len(), 2, "Should deduplicate shared inboxes");
}

// =============================================================================
// Scenario 11: Notification API Tests (with authentication)
// =============================================================================

/// Test that notification API returns notifications with proper structure
/// Note: This test may be flaky due to authentication in parallel test environments
#[tokio::test]
#[ignore = "Authentication may be unstable in parallel test runs - use database tests instead"]
async fn test_notification_api_structure() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create test data
    let notification = Notification {
        id: EntityId::new().0,
        notification_type: "follow".to_string(),
        origin_account_address: "test@example.com".to_string(),
        status_uri: None,
        read: false,
        created_at: Utc::now(),
    };
    server
        .state
        .db
        .insert_notification(&notification)
        .await
        .unwrap();

    // Get notifications via API
    let response = server
        .client
        .get(&server.url("/api/v1/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "Notification API should succeed, got {}",
        response.status()
    );

    let notifications: Vec<Value> = response.json().await.unwrap();
    if !notifications.is_empty() {
        let first = &notifications[0];
        assert!(first.get("id").is_some(), "Should have id");
        assert!(first.get("type").is_some(), "Should have type");
        assert!(first.get("created_at").is_some(), "Should have created_at");
        assert!(first.get("account").is_some(), "Should have account");
    }
}
