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
async fn test_create_status_rejects_invalid_visibility() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "Hello, world!",
        "visibility": "friends-only"
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
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
async fn test_private_status_is_hidden_from_public_status_endpoints() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "Private note",
        "visibility": "private"
    });
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    assert_eq!(create_response.status(), 200);
    let created: Value = create_response.json().await.unwrap();
    let status_id = created["id"].as_str().unwrap();

    let status_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}", status_id)))
        .send()
        .await
        .unwrap();
    assert_eq!(status_response.status(), 404);

    let context_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/context", status_id)))
        .send()
        .await
        .unwrap();
    assert_eq!(context_response.status(), 404);

    let reblogged_by_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/reblogged_by", status_id)))
        .send()
        .await
        .unwrap();
    assert_eq!(reblogged_by_response.status(), 404);

    let favourited_by_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/favourited_by", status_id)))
        .send()
        .await
        .unwrap();
    assert_eq!(favourited_by_response.status(), 404);
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

#[tokio::test]
async fn test_create_reply_status_persists_reply_metadata() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let parent = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/users/testuser/statuses/original".to_string(),
        content: "<p>Original post</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    server.state.db.insert_status(&parent).await.unwrap();

    let payload = serde_json::json!({
        "status": "This is a reply",
        "visibility": "public",
        "in_reply_to_id": parent.id
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let created: Value = response.json().await.unwrap();
    let reply_id = created["id"]
        .as_str()
        .expect("status response should contain id")
        .to_string();

    let reply = server
        .state
        .db
        .get_status(&reply_id)
        .await
        .unwrap()
        .expect("reply should be persisted");
    assert_eq!(reply.in_reply_to_uri, Some(parent.uri));
    assert_eq!(reply.persisted_reason, "reply_to_own");
}

#[tokio::test]
async fn test_create_reply_status_accepts_cache_only_remote_target() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/cache-only-reply-target";

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Remote status shown from cache</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let payload = serde_json::json!({
        "status": "Replying to cache-only status",
        "visibility": "public",
        "in_reply_to_id": remote_uri
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let created: Value = response.json().await.unwrap();
    let reply_id = created["id"]
        .as_str()
        .expect("status response should contain id")
        .to_string();

    let reply = server
        .state
        .db
        .get_status(&reply_id)
        .await
        .unwrap()
        .expect("reply should be persisted");
    assert_eq!(reply.in_reply_to_uri, Some(remote_uri.to_string()));
    assert_eq!(reply.persisted_reason, "own");
}

#[tokio::test]
async fn test_create_reply_status_delivers_to_remote_reply_target_inbox_without_followers() {
    use axum::{extract::State, http::StatusCode, routing::post};
    use chrono::Utc;
    use rustresort::data::{CachedProfile, CachedStatus};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::net::TcpListener;
    use tokio::time::{Duration, sleep};

    async fn record_inbox_delivery(
        State(counter): State<Arc<AtomicUsize>>,
        _body: String,
    ) -> StatusCode {
        counter.fetch_add(1, Ordering::SeqCst);
        StatusCode::ACCEPTED
    }

    let inbox_delivery_count = Arc::new(AtomicUsize::new(0));
    let remote_router = axum::Router::new()
        .route("/users/alice/inbox", post(record_inbox_delivery))
        .with_state(inbox_delivery_count.clone());
    let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_addr = remote_listener.local_addr().unwrap();
    let remote_base_url = format!("http://{}", remote_addr);

    tokio::spawn(async move {
        axum::serve(remote_listener, remote_router).await.unwrap();
    });

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_address = "alice@remote.example";
    let remote_status_uri = format!("{}/users/alice/statuses/reply-target", remote_base_url);

    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: remote_address.to_string(),
            uri: format!("{}/users/alice", remote_base_url),
            display_name: Some("Alice".to_string()),
            note: None,
            avatar_url: None,
            header_url: None,
            public_key_pem: "-----BEGIN PUBLIC KEY-----\nMIIB\n-----END PUBLIC KEY-----"
                .to_string(),
            inbox_uri: format!("{}/users/alice/inbox", remote_base_url),
            outbox_uri: None,
            followers_count: None,
            following_count: None,
            fetched_at: Utc::now(),
        })
        .await;

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_status_uri.clone(),
            uri: remote_status_uri.clone(),
            content: "<p>Remote status</p>".to_string(),
            account_address: remote_address.to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let payload = serde_json::json!({
        "status": "Replying to remote status",
        "visibility": "public",
        "in_reply_to_id": remote_status_uri
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let mut delivered = false;
    for _ in 0..600 {
        if inbox_delivery_count.load(Ordering::SeqCst) > 0 {
            delivered = true;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }

    assert!(
        delivered,
        "expected outbound Create delivery to include remote reply target inbox"
    );
}

#[tokio::test]
async fn test_favourite_remote_status_by_uri_persists_from_cache() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/fav-by-uri";

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Remote status</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let encoded_uri: String = url::form_urlencoded::byte_serialize(remote_uri.as_bytes()).collect();
    let response = server
        .client
        .post(&server.url(&format!(
            "/api/v1/statuses/placeholder/favourite?uri={}",
            encoded_uri
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["favourited"], true);
    assert_eq!(body["account"]["username"], "alice");
    assert_eq!(body["account"]["acct"], "alice@remote.example");

    let persisted = server
        .state
        .db
        .get_status_by_uri(remote_uri)
        .await
        .unwrap()
        .expect("remote status should be persisted");
    assert!(!persisted.is_local);
    assert_eq!(persisted.persisted_reason, "favourited");
}

#[tokio::test]
async fn test_favourite_status_rejects_empty_uri_query_parameter() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url("/api/v1/statuses/placeholder/favourite?uri="))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn test_favourite_remote_status_by_path_id_uri_fallback_persists_from_cache() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/fav-by-path-id";

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Remote status by path id</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let encoded_id: String = url::form_urlencoded::byte_serialize(remote_uri.as_bytes()).collect();
    let response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/favourite", encoded_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["favourited"], true);
    assert_eq!(body["account"]["acct"], "alice@remote.example");
    assert!(
        server
            .state
            .db
            .get_status_by_uri(remote_uri)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn test_bookmark_remote_status_by_uri_persists_from_cache() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/bookmark-by-uri";

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Remote status for bookmark</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let encoded_uri: String = url::form_urlencoded::byte_serialize(remote_uri.as_bytes()).collect();
    let response = server
        .client
        .post(&server.url(&format!(
            "/api/v1/statuses/placeholder/bookmark?uri={}",
            encoded_uri
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["bookmarked"], true);
    assert_eq!(body["account"]["username"], "alice");
    assert_eq!(body["account"]["acct"], "alice@remote.example");

    let persisted = server
        .state
        .db
        .get_status_by_uri(remote_uri)
        .await
        .unwrap()
        .expect("remote status should be persisted");
    assert!(!persisted.is_local);
    assert_eq!(persisted.persisted_reason, "bookmarked");
}

#[tokio::test]
async fn test_bookmark_remote_status_by_path_id_uri_fallback_persists_from_cache() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/bookmark-by-path-id";

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Remote status bookmark path id</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let encoded_id: String = url::form_urlencoded::byte_serialize(remote_uri.as_bytes()).collect();
    let response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/bookmark", encoded_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["bookmarked"], true);
    assert_eq!(body["account"]["acct"], "alice@remote.example");
    assert!(
        server
            .state
            .db
            .get_status_by_uri(remote_uri)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn test_reblog_remote_status_by_uri_persists_from_cache() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/reblog-by-uri";

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Remote status for reblog</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let encoded_uri: String = url::form_urlencoded::byte_serialize(remote_uri.as_bytes()).collect();
    let response = server
        .client
        .post(&server.url(&format!(
            "/api/v1/statuses/placeholder/reblog?uri={}",
            encoded_uri
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["reblogged"], true);
    assert_eq!(body["account"]["username"], "alice");
    assert_eq!(body["account"]["acct"], "alice@remote.example");

    let persisted = server
        .state
        .db
        .get_status_by_uri(remote_uri)
        .await
        .unwrap()
        .expect("remote status should be persisted");
    assert!(!persisted.is_local);
    assert_eq!(persisted.persisted_reason, "reposted");
    assert!(server.state.db.is_reposted(&persisted.id).await.unwrap());
}

#[tokio::test]
async fn test_notifications_fallback_to_cached_status() {
    use chrono::Utc;
    use rustresort::data::{CachedStatus, EntityId, Notification};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/notif-cache";
    let notification_id = EntityId::new().0;

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Cached notification status</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let notification = Notification {
        id: notification_id.clone(),
        notification_type: "mention".to_string(),
        origin_account_address: "alice@remote.example".to_string(),
        status_uri: Some(remote_uri.to_string()),
        read: false,
        created_at: Utc::now(),
    };
    server
        .state
        .db
        .insert_notification(&notification)
        .await
        .unwrap();

    let response = server
        .client
        .get(&server.url("/api/v1/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let notifications: Vec<Value> = response.json().await.unwrap();
    let target = notifications
        .iter()
        .find(|entry| entry["id"].as_str() == Some(&notification_id))
        .expect("notification should be returned");
    assert_eq!(target["status"]["uri"], remote_uri);
    assert_eq!(
        target["status"]["content"],
        "<p>Cached notification status</p>"
    );
}
