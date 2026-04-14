//! E2E tests for Mastodon streaming endpoints.

mod common;

use common::TestServer;
use rustresort::data::{EntityId, Follow};
use serde_json::Value;
use tokio::time::{Duration, timeout};

const REMOTE_ACTOR_ID: &str = "https://remote.example/users/alice";
const REMOTE_ACTOR_ADDRESS: &str = "alice@remote.example";

fn register_default_remote_key(server: &TestServer) -> String {
    let key_id = format!("{REMOTE_ACTOR_ID}#main-key");
    server.register_inbound_public_key(&key_id, common::test_public_key_pem());
    key_id
}

async fn read_sse_event(mut response: reqwest::Response) -> (String, String) {
    let mut buffer = String::new();

    let body = timeout(Duration::from_secs(5), async {
        loop {
            let chunk = response
                .chunk()
                .await
                .expect("stream chunk should be readable")
                .expect("stream closed before event");
            buffer.push_str(std::str::from_utf8(chunk.as_ref()).expect("SSE stream must be utf-8"));
            if let Some(frame_end) = buffer.find("\n\n") {
                return buffer[..frame_end].to_string();
            }
        }
    })
    .await
    .expect("timed out waiting for SSE event");

    let mut event_name = String::new();
    let mut data = String::new();
    for line in body.lines() {
        if let Some(value) = line.strip_prefix("event: ") {
            event_name = value.to_string();
        } else if let Some(value) = line.strip_prefix("data: ") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value);
        }
    }

    (event_name, data)
}

async fn expect_no_sse_event(mut response: reqwest::Response, duration: Duration) {
    let result = timeout(duration, async move {
        let mut buffer = String::new();
        loop {
            let chunk = response
                .chunk()
                .await
                .expect("stream chunk should be readable")
                .expect("stream closed before timeout");
            buffer.push_str(std::str::from_utf8(chunk.as_ref()).expect("SSE stream must be utf-8"));
            if buffer.contains("\n\n") {
                return;
            }
        }
    })
    .await;

    assert!(result.is_err(), "unexpected SSE event was received");
}

#[tokio::test]
async fn test_user_stream_receives_remote_followee_public_status_updates() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/stream-home".to_string(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/streaming/user"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/home-stream-1";
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/home-stream-1",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>hello from followee #stream</p>",
            "published": "2026-01-01T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let send_future = server.post_signed_activity("/inbox", &activity, &key_id);
    let read_future = read_sse_event(response);
    let (send_response, (event_name, data)) = tokio::join!(send_future, read_future);

    assert_eq!(send_response.status(), reqwest::StatusCode::OK);
    assert_eq!(event_name, "update");

    let json: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(json["uri"], status_uri);
    assert_eq!(json["account"]["acct"], REMOTE_ACTOR_ADDRESS);
}

#[tokio::test]
async fn test_public_stream_receives_remote_public_status_and_delete_events() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/stream-public".to_string(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let create_response = server
        .client
        .get(server.url("/api/v1/streaming/public"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/public-stream-1";
    let create_activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/public-stream-1",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>public federated streaming event</p>",
            "published": "2026-01-01T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let send_create = server.post_signed_activity("/inbox", &create_activity, &key_id);
    let read_create = read_sse_event(create_response);
    let (send_create_response, (create_event_name, create_data)) =
        tokio::join!(send_create, read_create);

    assert_eq!(send_create_response.status(), reqwest::StatusCode::OK);
    assert_eq!(create_event_name, "update");
    let create_json: Value = serde_json::from_str(&create_data).unwrap();
    assert_eq!(create_json["uri"], status_uri);

    let delete_response = server
        .client
        .get(server.url("/api/v1/streaming/public"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), 200);

    let delete_activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/public-stream-1/delete",
        "type": "Delete",
        "actor": REMOTE_ACTOR_ID,
        "object": status_uri
    });

    let send_delete = server.post_signed_activity("/inbox", &delete_activity, &key_id);
    let read_delete = read_sse_event(delete_response);
    let (send_delete_response, (delete_event_name, delete_data)) =
        tokio::join!(send_delete, read_delete);

    assert_eq!(send_delete_response.status(), reqwest::StatusCode::OK);
    assert_eq!(delete_event_name, "delete");
    assert_eq!(delete_data, status_uri);
}

#[tokio::test]
async fn test_user_stream_receives_remote_follow_notification_events() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/streaming/user"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/stream-follow-1",
        "type": "Follow",
        "actor": {
            "id": REMOTE_ACTOR_ID,
            "inbox": "https://remote.example/users/alice/inbox"
        },
        "object": server.public_url("/users/testuser")
    });

    let send_future = server.post_signed_activity("/users/testuser/inbox", &activity, &key_id);
    let read_future = read_sse_event(response);
    let (send_response, (event_name, data)) = tokio::join!(send_future, read_future);

    assert_eq!(send_response.status(), reqwest::StatusCode::OK);
    assert_eq!(event_name, "notification");

    let json: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(json["type"], "follow");
    assert_eq!(json["account"]["acct"], REMOTE_ACTOR_ADDRESS);
    assert!(json["status"].is_null());
}

#[tokio::test]
async fn test_root_stream_public_dispatches_to_public_stream() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/root-stream-public".to_string(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/streaming"))
        .query(&[("stream", "public")])
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/root-stream-public-1";
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/root-stream-public-1",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>public federated root stream event</p>",
            "published": "2026-01-01T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let send_future = server.post_signed_activity("/inbox", &activity, &key_id);
    let read_future = read_sse_event(response);
    let (send_response, (event_name, data)) = tokio::join!(send_future, read_future);

    assert_eq!(send_response.status(), reqwest::StatusCode::OK);
    assert_eq!(event_name, "update");
    let json: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(json["uri"], status_uri);
}

#[tokio::test]
async fn test_hashtag_stream_receives_remote_public_status_updates() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/stream-hashtag".to_string(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/streaming/hashtag?tag=StreamTag"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/hashtag-stream-1";
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/hashtag-stream-1",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>hello #StreamTag from followee</p>",
            "published": "2026-01-01T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let send_future = server.post_signed_activity("/inbox", &activity, &key_id);
    let read_future = read_sse_event(response);
    let (send_response, (event_name, data)) = tokio::join!(send_future, read_future);

    assert_eq!(send_response.status(), reqwest::StatusCode::OK);
    assert_eq!(event_name, "update");
    let json: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(json["uri"], status_uri);
}

#[tokio::test]
async fn test_list_stream_receives_remote_followed_account_updates() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/stream-list".to_string(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let list_id = server
        .state
        .db
        .create_list("Streaming list", "list")
        .await
        .unwrap();
    server
        .state
        .db
        .add_accounts_to_list(&list_id, &[REMOTE_ACTOR_ADDRESS.to_string()])
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url(&format!("/api/v1/streaming/list?list={}", list_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/list-stream-1";
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/list-stream-1",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>hello list stream</p>",
            "published": "2026-01-01T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let send_future = server.post_signed_activity("/inbox", &activity, &key_id);
    let read_future = read_sse_event(response);
    let (send_response, (event_name, data)) = tokio::join!(send_future, read_future);

    assert_eq!(send_response.status(), reqwest::StatusCode::OK);
    assert_eq!(event_name, "update");
    let json: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(json["uri"], status_uri);
    assert_eq!(json["account"]["acct"], REMOTE_ACTOR_ADDRESS);
}

#[tokio::test]
async fn test_list_stream_ignores_remote_direct_updates() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/stream-list-direct".to_string(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let list_id = server
        .state
        .db
        .create_list("Streaming list direct exclusion", "list")
        .await
        .unwrap();
    server
        .state
        .db
        .add_accounts_to_list(&list_id, &[REMOTE_ACTOR_ADDRESS.to_string()])
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url(&format!("/api/v1/streaming/list?list={}", list_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/list-stream-direct-1",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": "https://remote.example/users/alice/statuses/list-stream-direct-1",
            "content": "<p>direct list leak</p>",
            "published": "2026-01-01T00:00:00Z",
            "to": [server.public_url("/users/testuser")]
        }
    });

    let send_response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(send_response.status(), reqwest::StatusCode::OK);
    expect_no_sse_event(response, Duration::from_millis(750)).await;
}

#[tokio::test]
async fn test_public_stream_receives_embedded_announce_quote_update() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let quoted_local_status = rustresort::data::Status {
        id: "local-quoted-stream-target".to_string(),
        uri: server.public_url("/users/testuser/statuses/local-quoted-stream-target"),
        content: "<p>Quoted target</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Own,
        created_at: chrono::Utc::now(),
        fetched_at: None,
    };
    server
        .state
        .db
        .insert_status(&quoted_local_status)
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/streaming/public"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/announce-quote-stream-1";
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/announce-quote-stream-1",
        "type": "Announce",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "id": status_uri,
            "attributedTo": REMOTE_ACTOR_ID,
            "content": "<p>quoted remote note</p>",
            "published": "2026-01-01T00:00:00Z",
            "quoteUri": quoted_local_status.uri,
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let send_future = server.post_signed_activity("/inbox", &activity, &key_id);
    let read_future = read_sse_event(response);
    let (send_response, (event_name, data)) = tokio::join!(send_future, read_future);

    assert_eq!(send_response.status(), reqwest::StatusCode::OK);
    assert_eq!(event_name, "update");
    let json: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(json["uri"], status_uri);
}

#[tokio::test]
async fn test_direct_stream_receives_remote_direct_note_updates() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/streaming/direct"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/direct-stream-1";
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/direct-stream-1",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>direct hello</p>",
            "published": "2026-01-01T00:00:00Z",
            "to": [server.public_url("/users/testuser")]
        }
    });

    let send_future = server.post_signed_activity("/inbox", &activity, &key_id);
    let read_future = read_sse_event(response);
    let (send_response, (event_name, data)) = tokio::join!(send_future, read_future);

    assert_eq!(send_response.status(), reqwest::StatusCode::OK);
    assert_eq!(event_name, "update");
    let json: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(json["uri"], status_uri);
    assert_eq!(json["visibility"], "direct");
}

#[tokio::test]
async fn test_public_local_stream_ignores_remote_public_updates() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/stream-public-local".to_string(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/streaming/public/local"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/public-local-stream-1",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": "https://remote.example/users/alice/statuses/public-local-stream-1",
            "content": "<p>remote public should not reach local-only stream</p>",
            "published": "2026-01-01T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let send_response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(send_response.status(), reqwest::StatusCode::OK);
    expect_no_sse_event(response, Duration::from_millis(750)).await;
}

#[tokio::test]
async fn test_user_stream_notification_embeds_status_interactions() {
    use chrono::Utc;
    use rustresort::data::{PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let local_status = Status {
        id: EntityId::new_string(),
        uri: server.public_url("/users/testuser/statuses/stream-notif-status-1"),
        content: "<p>local status for notification stream</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    server.state.db.insert_status(&local_status).await.unwrap();
    server
        .state
        .db
        .insert_bookmark(&local_status.id)
        .await
        .unwrap();
    server
        .state
        .db
        .insert_status_pin(&local_status.id)
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/streaming/user"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/stream-like-1",
        "type": "Like",
        "actor": REMOTE_ACTOR_ID,
        "object": local_status.uri
    });

    let send_future = server.post_signed_activity("/inbox", &activity, &key_id);
    let read_future = read_sse_event(response);
    let (send_response, (event_name, data)) = tokio::join!(send_future, read_future);

    assert_eq!(send_response.status(), reqwest::StatusCode::OK);
    assert_eq!(event_name, "notification");
    let json: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(json["type"], "favourite");
    assert_eq!(json["status"]["id"], local_status.id);
    assert_eq!(json["status"]["bookmarked"], true);
    assert_eq!(json["status"]["pinned"], true);
}

#[tokio::test]
async fn test_user_stream_receives_update_notification_for_reblogged_remote_status() {
    use chrono::Utc;
    use rustresort::data::{PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let remote_status = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/users/alice/statuses/stream-update-notif-1".to_string(),
        content: "<p>before</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: REMOTE_ACTOR_ADDRESS.to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Mentioned,
        created_at: Utc::now(),
        fetched_at: Some(Utc::now()),
    };
    server.state.db.insert_status(&remote_status).await.unwrap();

    let reblog_response = server
        .client
        .post(server.url(&format!("/api/v1/statuses/{}/reblog", remote_status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(reblog_response.status(), reqwest::StatusCode::OK);

    let response = server
        .client
        .get(server.url("/api/v1/streaming/user"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/stream-update-notif-1",
        "type": "Update",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": remote_status.uri,
            "content": "<p>after</p>",
            "summary": "cw",
            "published": "2026-01-03T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let send_future = server.post_signed_activity("/inbox", &activity, &key_id);
    let read_future = read_sse_event(response);
    let (send_response, (event_name, data)) = tokio::join!(send_future, read_future);

    assert_eq!(send_response.status(), reqwest::StatusCode::OK);
    assert_eq!(event_name, "notification");
    let json: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(json["type"], "update");
    assert_eq!(json["account"]["acct"], REMOTE_ACTOR_ADDRESS);
    assert_eq!(json["status"]["uri"], remote_status.uri);
    assert_eq!(json["status"]["reblogged"], true);
}

#[tokio::test]
async fn test_user_stream_receives_quoted_update_notification_with_local_quote_status() {
    use chrono::Utc;
    use rustresort::data::{PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let remote_status = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/users/alice/statuses/stream-quoted-update-1".to_string(),
        content: "<p>before</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: REMOTE_ACTOR_ADDRESS.to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Mentioned,
        created_at: Utc::now(),
        fetched_at: Some(Utc::now()),
    };
    server.state.db.insert_status(&remote_status).await.unwrap();

    let local_quote = Status {
        id: EntityId::new_string(),
        uri: server.public_url("/users/testuser/statuses/stream-local-quote-1"),
        content: "<p>local quote</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: Some(remote_status.uri.clone()),
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    server.state.db.insert_status(&local_quote).await.unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/streaming/user"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let key_id = register_default_remote_key(&server);
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/stream-quoted-update-1",
        "type": "Update",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": remote_status.uri,
            "content": "<p>after</p>",
            "published": "2026-01-03T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let send_future = server.post_signed_activity("/inbox", &activity, &key_id);
    let read_future = read_sse_event(response);
    let (send_response, (event_name, data)) = tokio::join!(send_future, read_future);

    assert_eq!(send_response.status(), reqwest::StatusCode::OK);
    assert_eq!(event_name, "notification");
    let json: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(json["type"], "quoted_update");
    assert_eq!(json["account"]["acct"], REMOTE_ACTOR_ADDRESS);
    assert_eq!(json["status"]["uri"], local_quote.uri);
}
