//! E2E tests for ActivityPub federation endpoints

mod common;

use common::TestServer;
use serde_json::Value;

const REMOTE_ACTOR_ID: &str = "https://remote.example/users/alice";
const REMOTE_ACTOR_ADDRESS: &str = "alice@remote.example";
const EVIL_ACTOR_ID: &str = "https://evil.example/users/mallory";
const EVIL_ACTOR_ADDRESS: &str = "mallory@evil.example";

fn register_default_remote_key(server: &TestServer) -> String {
    let key_id = format!("{REMOTE_ACTOR_ID}#main-key");
    server.register_inbound_public_key(&key_id, common::test_public_key_pem());
    key_id
}

fn register_evil_remote_key(server: &TestServer) -> String {
    let key_id = format!("{EVIL_ACTOR_ID}#main-key");
    server.register_inbound_public_key(&key_id, common::test_public_key_pem());
    key_id
}

async fn insert_local_status(server: &TestServer, slug: &str) -> String {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let uri = server.public_url(&format!("/users/testuser/statuses/{slug}"));
    server
        .state
        .db
        .insert_status(&Status {
            id: EntityId::new_string(),
            uri: uri.clone(),
            content: format!("<p>local {slug}</p>"),
            content_warning: None,
            visibility: StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "testuser@test.example.com".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        })
        .await
        .unwrap();

    uri
}

#[tokio::test]
async fn test_actor_endpoint() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(server.url("/users/testuser"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    // Should return ActivityPub actor
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/activity+json")
    );
    let json: Value = response.json().await.unwrap();
    assert_eq!(json["type"], "Person");
    assert!(json.get("inbox").is_some());
    assert!(json.get("outbox").is_some());
    assert!(json.get("publicKey").is_some());
    assert_eq!(
        json["featured"],
        "https://test.example.com/users/testuser/collections/featured"
    );
    assert_eq!(
        json["featuredTags"],
        "https://test.example.com/users/testuser/collections/tags"
    );
    assert_eq!(json["discoverable"], true);
    assert_eq!(json["indexable"], true);
    assert_eq!(
        json["endpoints"]["sharedInbox"],
        "https://test.example.com/inbox"
    );
}

#[tokio::test]
async fn test_actor_endpoint_exposes_account_migration_fields() {
    use chrono::Utc;

    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    server
        .state
        .db
        .patch_account_migration(
            &account.id,
            Some(Some("https://old.example/users/testuser")),
            Some(Some("https://new.example/users/testuser")),
            Utc::now(),
        )
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/users/testuser"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert_eq!(
        json["alsoKnownAs"],
        serde_json::json!(["https://old.example/users/testuser"])
    );
    assert_eq!(json["movedTo"], "https://new.example/users/testuser");
}

#[tokio::test]
async fn test_inbox_endpoint_rejects_unsigned_activity() {
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
        .post(server.url("/users/testuser/inbox"))
        .header("Content-Type", "application/activity+json")
        .json(&activity)
        .send()
        .await
        .unwrap();

    assert!(
        response.status() == 401 || response.status() == 403,
        "Unsigned inbox request should be rejected"
    );
}

#[tokio::test]
async fn test_inbox_rejects_signature_key_id_actor_mismatch() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "type": "Follow",
        "actor": "https://remote.example.com/users/alice",
        "object": "https://test.example.com/users/testuser"
    });

    let response = server
        .client
        .post(server.url("/users/testuser/inbox"))
        .header("Content-Type", "application/activity+json")
        .header(
            "Signature",
            "keyId=\"https://remote.example.com/users/bob#main-key\",algorithm=\"rsa-sha256\",headers=\"(request-target) host date\",signature=\"Zm9v\"",
        )
        .json(&activity)
        .send()
        .await
        .unwrap();

    assert!(
        response.status() == 401 || response.status() == 403,
        "Inbox request must be rejected when keyId actor and activity actor differ"
    );
}

#[tokio::test]
async fn test_outbox_endpoint() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(server.url("/users/testuser/outbox"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    // Should return ActivityPub OrderedCollection
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/activity+json")
    );
    let json: Value = response.json().await.unwrap();
    assert_eq!(json["type"], "OrderedCollection");
    assert!(json.get("totalItems").is_some());
    assert_eq!(
        json["first"],
        "https://test.example.com/users/testuser/outbox?page=true"
    );
}

#[tokio::test]
async fn test_outbox_excludes_private_and_direct_statuses() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    server.create_test_account().await;

    for visibility in ["public", "unlisted", "private", "direct"] {
        let parsed_visibility = match visibility {
            "public" => rustresort::data::StatusVisibility::Public,
            "unlisted" => rustresort::data::StatusVisibility::Unlisted,
            "private" => rustresort::data::StatusVisibility::Private,
            "direct" => rustresort::data::StatusVisibility::Direct,
            _ => unreachable!("test visibility fixture should be valid"),
        };

        let status = Status {
            id: EntityId::new_string(),
            uri: format!(
                "https://test.example.com/users/testuser/statuses/outbox-{}",
                visibility
            ),
            content: format!("<p>{}</p>", visibility),
            content_warning: None,
            visibility: parsed_visibility,
            language: Some("en".to_string()),
            account_address: "testuser@test.example.com".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: rustresort::data::PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };
        server.state.db.insert_status(&status).await.unwrap();
    }

    let response = server
        .client
        .get(server.url("/users/testuser/outbox"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();

    let ordered_items = json["orderedItems"].as_array().unwrap();
    let object_ids: Vec<String> = ordered_items
        .iter()
        .filter_map(|item| item["object"]["id"].as_str().map(ToString::to_string))
        .collect();

    assert!(
        object_ids
            .iter()
            .any(|id| id.ends_with("/statuses/outbox-public"))
    );
    assert!(
        object_ids
            .iter()
            .any(|id| id.ends_with("/statuses/outbox-unlisted"))
    );
    assert!(
        !object_ids
            .iter()
            .any(|id| id.ends_with("/statuses/outbox-private"))
    );
    assert!(
        !object_ids
            .iter()
            .any(|id| id.ends_with("/statuses/outbox-direct"))
    );
}

#[tokio::test]
async fn test_outbox_pagination_exposes_next_page_without_duplicates() {
    use chrono::{Duration, Utc};
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;

    for index in 0..25 {
        let slug = format!("paged-{index:02}");
        let uri = server.public_url(&format!("/users/testuser/statuses/{slug}"));
        server
            .state
            .db
            .insert_status(&Status {
                id: EntityId::new_string(),
                uri,
                content: format!("<p>{slug}</p>"),
                content_warning: None,
                visibility: StatusVisibility::Public,
                language: Some("en".to_string()),
                account_address: "testuser@test.example.com".to_string(),
                is_local: true,
                in_reply_to_uri: None,
                boost_of_uri: None,
                quote_of_uri: None,
                persisted_reason: PersistedReason::Own,
                created_at: Utc::now() - Duration::seconds(index as i64),
                fetched_at: None,
            })
            .await
            .unwrap();
    }

    let first = server
        .client
        .get(server.url("/users/testuser/outbox?page=true"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let first_json: Value = first.json().await.unwrap();
    let first_items = first_json["orderedItems"].as_array().unwrap();
    assert_eq!(first_items.len(), 20);
    let next = first_json["next"].as_str().expect("next page URL");
    assert!(next.ends_with("offset=20"));

    let second = server
        .client
        .get(server.url("/users/testuser/outbox?page=true&offset=20"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 200);
    let second_json: Value = second.json().await.unwrap();
    let second_items = second_json["orderedItems"].as_array().unwrap();
    assert_eq!(second_items.len(), 5);
    assert!(second_json["next"].is_null());

    let first_ids = first_items
        .iter()
        .filter_map(|item| item["object"]["id"].as_str())
        .collect::<std::collections::HashSet<_>>();
    let second_ids = second_items
        .iter()
        .filter_map(|item| item["object"]["id"].as_str())
        .collect::<std::collections::HashSet<_>>();
    assert!(first_ids.is_disjoint(&second_ids));
}

#[tokio::test]
async fn test_followers_collection() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(server.url("/users/testuser/followers"))
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
        .get(server.url("/users/testuser/following"))
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
async fn test_featured_collection_returns_pinned_statuses() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let pinned_uri = insert_local_status(&server, "featured-pinned").await;
    let unpinned_uri = insert_local_status(&server, "featured-unpinned").await;

    let pinned_status = server
        .state
        .db
        .get_status_by_uri(&pinned_uri)
        .await
        .unwrap()
        .unwrap();
    server
        .state
        .db
        .insert_status_pin(&pinned_status.id)
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/users/testuser/collections/featured"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert_eq!(json["type"], "OrderedCollection");
    let ordered_items = json["orderedItems"].as_array().unwrap();
    assert!(ordered_items.iter().any(|item| item["id"] == pinned_uri));
    assert!(!ordered_items.iter().any(|item| item["id"] == unpinned_uri));
}

#[tokio::test]
async fn test_featured_tags_collection_is_empty_ordered_collection() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(server.url("/users/testuser/collections/tags"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert_eq!(json["type"], "OrderedCollection");
    assert_eq!(json["totalItems"], 0);
    assert_eq!(json["orderedItems"], serde_json::json!([]));
}

#[tokio::test]
async fn test_tag_collection_resolves_both_tagged_and_tags_routes() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;

    let status_uri = server.public_url("/users/testuser/statuses/hashtag-collection");
    server
        .state
        .db
        .insert_status(&Status {
            id: EntityId::new_string(),
            uri: status_uri.clone(),
            content: "<p>#breakfast timeline item</p>".to_string(),
            content_warning: None,
            visibility: StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "testuser@test.example.com".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        })
        .await
        .unwrap();

    for path in ["/tagged/breakfast", "/tags/breakfast"] {
        let response = server
            .client
            .get(server.url(path))
            .header("Accept", "application/activity+json")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200, "{path} should resolve");
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["type"], "OrderedCollection");
        let ordered_items = json["orderedItems"].as_array().unwrap();
        assert!(
            ordered_items
                .iter()
                .any(|item| item == &serde_json::json!(status_uri))
        );
    }
}

#[tokio::test]
async fn test_followers_collection_prefers_stored_actor_uri() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follower};

    let server = TestServer::new().await;
    server.create_test_account().await;

    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: "alice@remote.example".to_string(),
            actor_uri: Some("https://remote.example/@alice".to_string()),
            inbox_uri: "https://remote.example/inbox".to_string(),
            uri: "https://remote.example/follows/1".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/users/testuser/followers"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let ordered_items = json["orderedItems"]
        .as_array()
        .expect("followers should expose orderedItems");
    assert_eq!(ordered_items.len(), 1);
    assert_eq!(ordered_items[0], "https://remote.example/@alice");
}

#[tokio::test]
async fn test_following_collection_prefers_stored_actor_uri() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: "alice@remote.example".to_string(),
            actor_uri: Some("https://remote.example/users/alice".to_string()),
            uri: "https://test.example.com/users/testuser/follow/1".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/users/testuser/following"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let ordered_items = json["orderedItems"]
        .as_array()
        .expect("following should expose orderedItems");
    assert_eq!(ordered_items.len(), 1);
    assert_eq!(ordered_items[0], "https://remote.example/users/alice");
}

#[tokio::test]
async fn test_status_as_activity() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create a status
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/users/testuser/statuses/123".to_string(),
        content: "<p>ActivityPub test</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };

    server.state.db.insert_status(&status).await.unwrap();

    let response = server
        .client
        .get(server.url("/users/testuser/statuses/123"))
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
async fn test_status_activity_endpoint_returns_create_activity() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let status_uri = insert_local_status(&server, "create-activity").await;

    let response = server
        .client
        .get(server.url("/users/testuser/statuses/create-activity/activity"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/activity+json")
    );
    let json: Value = response.json().await.unwrap();
    assert_eq!(json["type"], "Create");
    assert!(json.get("@context").is_some());
    assert_eq!(json["object"]["id"], status_uri);
}

#[tokio::test]
async fn test_status_note_and_create_include_quote_fields() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let quote_target_uri = "https://remote.example/users/alice/statuses/quoted-note";

    server
        .state
        .db
        .insert_status(&Status {
            id: EntityId::new_string(),
            uri: "https://test.example.com/users/testuser/statuses/quoted-local".to_string(),
            content: "<p>Quoted local note</p>".to_string(),
            content_warning: None,
            visibility: StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "testuser@test.example.com".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: Some(quote_target_uri.to_string()),
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        })
        .await
        .unwrap();

    let note_response = server
        .client
        .get(server.url("/users/testuser/statuses/quoted-local"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();
    assert_eq!(note_response.status(), 200);
    let note_json: Value = note_response.json().await.unwrap();
    assert_eq!(note_json["quoteUri"], quote_target_uri);
    assert_eq!(note_json["quoteUrl"], quote_target_uri);

    let activity_response = server
        .client
        .get(server.url("/users/testuser/statuses/quoted-local/activity"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();
    assert_eq!(activity_response.status(), 200);
    let activity_json: Value = activity_response.json().await.unwrap();
    assert_eq!(activity_json["type"], "Create");
    assert!(activity_json.get("@context").is_some());
    assert_eq!(activity_json["object"]["quoteUri"], quote_target_uri);
    assert_eq!(activity_json["object"]["quoteUrl"], quote_target_uri);
}

#[tokio::test]
async fn test_outbox_page_and_status_activity_include_local_announce() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;

    let remote_status = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/users/alice/statuses/announce-target".to_string(),
        content: "<p>Remote target</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: REMOTE_ACTOR_ADDRESS.to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Reposted,
        created_at: Utc::now(),
        fetched_at: Some(Utc::now()),
    };
    server.state.db.insert_status(&remote_status).await.unwrap();

    let announce_activity_uri =
        "https://test.example.com/users/testuser/statuses/local-announce-1/activity";
    server
        .state
        .db
        .insert_repost(&remote_status.id, announce_activity_uri)
        .await
        .unwrap();

    let outbox_response = server
        .client
        .get(server.url("/users/testuser/outbox?page=true"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();
    assert_eq!(outbox_response.status(), 200);
    let outbox_json: Value = outbox_response.json().await.unwrap();
    assert_eq!(outbox_json["type"], "OrderedCollectionPage");
    let ordered_items = outbox_json["orderedItems"]
        .as_array()
        .expect("outbox page should contain orderedItems");
    assert!(
        ordered_items
            .iter()
            .any(|item| { item["type"] == "Announce" && item["object"] == remote_status.uri })
    );

    let announce_response = server
        .client
        .get(server.url("/users/testuser/statuses/local-announce-1/activity"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();
    assert_eq!(announce_response.status(), 200);
    let announce_json: Value = announce_response.json().await.unwrap();
    assert_eq!(announce_json["type"], "Announce");
    assert!(announce_json.get("@context").is_some());
    assert_eq!(announce_json["object"], remote_status.uri);
}

#[tokio::test]
async fn test_unlisted_status_activity_audience() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/users/testuser/statuses/124".to_string(),
        content: "<p>Unlisted ActivityPub test</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Unlisted,
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };

    server.state.db.insert_status(&status).await.unwrap();

    let response = server
        .client
        .get(server.url("/users/testuser/statuses/124"))
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
async fn test_shared_inbox_rejects_unsigned_activity() {
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
        .post(server.url("/inbox"))
        .header("Content-Type", "application/activity+json")
        .json(&activity)
        .send()
        .await
        .unwrap();

    assert!(
        response.status() == 401 || response.status() == 403,
        "Unsigned shared inbox request should be rejected"
    );
}

#[tokio::test]
async fn test_shared_inbox_rejects_signature_key_id_actor_mismatch() {
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
        .post(server.url("/inbox"))
        .header("Content-Type", "application/activity+json")
        .header(
            "Signature",
            "keyId=\"https://remote.example.com/users/bob#main-key\",algorithm=\"rsa-sha256\",headers=\"(request-target) host date\",signature=\"Zm9v\"",
        )
        .json(&activity)
        .send()
        .await
        .unwrap();

    assert!(
        response.status() == 401 || response.status() == 403,
        "Shared inbox request must be rejected when keyId actor and activity actor differ"
    );
}

#[tokio::test]
async fn test_signed_personal_inbox_follow_persists_and_delivers_accept() {
    use axum::{extract::State, http::StatusCode, routing::post};
    use std::sync::Arc;
    use tokio::{
        net::TcpListener,
        sync::Mutex,
        time::{Duration, sleep},
    };

    async fn record_accept(
        State(received): State<Arc<Mutex<Vec<Value>>>>,
        body: String,
    ) -> StatusCode {
        if let Ok(activity) = serde_json::from_str::<Value>(&body) {
            received.lock().await.push(activity);
        }
        StatusCode::ACCEPTED
    }

    let received = Arc::new(Mutex::new(Vec::new()));
    let remote_router = axum::Router::new()
        .route("/users/alice/inbox", post(record_accept))
        .with_state(received.clone());
    let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_addr = remote_listener.local_addr().unwrap();
    let remote_base_url = format!("http://localtest.me:{}", remote_addr.port());
    tokio::spawn(async move {
        axum::serve(remote_listener, remote_router).await.unwrap();
    });

    let server = TestServer::new().await;
    server.create_test_account().await;

    let actor_id = "https://remote.example/users/alice";
    let key_id = format!("{actor_id}#main-key");
    server.register_inbound_public_key(&key_id, common::test_public_key_pem());

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/follows/1",
        "type": "Follow",
        "actor": {
            "id": actor_id,
            "inbox": format!("{remote_base_url}/users/alice/inbox")
        },
        "object": "https://test.example.com/users/testuser"
    });

    let response = server
        .post_signed_activity("/users/testuser/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let followers = server.state.db.get_all_followers().await.unwrap();
    assert_eq!(followers.len(), 1);
    assert_eq!(followers[0].follower_address, "alice@remote.example");

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert_eq!(notifications.len(), 1);
    assert_eq!(
        notifications[0].notification_type,
        rustresort::data::NotificationType::Follow
    );

    let mut delivered = None;
    for _ in 0..200 {
        {
            let events = received.lock().await;
            if let Some(first) = events.first() {
                delivered = Some(first.clone());
                break;
            }
        }
        sleep(Duration::from_millis(10)).await;
    }

    let delivered = delivered.expect("expected Accept delivery to remote inbox");
    assert_eq!(delivered["type"], "Accept");
    assert_eq!(delivered["object"]["type"], "Follow");
    assert_eq!(
        delivered["object"]["id"],
        "https://remote.example/follows/1"
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_move_retargets_follow_and_queues_new_follow() {
    use axum::{extract::State, http::StatusCode, routing::post};
    use rustresort::data::{EntityId, Follow};
    use std::sync::Arc;
    use tokio::{
        net::TcpListener,
        sync::Mutex,
        time::{Duration, sleep},
    };

    async fn record_follow(
        State(received): State<Arc<Mutex<Vec<Value>>>>,
        body: String,
    ) -> StatusCode {
        if let Ok(activity) = serde_json::from_str::<Value>(&body) {
            received.lock().await.push(activity);
        }
        StatusCode::ACCEPTED
    }

    let received = Arc::new(Mutex::new(Vec::new()));
    let remote_router = axum::Router::new()
        .route("/users/alice-new/inbox", post(record_follow))
        .with_state(received.clone());
    let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_addr = remote_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(remote_listener, remote_router).await.unwrap();
    });

    let server = TestServer::new().await;
    server.create_test_account().await;

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/original".to_string(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let key_id = register_default_remote_key(&server);
    let new_actor_uri = format!("http://{remote_addr}/users/alice-new");
    let new_inbox_uri = format!("{new_actor_uri}/inbox");
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/move-1",
        "type": "Move",
        "actor": REMOTE_ACTOR_ID,
        "object": REMOTE_ACTOR_ID,
        "target": {
            "id": new_actor_uri,
            "inbox": new_inbox_uri,
            "alsoKnownAs": [REMOTE_ACTOR_ID]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let mut delivered = None;
    for _ in 0..200 {
        {
            let events = received.lock().await;
            if let Some(first) = events.first() {
                delivered = Some(first.clone());
                break;
            }
        }
        sleep(Duration::from_millis(10)).await;
    }

    let delivered = delivered.expect("expected Follow delivery to moved account inbox");
    assert_eq!(delivered["type"], "Follow");
    assert_eq!(delivered["object"], activity["target"]["id"]);

    let old_follow = server
        .state
        .db
        .get_follow(REMOTE_ACTOR_ADDRESS, None)
        .await
        .unwrap();
    assert!(
        old_follow.is_none(),
        "old follow should be removed after Move"
    );

    let new_target_address = format!("alice-new@127.0.0.1:{}", remote_addr.port());
    let new_follow = server
        .state
        .db
        .get_follow(&new_target_address, None)
        .await
        .unwrap()
        .expect("new follow should be inserted after Move");
    assert_eq!(
        new_follow.actor_uri.as_deref(),
        activity["target"]["id"].as_str()
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_create_persists_mention_and_exposes_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let actor_id = "https://remote.example/users/alice";
    let key_id = format!("{actor_id}#main-key");
    server.register_inbound_public_key(&key_id, common::test_public_key_pem());
    let status_uri = "https://remote.example/users/alice/statuses/mention-http";

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/create-mention-http",
        "type": "Create",
        "actor": {
            "id": actor_id,
            "inbox": "https://remote.example/inbox"
        },
        "object": {
            "type": "Note",
            "attributedTo": actor_id,
            "id": status_uri,
            "content": "<p>Hello @testuser</p>",
            "published": "2026-01-01T00:00:00Z",
            "to": "https://test.example.com/users/testuser/"
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let persisted = server.state.db.get_status_by_uri(status_uri).await.unwrap();
    assert!(persisted.is_some(), "mentioned status should be persisted");

    let notifications_response = server
        .client
        .get(server.url("/api/v1/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(notifications_response.status(), reqwest::StatusCode::OK);
    let notifications: Vec<Value> = notifications_response.json().await.unwrap();

    let mention = notifications
        .iter()
        .find(|value| value["type"] == "mention")
        .expect("expected mention notification");
    assert_eq!(mention["status"]["uri"], status_uri);
    assert_eq!(mention["account"]["acct"], "alice@remote.example");
}

#[tokio::test]
async fn test_signed_shared_inbox_delete_removes_persisted_remote_status() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status};

    let server = TestServer::new().await;
    server.create_test_account().await;

    let actor_id = "https://remote.example/users/alice";
    let key_id = format!("{actor_id}#main-key");
    server.register_inbound_public_key(&key_id, common::test_public_key_pem());
    let status_uri = "https://remote.example/users/alice/statuses/delete-me";

    server
        .state
        .db
        .insert_status(&Status {
            id: EntityId::new_string(),
            uri: status_uri.to_string(),
            content: "<p>Delete me</p>".to_string(),
            content_warning: None,
            visibility: rustresort::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "alice@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Mentioned,
            created_at: Utc::now(),
            fetched_at: Some(Utc::now()),
        })
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/delete-1",
        "type": "Delete",
        "actor": actor_id,
        "object": status_uri
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        server
            .state
            .db
            .get_status_by_uri(status_uri)
            .await
            .unwrap()
            .is_none(),
        "Delete should remove persisted remote status"
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_create_reply_to_local_persists_status_and_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let local_status_uri = insert_local_status(&server, "reply-target").await;
    let remote_status_uri = "https://remote.example/users/alice/statuses/reply-http";

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/create-reply-http",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": remote_status_uri,
            "content": "<p>Replying to you</p>",
            "published": "2026-01-01T00:00:00Z",
            "inReplyTo": local_status_uri,
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let persisted = server
        .state
        .db
        .get_status_by_uri(remote_status_uri)
        .await
        .unwrap()
        .expect("reply should be persisted");
    assert_eq!(
        persisted.in_reply_to_uri.as_deref(),
        Some(local_status_uri.as_str())
    );

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert!(
        notifications.iter().any(|notification| {
            notification.notification_type == rustresort::data::NotificationType::Mention
                && notification.status_uri.as_deref() == Some(remote_status_uri)
        }),
        "reply to a local status should create a mention notification"
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_create_from_followee_persists_and_caches_for_timelines() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/followee-cached";

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: server.public_url("/users/testuser/follows/remote-alice"),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/create-followee-cache",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>post from a followed remote actor</p>",
            "published": "2026-01-02T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let persisted = server
        .state
        .db
        .get_status_by_uri(status_uri)
        .await
        .unwrap()
        .expect("followee posts should be persisted for restart-safe timelines");
    assert_eq!(
        persisted.persisted_reason,
        rustresort::data::PersistedReason::Timeline
    );
    assert!(
        server
            .state
            .timeline_cache
            .get_by_uri(status_uri)
            .await
            .is_some(),
        "followee posts should still be cached for timeline use"
    );
    assert!(
        server
            .state
            .db
            .get_notifications(10, None, false)
            .await
            .unwrap()
            .is_empty(),
        "cache-only Create should not create notifications"
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_create_question_from_followee_persists_poll_for_api() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};
    use url::form_urlencoded::byte_serialize;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/followee-question";

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: server.public_url("/users/testuser/follows/remote-alice-question"),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/create-followee-question",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Question",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>tea or coffee?</p>",
            "published": "2026-01-02T00:00:00Z",
            "endTime": "2026-01-12T00:00:00Z",
            "oneOf": [
                { "name": "tea", "replies": { "totalItems": 2 } },
                { "name": "coffee", "replies": { "totalItems": 1 } }
            ],
            "votersCount": 3,
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let persisted = server
        .state
        .db
        .get_status_by_uri(status_uri)
        .await
        .unwrap()
        .expect("remote question should be persisted");
    let encoded_id: String = byte_serialize(persisted.id.as_bytes()).collect();
    let status_response = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{}", encoded_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(status_response.status(), reqwest::StatusCode::OK);
    let body = status_response.json::<Value>().await.unwrap();
    assert_eq!(body["poll"]["multiple"], false);
    assert_eq!(body["poll"]["voters_count"], 3);
    assert_eq!(body["poll"]["options"][0]["title"], "tea");
    assert_eq!(body["poll"]["options"][0]["votes_count"], 2);
}

#[tokio::test]
async fn test_activitypub_status_object_for_local_poll_includes_question_and_mention_tag() {
    use chrono::Utc;
    use rustresort::data::CachedProfile;

    let server = TestServer::new().await;
    server.create_test_account().await;

    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: REMOTE_ACTOR_ADDRESS.to_string(),
            uri: REMOTE_ACTOR_ID.to_string(),
            display_name: Some("Alice".to_string()),
            note: None,
            avatar_url: None,
            header_url: None,
            public_key_pem: common::test_public_key_pem().to_string(),
            inbox_uri: "https://remote.example/users/alice/inbox".to_string(),
            outbox_uri: Some("https://remote.example/users/alice/outbox".to_string()),
            followers_count: None,
            following_count: None,
            fetched_at: Utc::now(),
        })
        .await;

    let status_id = "local-poll-activitypub";
    let status_uri = server.public_url(&format!("/users/testuser/statuses/{status_id}"));
    server
        .state
        .db
        .insert_status(&rustresort::data::Status {
            id: status_id.to_string(),
            uri: status_uri.clone(),
            content:
                "<p>@alice@remote.example @testuser@test.example.com tea or coffee? #breakfast</p>"
                    .to_string(),
            content_warning: None,
            visibility: rustresort::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "testuser@test.example.com".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: rustresort::data::PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        })
        .await
        .unwrap();
    server
        .state
        .db
        .create_poll(
            status_id,
            &["tea".to_string(), "coffee".to_string()],
            600,
            false,
        )
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url(&format!("/users/testuser/statuses/{status_id}")))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response.json::<Value>().await.unwrap();
    assert_eq!(body["type"], "Question");
    assert_eq!(body["oneOf"][0]["name"], "tea");
    assert!(
        body["tag"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tag| tag["type"] == "Mention" && tag["href"] == REMOTE_ACTOR_ID)
    );
    assert!(
        body["tag"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tag| tag["type"] == "Mention"
                && tag["href"] == server.public_url("/users/testuser"))
    );
    assert!(
        body["tag"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tag| tag["type"] == "Hashtag" && tag["name"] == "#breakfast")
    );

    let activity_response = server
        .client
        .get(server.url(&format!("/users/testuser/statuses/{status_id}/activity")))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();
    assert_eq!(activity_response.status(), reqwest::StatusCode::OK);
    let activity_body = activity_response.json::<Value>().await.unwrap();
    assert_eq!(activity_body["type"], "Create");
    assert_eq!(activity_body["object"]["type"], "Question");
}

#[tokio::test]
async fn test_signed_shared_inbox_poll_vote_updates_local_poll() {
    use chrono::Utc;
    use rustresort::data::Status;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);

    let status_id = "local-poll-vote";
    let status_uri = server.public_url(&format!("/users/testuser/statuses/{status_id}"));
    server
        .state
        .db
        .insert_status(&Status {
            id: status_id.to_string(),
            uri: status_uri.clone(),
            content: "<p>Tea or coffee?</p>".to_string(),
            content_warning: None,
            visibility: rustresort::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: rustresort::data::PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        })
        .await
        .unwrap();
    let poll_id = server
        .state
        .db
        .create_poll(
            status_id,
            &["tea".to_string(), "coffee".to_string()],
            600,
            false,
        )
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/users/alice#votes/1/activity",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "id": "https://remote.example/users/alice#votes/1",
            "type": "Note",
            "name": "tea",
            "attributedTo": REMOTE_ACTOR_ID,
            "to": [server.public_url("/users/testuser")],
            "inReplyTo": status_uri
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let poll = server.state.db.get_poll(&poll_id).await.unwrap().unwrap();
    assert_eq!(poll.4, 1);
    assert_eq!(poll.5, 1);
    let options = server.state.db.get_poll_options(&poll_id).await.unwrap();
    assert_eq!(options[0].2, 1);
    assert_eq!(options[1].2, 0);
}

#[tokio::test]
async fn test_signed_shared_inbox_private_poll_vote_requires_follower() {
    use chrono::Utc;
    use rustresort::data::Status;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);

    let status_id = "private-local-poll-vote";
    let status_uri = server.public_url(&format!("/users/testuser/statuses/{status_id}"));
    server
        .state
        .db
        .insert_status(&Status {
            id: status_id.to_string(),
            uri: status_uri.clone(),
            content: "<p>Private tea or coffee?</p>".to_string(),
            content_warning: None,
            visibility: rustresort::data::StatusVisibility::Private,
            language: Some("en".to_string()),
            account_address: "".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: rustresort::data::PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        })
        .await
        .unwrap();
    let poll_id = server
        .state
        .db
        .create_poll(
            status_id,
            &["tea".to_string(), "coffee".to_string()],
            600,
            false,
        )
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/users/alice#votes/private-1/activity",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "id": "https://remote.example/users/alice#votes/private-1",
            "type": "Note",
            "name": "tea",
            "attributedTo": REMOTE_ACTOR_ID,
            "inReplyTo": status_uri
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

    let poll = server.state.db.get_poll(&poll_id).await.unwrap().unwrap();
    assert_eq!(poll.4, 0);
    assert_eq!(poll.5, 0);
}

#[tokio::test]
async fn test_signed_shared_inbox_direct_poll_vote_requires_mentioned_actor() {
    use chrono::Utc;
    use rustresort::data::Status;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);

    let status_id = "direct-local-poll-vote";
    let status_uri = server.public_url(&format!("/users/testuser/statuses/{status_id}"));
    server
        .state
        .db
        .insert_status(&Status {
            id: status_id.to_string(),
            uri: status_uri.clone(),
            content: "<p>@bob@remote.example tea or coffee?</p>".to_string(),
            content_warning: None,
            visibility: rustresort::data::StatusVisibility::Direct,
            language: Some("en".to_string()),
            account_address: "".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: rustresort::data::PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        })
        .await
        .unwrap();
    let poll_id = server
        .state
        .db
        .create_poll(
            status_id,
            &["tea".to_string(), "coffee".to_string()],
            600,
            false,
        )
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/users/alice#votes/direct-1/activity",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "id": "https://remote.example/users/alice#votes/direct-1",
            "type": "Note",
            "name": "tea",
            "attributedTo": REMOTE_ACTOR_ID,
            "inReplyTo": status_uri
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

    let poll = server.state.db.get_poll(&poll_id).await.unwrap().unwrap();
    assert_eq!(poll.4, 0);
    assert_eq!(poll.5, 0);
}

#[tokio::test]
async fn test_signed_shared_inbox_update_note_persists_edit_snapshot() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/edit-http";

    server
        .state
        .db
        .insert_status(&Status {
            id: EntityId::new_string(),
            uri: status_uri.to_string(),
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
        })
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/update-note-http",
        "type": "Update",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>after</p>",
            "summary": "cw",
            "published": "2026-01-03T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let updated = server
        .state
        .db
        .get_status_by_uri(status_uri)
        .await
        .unwrap()
        .expect("updated status should exist");
    assert_eq!(updated.content, "<p>after</p>");
    assert_eq!(updated.content_warning.as_deref(), Some("cw"));

    let edits = server
        .state
        .db
        .get_status_edits(&updated.id, 10)
        .await
        .unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].1, "<p>before</p>");
}

#[tokio::test]
async fn test_signed_shared_inbox_update_note_creates_notification_for_reblogged_status() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/edit-notify-http";
    let status_id = EntityId::new_string();

    server
        .state
        .db
        .insert_status(&Status {
            id: status_id.clone(),
            uri: status_uri.to_string(),
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
        })
        .await
        .unwrap();

    let reblog_response = server
        .client
        .post(server.url(&format!("/api/v1/statuses/{status_id}/reblog")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(reblog_response.status(), reqwest::StatusCode::OK);

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/update-note-notify-http",
        "type": "Update",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>after</p>",
            "summary": "cw",
            "published": "2026-01-03T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert!(
        notifications.iter().any(|notification| {
            notification.notification_type == rustresort::data::NotificationType::Update
                && notification.origin_account_address == REMOTE_ACTOR_ADDRESS
                && notification.status_uri.as_deref() == Some(status_uri)
        }),
        "edited reblogged remote status should create an update notification"
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_update_note_without_local_interaction_does_not_notify() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/edit-no-notify-http";

    server
        .state
        .db
        .insert_status(&Status {
            id: EntityId::new_string(),
            uri: status_uri.to_string(),
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
        })
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/update-note-no-notify-http",
        "type": "Update",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>after</p>",
            "published": "2026-01-03T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert!(
        notifications
            .iter()
            .all(|notification| notification.notification_type
                != rustresort::data::NotificationType::Update),
        "edited remote status without local interaction should not create update notifications"
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_update_profile_refreshes_profile_cache() {
    use chrono::Utc;
    use rustresort::data::CachedProfile;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);

    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: REMOTE_ACTOR_ADDRESS.to_string(),
            uri: REMOTE_ACTOR_ID.to_string(),
            display_name: Some("Alice".to_string()),
            note: Some("before".to_string()),
            avatar_url: None,
            header_url: None,
            public_key_pem: "old-key".to_string(),
            inbox_uri: "https://remote.example/inbox-old".to_string(),
            outbox_uri: Some("https://remote.example/outbox-old".to_string()),
            followers_count: Some(1),
            following_count: Some(2),
            fetched_at: Utc::now(),
        })
        .await;

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/update-profile-http",
        "type": "Update",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "id": REMOTE_ACTOR_ID,
            "type": "Person",
            "name": "Alice Updated",
            "summary": "after",
            "publicKey": {
                "publicKeyPem": "new-key"
            },
            "icon": { "url": "https://cdn.remote.example/alice.png" },
            "image": { "url": "https://cdn.remote.example/alice-header.png" },
            "inbox": "https://remote.example/inbox-new",
            "outbox": "https://remote.example/outbox-new",
            "followersCount": 10,
            "followingCount": 20
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let updated = server
        .state
        .profile_cache
        .get_by_uri(REMOTE_ACTOR_ID)
        .await
        .expect("profile should remain cached");
    assert_eq!(updated.display_name.as_deref(), Some("Alice Updated"));
    assert_eq!(updated.note.as_deref(), Some("after"));
    assert_eq!(updated.public_key_pem, "new-key");
    assert_eq!(
        updated.avatar_url.as_deref(),
        Some("https://cdn.remote.example/alice.png")
    );
    assert_eq!(
        updated.header_url.as_deref(),
        Some("https://cdn.remote.example/alice-header.png")
    );
    assert_eq!(updated.inbox_uri, "https://remote.example/inbox-new");
    assert_eq!(
        updated.outbox_uri.as_deref(),
        Some("https://remote.example/outbox-new")
    );
    assert_eq!(updated.followers_count, Some(10));
    assert_eq!(updated.following_count, Some(20));
}

#[tokio::test]
async fn test_signed_shared_inbox_accept_marks_follow_as_accepted() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: REMOTE_ACTOR_ADDRESS.to_string(),
        actor_uri: None,
        uri: server.public_url("/users/testuser/follow/accept-http"),
        created_at: Utc::now(),
    };
    server.state.db.insert_follow(&follow).await.unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/accept-http",
        "type": "Accept",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Follow",
            "id": follow.uri
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        server
            .state
            .db
            .is_follow_accepted(REMOTE_ACTOR_ADDRESS, Some(443))
            .await
            .unwrap()
    );

    let stored = server
        .state
        .db
        .get_follow(REMOTE_ACTOR_ADDRESS, Some(443))
        .await
        .unwrap()
        .expect("follow should remain");
    assert_eq!(stored.actor_uri.as_deref(), Some(REMOTE_ACTOR_ID));
}

#[tokio::test]
async fn test_signed_shared_inbox_reject_removes_pending_follow() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: REMOTE_ACTOR_ADDRESS.to_string(),
        actor_uri: None,
        uri: server.public_url("/users/testuser/follow/reject-http"),
        created_at: Utc::now(),
    };
    server.state.db.insert_follow(&follow).await.unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/reject-http",
        "type": "Reject",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Follow",
            "id": follow.uri
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        server
            .state
            .db
            .get_follow(REMOTE_ACTOR_ADDRESS, Some(443))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_signed_personal_inbox_undo_follow_removes_follower() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follower};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let follow_uri = "https://remote.example/follows/undo-http";

    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            inbox_uri: "https://remote.example/inbox".to_string(),
            uri: follow_uri.to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/undo-follow-http",
        "type": "Undo",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Follow",
            "id": follow_uri,
            "object": server.public_url("/users/testuser")
        }
    });

    let response = server
        .post_signed_activity("/users/testuser/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        server
            .state
            .db
            .get_follower(REMOTE_ACTOR_ADDRESS, Some(443))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_like_creates_favourite_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);
    let local_status_uri = insert_local_status(&server, "liked-status").await;

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/likes/http-1",
        "type": "Like",
        "actor": REMOTE_ACTOR_ID,
        "object": local_status_uri
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert!(
        notifications.iter().any(|notification| {
            notification.notification_type == rustresort::data::NotificationType::Favourite
                && notification.origin_account_address == REMOTE_ACTOR_ADDRESS
        }),
        "Like should create a favourite notification"
    );

    let status = server
        .state
        .db
        .get_status_by_uri(&local_status_uri)
        .await
        .unwrap()
        .unwrap();
    let status_response = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{}", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(status_response.status(), reqwest::StatusCode::OK);
    let body = status_response.json::<Value>().await.unwrap();
    assert_eq!(body["favourites_count"], 1);
}

#[tokio::test]
async fn test_signed_shared_inbox_announce_regular_creates_reblog_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);
    let local_status_uri = insert_local_status(&server, "boost-target").await;

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/announces/http-1",
        "type": "Announce",
        "actor": REMOTE_ACTOR_ID,
        "object": local_status_uri
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert!(
        notifications.iter().any(|notification| {
            notification.notification_type == rustresort::data::NotificationType::Reblog
                && notification.origin_account_address == REMOTE_ACTOR_ADDRESS
        }),
        "Announce of a local status should create a reblog notification"
    );

    let status = server
        .state
        .db
        .get_status_by_uri(&local_status_uri)
        .await
        .unwrap()
        .unwrap();
    let status_response = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{}", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(status_response.status(), reqwest::StatusCode::OK);
    let body = status_response.json::<Value>().await.unwrap();
    assert_eq!(body["reblogs_count"], 1);
}

#[tokio::test]
async fn test_signed_shared_inbox_create_quote_uri_persists_status_and_quote_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let local_status_uri = insert_local_status(&server, "quoted-target-http").await;
    let quote_status_uri = "https://remote.example/users/alice/statuses/quote-uri-http";

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/create-quote-uri-http",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": quote_status_uri,
            "content": "<p>Quoting a local post</p>",
            "quoteUri": local_status_uri,
            "published": "2026-01-04T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let persisted = server
        .state
        .db
        .get_status_by_uri(quote_status_uri)
        .await
        .unwrap()
        .expect("quote post should be persisted");
    assert_eq!(
        persisted.quote_of_uri.as_deref(),
        Some(local_status_uri.as_str())
    );

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert!(
        notifications.iter().any(|notification| {
            notification.notification_type == rustresort::data::NotificationType::Quote
                && notification.origin_account_address == REMOTE_ACTOR_ADDRESS
                && notification.status_uri.as_deref() == Some(quote_status_uri)
        }),
        "quote of a local status should create a quote notification"
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_announce_quote_mention_persists_status_and_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let quote_status_uri = "https://remote.example/users/alice/statuses/quote-http";

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/announces/quote-http",
        "type": "Announce",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": quote_status_uri,
            "content": "<p>Quoting @testuser</p>",
            "published": "2026-01-04T00:00:00Z",
            "to": server.public_url("/users/testuser")
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        server
            .state
            .db
            .get_status_by_uri(quote_status_uri)
            .await
            .unwrap()
            .is_some(),
        "quote mention should persist the embedded remote note"
    );

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert!(
        notifications.iter().any(|notification| {
            notification.notification_type == rustresort::data::NotificationType::Mention
                && notification.status_uri.as_deref() == Some(quote_status_uri)
        }),
        "quote mention should create a mention notification"
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_announce_quote_and_mention_creates_both_notifications() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let local_status_uri = insert_local_status(&server, "announce-quoted-target").await;
    let quote_status_uri = "https://remote.example/users/alice/statuses/announce-quote-mention";

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/announces/quote-and-mention",
        "type": "Announce",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": quote_status_uri,
            "content": "<p>Quoting @testuser</p>",
            "published": "2026-01-04T00:00:00Z",
            "quoteUri": local_status_uri,
            "to": [server.public_url("/users/testuser")]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert!(notifications.iter().any(|notification| {
        notification.notification_type == rustresort::data::NotificationType::Quote
            && notification.status_uri.as_deref() == Some(quote_status_uri)
    }));
    assert!(notifications.iter().any(|notification| {
        notification.notification_type == rustresort::data::NotificationType::Mention
            && notification.status_uri.as_deref() == Some(quote_status_uri)
    }));
}

#[tokio::test]
async fn test_signed_shared_inbox_update_note_creates_quoted_update_notification_for_local_quote() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let remote_status_uri = "https://remote.example/users/alice/statuses/quoted-update-http";
    let local_quote_uri = server.public_url("/users/testuser/statuses/local-quote-http");

    server
        .state
        .db
        .insert_status(&Status {
            id: EntityId::new_string(),
            uri: remote_status_uri.to_string(),
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
        })
        .await
        .unwrap();

    server
        .state
        .db
        .insert_status(&Status {
            id: EntityId::new_string(),
            uri: local_quote_uri.clone(),
            content: "<p>local quote</p>".to_string(),
            content_warning: None,
            visibility: StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "testuser@test.example.com".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: Some(remote_status_uri.to_string()),
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        })
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/update-quoted-http",
        "type": "Update",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": remote_status_uri,
            "content": "<p>after</p>",
            "published": "2026-01-05T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert!(
        notifications.iter().any(|notification| {
            notification.notification_type == rustresort::data::NotificationType::QuotedUpdate
                && notification.origin_account_address == REMOTE_ACTOR_ADDRESS
                && notification.status_uri.as_deref() == Some(local_quote_uri.as_str())
        }),
        "editing a remotely quoted status should notify with the local quote status"
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_update_note_creates_quoted_update_for_each_local_quote() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let remote_status_uri = "https://remote.example/users/alice/statuses/quoted-update-multi";

    server
        .state
        .db
        .insert_status(&Status {
            id: EntityId::new_string(),
            uri: remote_status_uri.to_string(),
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
        })
        .await
        .unwrap();

    for suffix in ["1", "2"] {
        server
            .state
            .db
            .insert_status(&Status {
                id: format!("local-quote-{suffix}"),
                uri: server.public_url(&format!("/users/testuser/statuses/local-quote-{suffix}")),
                content: format!("<p>local quote {suffix}</p>"),
                content_warning: None,
                visibility: StatusVisibility::Public,
                language: Some("en".to_string()),
                account_address: "testuser@test.example.com".to_string(),
                is_local: true,
                in_reply_to_uri: None,
                boost_of_uri: None,
                quote_of_uri: Some(remote_status_uri.to_string()),
                persisted_reason: PersistedReason::Own,
                created_at: Utc::now(),
                fetched_at: None,
            })
            .await
            .unwrap();
    }

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/update-quoted-multi",
        "type": "Update",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": remote_status_uri,
            "content": "<p>after</p>",
            "published": "2026-01-05T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    let quoted_updates = notifications
        .iter()
        .filter(|notification| {
            notification.notification_type == rustresort::data::NotificationType::QuotedUpdate
        })
        .count();
    assert_eq!(quoted_updates, 2);
}

#[tokio::test]
async fn test_signed_shared_inbox_undo_like_removes_favourite_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);
    let local_status_uri = insert_local_status(&server, "undo-like-target").await;
    let like_id = "https://remote.example/likes/http-undo";

    let like = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": like_id,
        "type": "Like",
        "actor": REMOTE_ACTOR_ID,
        "object": local_status_uri
    });
    let like_response = server.post_signed_activity("/inbox", &like, &key_id).await;
    assert_eq!(like_response.status(), reqwest::StatusCode::OK);

    let undo = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/undo-like-http",
        "type": "Undo",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Like",
            "id": like_id,
            "object": local_status_uri
        }
    });
    let undo_response = server.post_signed_activity("/inbox", &undo, &key_id).await;
    assert_eq!(undo_response.status(), reqwest::StatusCode::OK);

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert!(
        notifications.iter().all(|notification| {
            notification.notification_type != rustresort::data::NotificationType::Favourite
        }),
        "Undo Like should remove the favourite notification"
    );

    let status = server
        .state
        .db
        .get_status_by_uri(&local_status_uri)
        .await
        .unwrap()
        .unwrap();
    let status_response = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{}", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    let body = status_response.json::<Value>().await.unwrap();
    assert_eq!(body["favourites_count"], 0);
}

#[tokio::test]
async fn test_signed_shared_inbox_compact_undo_like_removes_remote_favourite_count() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);
    let local_status_uri = insert_local_status(&server, "compact-undo-like-target").await;
    let like_id = "https://remote.example/likes/compact-undo";

    let like = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": like_id,
        "type": "Like",
        "actor": REMOTE_ACTOR_ID,
        "object": local_status_uri
    });
    let like_response = server.post_signed_activity("/inbox", &like, &key_id).await;
    assert_eq!(like_response.status(), reqwest::StatusCode::OK);

    let undo = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/compact-undo-like",
        "type": "Undo",
        "actor": REMOTE_ACTOR_ID,
        "object": like_id
    });
    let undo_response = server.post_signed_activity("/inbox", &undo, &key_id).await;
    assert_eq!(undo_response.status(), reqwest::StatusCode::OK);

    let status = server
        .state
        .db
        .get_status_by_uri(&local_status_uri)
        .await
        .unwrap()
        .unwrap();
    let status_response = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{}", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    let body = status_response.json::<Value>().await.unwrap();
    assert_eq!(body["favourites_count"], 0);
}

#[tokio::test]
async fn test_signed_shared_inbox_undo_like_cannot_remove_another_actors_favourite() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);
    let evil_key_id = register_evil_remote_key(&server);
    let local_status_uri = insert_local_status(&server, "forged-undo-like-target").await;
    let like_id = "https://remote.example/likes/forged-undo";

    let like = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": like_id,
        "type": "Like",
        "actor": REMOTE_ACTOR_ID,
        "object": local_status_uri
    });
    let like_response = server.post_signed_activity("/inbox", &like, &key_id).await;
    assert_eq!(like_response.status(), reqwest::StatusCode::OK);

    let forged_undo = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://evil.example/activities/forged-undo-like",
        "type": "Undo",
        "actor": EVIL_ACTOR_ID,
        "object": {
            "type": "Like",
            "id": like_id,
            "object": local_status_uri
        }
    });
    let undo_response = server
        .post_signed_activity("/inbox", &forged_undo, &evil_key_id)
        .await;
    assert_eq!(undo_response.status(), reqwest::StatusCode::OK);

    let status = server
        .state
        .db
        .get_status_by_uri(&local_status_uri)
        .await
        .unwrap()
        .unwrap();
    let status_response = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{}", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    let body = status_response.json::<Value>().await.unwrap();
    assert_eq!(body["favourites_count"], 1);

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert!(notifications.iter().any(|notification| {
        notification.notification_type == rustresort::data::NotificationType::Favourite
            && notification.origin_account_address == REMOTE_ACTOR_ADDRESS
    }));
    assert!(
        notifications
            .iter()
            .all(|notification| { notification.origin_account_address != EVIL_ACTOR_ADDRESS })
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_undo_announce_removes_reblog_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);
    let local_status_uri = insert_local_status(&server, "undo-announce-target").await;
    let announce_id = "https://remote.example/announces/http-undo";

    let announce = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": announce_id,
        "type": "Announce",
        "actor": REMOTE_ACTOR_ID,
        "object": local_status_uri
    });
    let announce_response = server
        .post_signed_activity("/inbox", &announce, &key_id)
        .await;
    assert_eq!(announce_response.status(), reqwest::StatusCode::OK);

    let undo = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/undo-announce-http",
        "type": "Undo",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Announce",
            "id": announce_id,
            "object": local_status_uri
        }
    });
    let undo_response = server.post_signed_activity("/inbox", &undo, &key_id).await;
    assert_eq!(undo_response.status(), reqwest::StatusCode::OK);

    let notifications = server
        .state
        .db
        .get_notifications(10, None, false)
        .await
        .unwrap();
    assert!(
        notifications.iter().all(|notification| {
            notification.notification_type != rustresort::data::NotificationType::Reblog
        }),
        "Undo Announce should remove the reblog notification"
    );

    let status = server
        .state
        .db
        .get_status_by_uri(&local_status_uri)
        .await
        .unwrap()
        .unwrap();
    let status_response = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{}", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    let body = status_response.json::<Value>().await.unwrap();
    assert_eq!(body["reblogs_count"], 0);
}

#[tokio::test]
async fn test_signed_shared_inbox_compact_undo_announce_removes_remote_repost_count() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);
    let local_status_uri = insert_local_status(&server, "compact-undo-announce-target").await;
    let announce_id = "https://remote.example/announces/compact-undo";

    let announce = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": announce_id,
        "type": "Announce",
        "actor": REMOTE_ACTOR_ID,
        "object": local_status_uri
    });
    let announce_response = server
        .post_signed_activity("/inbox", &announce, &key_id)
        .await;
    assert_eq!(announce_response.status(), reqwest::StatusCode::OK);

    let undo = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/compact-undo-announce",
        "type": "Undo",
        "actor": REMOTE_ACTOR_ID,
        "object": announce_id
    });
    let undo_response = server.post_signed_activity("/inbox", &undo, &key_id).await;
    assert_eq!(undo_response.status(), reqwest::StatusCode::OK);

    let status = server
        .state
        .db
        .get_status_by_uri(&local_status_uri)
        .await
        .unwrap()
        .unwrap();
    let status_response = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{}", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    let body = status_response.json::<Value>().await.unwrap();
    assert_eq!(body["reblogs_count"], 0);
}

#[tokio::test]
async fn test_signed_user_inbox_compact_undo_follow_removes_follower_after_follow_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let follow_uri = "https://remote.example/follows/compact-undo-http";

    let follow = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": follow_uri,
        "type": "Follow",
        "actor": {
            "id": REMOTE_ACTOR_ID,
            "inbox": "https://remote.example/users/alice/inbox"
        },
        "object": server.public_url("/users/testuser")
    });
    let follow_response = server
        .post_signed_activity("/users/testuser/inbox", &follow, &key_id)
        .await;
    assert_eq!(follow_response.status(), reqwest::StatusCode::OK);

    let undo = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/compact-undo-follow-http",
        "type": "Undo",
        "actor": REMOTE_ACTOR_ID,
        "object": follow_uri
    });
    let undo_response = server
        .post_signed_activity("/users/testuser/inbox", &undo, &key_id)
        .await;
    assert_eq!(undo_response.status(), reqwest::StatusCode::OK);
    assert!(
        server
            .state
            .db
            .get_follower(REMOTE_ACTOR_ADDRESS, Some(443))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_announce_embedded_quote_rejects_mismatched_attribution() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let local_status_uri = insert_local_status(&server, "quoted-target-mismatch").await;
    let quote_status_uri = "https://remote.example/users/alice/statuses/mismatch-quote";

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/announces/mismatched-attribution",
        "type": "Announce",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "id": quote_status_uri,
            "attributedTo": "https://evil.example/users/mallory",
            "content": "<p>forged quote</p>",
            "published": "2026-01-04T00:00:00Z",
            "quoteUri": local_status_uri,
            "to": [server.public_url("/users/testuser")]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(
        server
            .state
            .db
            .get_status_by_uri(quote_status_uri)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_block_records_remote_block_and_filters_delivery() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follower};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            inbox_uri: "https://remote.example/inbox".to_string(),
            uri: "https://remote.example/follows/alice".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/blocks/http-1",
        "type": "Block",
        "actor": REMOTE_ACTOR_ID,
        "object": server.public_url("/users/testuser")
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        server
            .state
            .db
            .is_blocked_by_remote(REMOTE_ACTOR_ID)
            .await
            .unwrap()
    );
    assert!(
        server
            .state
            .db
            .get_follower_inboxes()
            .await
            .unwrap()
            .is_empty(),
        "followers who blocked the local user should no longer receive delivery"
    );
    assert!(
        server
            .state
            .db
            .get_follower(REMOTE_ACTOR_ADDRESS, Some(443))
            .await
            .unwrap()
            .is_none(),
        "remote Block should remove the follower row as well"
    );
    assert!(
        server
            .state
            .db
            .get_notifications(10, None, false)
            .await
            .unwrap()
            .is_empty(),
        "Block should not create notifications"
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_move_with_uri_target_requires_target_backlink() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/original".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let key_id = register_default_remote_key(&server);
    let new_actor_uri = "https://remote.example/users/alice-new";
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/move-with-uri-target",
        "type": "Move",
        "actor": REMOTE_ACTOR_ID,
        "object": REMOTE_ACTOR_ID,
        "target": {
            "id": new_actor_uri,
            "inbox": "https://remote.example/users/alice-new/inbox"
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        server
            .state
            .db
            .get_follow(REMOTE_ACTOR_ADDRESS, None)
            .await
            .unwrap()
            .is_some(),
        "Move without alsoKnownAs backlink should keep the original follow"
    );
    assert!(
        server
            .state
            .db
            .get_follow("alice-new@remote.example", None)
            .await
            .unwrap()
            .is_none(),
        "unverified Move target must not receive a replacement follow"
    );
}

#[tokio::test]
async fn test_signed_shared_inbox_update_person_populates_profile_cache_when_missing() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/update-person-http",
        "type": "Update",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "id": REMOTE_ACTOR_ID,
            "type": "Person",
            "preferredUsername": "alice",
            "name": "Alice Remote",
            "summary": "<p>Updated profile</p>",
            "inbox": "https://remote.example/inbox",
            "publicKey": {
                "id": "https://remote.example/users/alice#main-key",
                "owner": REMOTE_ACTOR_ID,
                "publicKeyPem": common::test_public_key_pem()
            }
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let profile = server
        .state
        .profile_cache
        .get_by_uri(REMOTE_ACTOR_ID)
        .await
        .expect("Update(Person) should populate the profile cache");
    assert_eq!(profile.address, REMOTE_ACTOR_ADDRESS);
    assert_eq!(profile.display_name.as_deref(), Some("Alice Remote"));
    assert_eq!(profile.inbox_uri, "https://remote.example/inbox");
}

#[tokio::test]
async fn test_signed_shared_inbox_rejects_actor_blocked_by_local_user() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);

    server
        .state
        .db
        .block_account_with_remote_metadata(
            REMOTE_ACTOR_ADDRESS,
            Some(REMOTE_ACTOR_ID),
            Some("https://remote.example/inbox"),
            Some(443),
        )
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/blocked-create",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "id": "https://remote.example/users/alice/statuses/blocked-create",
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "content": "<p>Should be rejected</p>",
            "published": "2025-01-01T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_signed_create_rejects_mismatched_object_attribution() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/mismatch-create";

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/mismatch-create",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "id": status_uri,
            "type": "Note",
            "attributedTo": "https://evil.example/users/mallory",
            "content": "<p>forged</p>",
            "published": "2025-01-01T00:00:00Z",
            "to": [server.public_url("/users/testuser")]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(
        server
            .state
            .db
            .get_status_by_uri(status_uri)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_signed_update_rejects_mismatched_object_attribution() {
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);
    let status_uri = "https://remote.example/users/alice/statuses/mismatch-update";

    server
        .state
        .db
        .insert_status(&Status {
            id: EntityId::new_string(),
            uri: status_uri.to_string(),
            content: "<p>original</p>".to_string(),
            content_warning: None,
            visibility: StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: REMOTE_ACTOR_ADDRESS.to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Mentioned,
            created_at: chrono::Utc::now(),
            fetched_at: None,
        })
        .await
        .unwrap();

    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/mismatch-update",
        "type": "Update",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "id": status_uri,
            "type": "Note",
            "attributedTo": "https://evil.example/users/mallory",
            "content": "<p>forged update</p>",
            "published": "2025-01-01T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let persisted = server
        .state
        .db
        .get_status_by_uri(status_uri)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.content, "<p>original</p>");
}

#[tokio::test]
async fn test_actor_content_negotiation() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Request with HTML Accept header
    let _html_response = server
        .client
        .get(server.url("/users/testuser"))
        .header("Accept", "text/html")
        .send()
        .await
        .unwrap();

    // Request with ActivityPub Accept header
    let ap_response = server
        .client
        .get(server.url("/users/testuser"))
        .header("Accept", "application/activity+json")
        .send()
        .await
        .unwrap();

    // Should handle content negotiation differently
    // HTML might redirect or return HTML page
    // ActivityPub should return JSON
    assert_eq!(ap_response.status(), 200);
    let content_type = ap_response.headers().get("content-type").unwrap();
    assert_eq!(content_type.to_str().unwrap(), "application/activity+json");
}
