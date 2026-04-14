mod common;

use axum::{
    Router, body::Bytes, extract::State, http::StatusCode as AxumStatusCode, routing::post,
};
use chrono::Utc;
use common::TestServer;
use reqwest::StatusCode;
use rustresort::data::{CachedProfile, EntityId, Follower, MediaAttachment};
use tokio::{
    net::TcpListener,
    sync::mpsc,
    time::{Duration, timeout},
};

#[derive(Debug)]
struct CapturedActivity {
    body: serde_json::Value,
}

#[derive(Clone)]
struct CaptureState {
    tx: mpsc::UnboundedSender<CapturedActivity>,
}

struct ActivityCaptureServer {
    addr: String,
    rx: mpsc::UnboundedReceiver<CapturedActivity>,
}

impl ActivityCaptureServer {
    async fn new() -> Self {
        async fn capture(State(state): State<CaptureState>, body: Bytes) -> AxumStatusCode {
            let captured = CapturedActivity {
                body: serde_json::from_slice(&body).expect("ActivityPub JSON body"),
            };
            let _ = state.tx.send(captured);
            AxumStatusCode::ACCEPTED
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/inbox", post(capture))
            .with_state(CaptureState { tx });

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self {
            addr: format!("http://{}", addr),
            rx,
        }
    }

    fn inbox_url(&self) -> String {
        format!("{}/inbox", self.addr)
    }

    async fn recv(&mut self) -> CapturedActivity {
        timeout(Duration::from_secs(5), self.rx.recv())
            .await
            .expect("timed out waiting for outbound activity")
            .expect("capture channel unexpectedly closed")
    }
}

async fn insert_remote_profile(
    server: &TestServer,
    address: &str,
    actor_uri: &str,
    inbox_uri: &str,
) {
    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: address.to_string(),
            uri: actor_uri.to_string(),
            display_name: Some("Remote User".to_string()),
            note: None,
            avatar_url: None,
            header_url: None,
            public_key_pem: common::test_public_key_pem().to_string(),
            inbox_uri: inbox_uri.to_string(),
            outbox_uri: Some(format!("{actor_uri}/outbox")),
            followers_count: None,
            following_count: None,
            fetched_at: Utc::now(),
        })
        .await;
}

#[tokio::test]
async fn test_private_status_delivers_create_to_remote_follower() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let mut capture = ActivityCaptureServer::new().await;

    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: "bob@followers.example".to_string(),
            actor_uri: Some("https://followers.example/users/bob".to_string()),
            inbox_uri: capture.inbox_url(),
            uri: "https://followers.example/follows/1".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .post(server.url("/api/v1/statuses"))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "status": "Private delivery",
            "visibility": "private"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let delivered = capture.recv().await;
    assert_eq!(delivered.body["type"], "Create");
    assert_eq!(
        delivered.body["to"],
        serde_json::json!([server.public_url("/users/testuser/followers")])
    );
}

#[tokio::test]
async fn test_direct_mention_delivers_create_with_explicit_recipient_and_tag() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let mut capture = ActivityCaptureServer::new().await;
    let remote_actor_uri = "https://remote.example/users/alice";
    insert_remote_profile(
        &server,
        "alice@remote.example",
        remote_actor_uri,
        &capture.inbox_url(),
    )
    .await;

    let response = server
        .client
        .post(server.url("/api/v1/statuses"))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "status": "@alice@remote.example hi there",
            "visibility": "direct"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let delivered = capture.recv().await;
    assert_eq!(delivered.body["type"], "Create");
    assert_eq!(delivered.body["to"], serde_json::json!([remote_actor_uri]));
    assert_eq!(
        delivered.body["object"]["to"],
        serde_json::json!([remote_actor_uri])
    );
    assert_eq!(
        delivered.body["object"]["tag"][0]["href"],
        serde_json::json!(remote_actor_uri)
    );
    assert_eq!(
        delivered.body["object"]["tag"][0]["name"],
        serde_json::json!("@alice@remote.example")
    );
}

#[tokio::test]
async fn test_status_edit_and_delete_deliver_update_and_delete_to_explicit_recipient() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let mut capture = ActivityCaptureServer::new().await;
    let remote_actor_uri = "https://remote.example/users/alice";
    insert_remote_profile(
        &server,
        "alice@remote.example",
        remote_actor_uri,
        &capture.inbox_url(),
    )
    .await;

    let create_response = server
        .client
        .post(server.url("/api/v1/statuses"))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "status": "@alice@remote.example original",
            "visibility": "direct"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);
    let created = create_response.json::<serde_json::Value>().await.unwrap();
    let status_id = created["id"].as_str().unwrap().to_string();
    let _ = capture.recv().await;

    let update_response = server
        .client
        .put(server.url(&format!("/api/v1/statuses/{status_id}")))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "status": "@alice@remote.example edited"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(update_response.status(), StatusCode::OK);

    let delivered_update = capture.recv().await;
    assert_eq!(delivered_update.body["type"], "Update");
    assert_eq!(
        delivered_update.body["object"]["content"],
        serde_json::json!("<p>@alice@remote.example edited</p>")
    );
    assert_eq!(
        delivered_update.body["object"]["to"],
        serde_json::json!([remote_actor_uri])
    );

    let delete_response = server
        .client
        .delete(server.url(&format!("/api/v1/statuses/{status_id}")))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), StatusCode::OK);

    let delivered_delete = capture.recv().await;
    assert_eq!(delivered_delete.body["type"], "Delete");
    assert_eq!(
        delivered_delete.body["to"],
        serde_json::json!([remote_actor_uri])
    );
    assert_eq!(
        delivered_delete.body["object"]["id"],
        serde_json::json!(server.public_url(&format!("/users/testuser/statuses/{status_id}")))
    );
}

#[tokio::test]
async fn test_create_with_media_delivers_activitypub_attachment_metadata() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let mut capture = ActivityCaptureServer::new().await;

    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: "bob@followers.example".to_string(),
            actor_uri: Some("https://followers.example/users/bob".to_string()),
            inbox_uri: capture.inbox_url(),
            uri: "https://followers.example/follows/media-1".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let media = MediaAttachment {
        id: EntityId::new_string(),
        status_id: None,
        s3_key: "attachments/test-image.webp".to_string(),
        thumbnail_s3_key: Some("attachments/test-image-thumb.webp".to_string()),
        content_type: "image/webp".to_string(),
        file_size: 1234,
        description: Some("cover image".to_string()),
        blurhash: Some("LEHV6nWB2yk8pyo0adR*.7kCMdnj".to_string()),
        width: Some(64),
        height: Some(32),
        focus_x: None,
        focus_y: None,
        created_at: Utc::now(),
    };
    server.state.db.insert_media(&media).await.unwrap();

    let response = server
        .client
        .post(server.url("/api/v1/statuses"))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "status": "Media delivery",
            "visibility": "public",
            "spoiler_text": "cw",
            "media_ids": [media.id]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let delivered = capture.recv().await;
    assert_eq!(delivered.body["type"], "Create");
    assert_eq!(delivered.body["object"]["summary"], "cw");
    assert_eq!(delivered.body["object"]["attachment"][0]["type"], "Image");
    assert_eq!(
        delivered.body["object"]["attachment"][0]["url"],
        "https://media.test.example.com/attachments/test-image.webp"
    );
    assert_eq!(
        delivered.body["object"]["attachment"][0]["icon"]["url"],
        "https://media.test.example.com/attachments/test-image-thumb.webp"
    );
    assert_eq!(
        delivered.body["object"]["attachment"][0]["name"],
        "cover image"
    );
}

#[tokio::test]
async fn test_account_update_delivers_actor_update_to_followers() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let mut capture = ActivityCaptureServer::new().await;

    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: "alice@remote.example".to_string(),
            actor_uri: Some("https://remote.example/users/alice".to_string()),
            inbox_uri: capture.inbox_url(),
            uri: "https://remote.example/follows/1".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .patch(server.url("/api/v1/accounts/update_credentials"))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "display_name": "Updated Name",
            "note": "Updated bio"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let delivered = capture.recv().await;
    assert_eq!(delivered.body["type"], "Update");
    assert_eq!(delivered.body["object"]["type"], "Person");
    assert_eq!(
        delivered.body["object"]["id"],
        serde_json::json!(server.public_url("/users/testuser"))
    );
    assert_eq!(
        delivered.body["object"]["name"],
        serde_json::json!("Updated Name")
    );
}
