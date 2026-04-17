//! E2E tests for timeline operations

mod common;

use common::TestServer;
use serde_json::Value;

const REMOTE_ACTOR_ID: &str = "https://remote.example/users/alice";
const REMOTE_ACTOR_ADDRESS: &str = "alice@remote.example";

fn register_default_remote_key(server: &TestServer) -> String {
    let key_id = format!("{REMOTE_ACTOR_ID}#main-key");
    server.register_inbound_public_key(&key_id, common::test_public_key_pem());
    key_id
}

#[tokio::test]
async fn test_home_timeline_without_auth() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/home"))
        .send()
        .await
        .unwrap();

    // Should return 401 Unauthorized
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_home_timeline_with_auth() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/home"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should return timeline if implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_public_timeline() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();

    // Public timeline should be accessible without auth
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_public_timeline_only_media_includes_remote_status_with_remote_attachment() {
    use chrono::Utc;
    use rustresort::data::{PersistedReason, RemoteStatusAttachment, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;

    server
        .state
        .profile_cache
        .insert(rustresort::data::CachedProfile {
            address: REMOTE_ACTOR_ADDRESS.to_string(),
            uri: REMOTE_ACTOR_ID.to_string(),
            display_name: Some("Alice Remote".to_string()),
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            avatar_url: None,
            header_url: None,
            public_key_pem: "-----BEGIN PUBLIC KEY-----\nMIIB\n-----END PUBLIC KEY-----"
                .to_string(),
            inbox_uri: "https://remote.example/inbox".to_string(),
            outbox_uri: None,
            followers_count: Some(1),
            following_count: Some(1),
            fetched_at: Utc::now(),
        })
        .await;

    let with_media = Status {
        id: "remote-with-media".to_string(),
        uri: "https://remote.example/users/alice/statuses/with-media".to_string(),
        content: "<p>remote media post</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: REMOTE_ACTOR_ADDRESS.to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Timeline,
        created_at: Utc::now(),
        fetched_at: Some(Utc::now()),
    };
    let without_media = Status {
        id: "remote-without-media".to_string(),
        uri: "https://remote.example/users/alice/statuses/without-media".to_string(),
        content: "<p>remote text post</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: REMOTE_ACTOR_ADDRESS.to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Timeline,
        created_at: Utc::now(),
        fetched_at: Some(Utc::now()),
    };
    server.state.db.insert_status(&with_media).await.unwrap();
    server.state.db.insert_status(&without_media).await.unwrap();
    server
        .state
        .db
        .replace_remote_status_attachments(
            &with_media.id,
            &[RemoteStatusAttachment {
                id: "remote-attachment-1".to_string(),
                status_id: with_media.id.clone(),
                remote_url: "https://remote.example/media/1.webp".to_string(),
                preview_url: Some("https://remote.example/media/1-preview.webp".to_string()),
                content_type: "image/webp".to_string(),
                description: Some("remote alt".to_string()),
                blurhash: None,
                width: Some(64),
                height: Some(64),
                created_at: Utc::now(),
            }],
        )
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/timelines/public?remote=true&only_media=true"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    let statuses = body.as_array().unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0]["id"], with_media.id);
}

#[tokio::test]
async fn test_public_timeline_honors_since_id_and_min_id() {
    use chrono::{Duration, Utc};
    use rustresort::data::{PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let now = Utc::now();

    let oldest = Status {
        id: "remote-oldest".to_string(),
        uri: "https://test.example.com/users/testuser/statuses/oldest".to_string(),
        content: "<p>oldest</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: now - Duration::seconds(3),
        fetched_at: None,
    };
    let middle = Status {
        id: "remote-middle".to_string(),
        uri: "https://test.example.com/users/testuser/statuses/middle".to_string(),
        content: "<p>middle</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: now - Duration::seconds(2),
        fetched_at: None,
    };
    let newest = Status {
        id: "remote-newest".to_string(),
        uri: "https://test.example.com/users/testuser/statuses/newest".to_string(),
        content: "<p>newest</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: now - Duration::seconds(1),
        fetched_at: None,
    };

    server.state.db.insert_status(&oldest).await.unwrap();
    server.state.db.insert_status(&middle).await.unwrap();
    server.state.db.insert_status(&newest).await.unwrap();

    let since_response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .query(&[("since_id", middle.id.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(since_response.status(), 200);
    let since_body: Value = since_response.json().await.unwrap();
    let since_entries = since_body.as_array().expect("timeline should be array");
    assert_eq!(since_entries.len(), 1);
    assert_eq!(since_entries[0]["id"], newest.id);

    let min_response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .query(&[("min_id", middle.id.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(min_response.status(), 200);
    let min_body: Value = min_response.json().await.unwrap();
    let min_entries = min_body.as_array().expect("timeline should be array");
    assert_eq!(min_entries.len(), 1);
    assert_eq!(min_entries[0]["id"], newest.id);
}

#[tokio::test]
async fn test_local_timeline() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/public?local=true"))
        .send()
        .await
        .unwrap();

    // Local timeline should be accessible
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_home_timeline_includes_cached_remote_followee_status() {
    use chrono::Utc;
    use rustresort::data::{CachedStatus, EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let actor_uri = "https://remote.example/users/alice";
    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: actor_uri.to_string(),
            actor_uri: Some(actor_uri.to_string()),
            uri: "https://test.example.com/users/testuser/follow/alice".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let status_uri = "https://remote.example/users/alice/statuses/cached-home";
    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: status_uri.to_string(),
            uri: status_uri.to_string(),
            content: "<p>Cached followee post</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
        })
        .await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/home"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let entries = json.as_array().expect("timeline should be array");
    assert!(
        entries.iter().any(|item| item["uri"] == status_uri),
        "home timeline should include cached remote followee status"
    );
}

#[tokio::test]
async fn test_home_timeline_excludes_silenced_remote_followee_status() {
    use chrono::Utc;
    use rustresort::data::{CachedStatus, EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let actor_uri = "https://remote.example/users/alice";
    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: actor_uri.to_string(),
            actor_uri: Some(actor_uri.to_string()),
            uri: "https://test.example.com/users/testuser/follow/alice-silenced".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    server
        .state
        .db
        .upsert_domain_block("remote.example", "silence", false, false, None, None, false)
        .await
        .unwrap();

    let status_uri = "https://remote.example/users/alice/statuses/cached-home-silenced";
    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: status_uri.to_string(),
            uri: status_uri.to_string(),
            content: "<p>Cached silenced followee post</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
        })
        .await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/home"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let entries = json.as_array().expect("timeline should be array");
    assert!(entries.iter().all(|item| item["uri"] != status_uri));
}

#[tokio::test]
async fn test_public_timeline_includes_cached_remote_public_status() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    let status_uri = "https://remote.example/users/alice/statuses/cached-public";
    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: status_uri.to_string(),
            uri: status_uri.to_string(),
            content: "<p>Cached public post</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
        })
        .await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let entries = json.as_array().expect("timeline should be array");
    assert!(
        entries.iter().any(|item| item["uri"] == status_uri),
        "public timeline should include cached remote public status"
    );
}

#[tokio::test]
async fn test_public_timeline_excludes_silenced_remote_domain_statuses() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    server
        .state
        .db
        .upsert_domain_block("remote.example", "silence", false, false, None, None, false)
        .await
        .unwrap();

    let status_uri = "https://remote.example/users/alice/statuses/cached-public-silenced";
    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: status_uri.to_string(),
            uri: status_uri.to_string(),
            content: "<p>Cached public post</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
        })
        .await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let entries = json.as_array().expect("timeline should be array");
    assert!(entries.iter().all(|item| item["uri"] != status_uri));
}

#[tokio::test]
async fn test_public_timeline_preserves_cached_quote_relationship() {
    use chrono::Utc;
    use rustresort::data::{CachedStatus, EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;

    let quoted_target = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/users/testuser/statuses/quoted-target".to_string(),
        content: "<p>Quoted target</p>".to_string(),
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
    server.state.db.insert_status(&quoted_target).await.unwrap();

    let status_uri = "https://remote.example/users/alice/statuses/cached-quote";
    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: status_uri.to_string(),
            uri: status_uri.to_string(),
            content: "<p>Cached public quote</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: Some(quoted_target.uri.clone()),
        })
        .await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let entries = json.as_array().expect("timeline should be array");
    let cached_quote = entries
        .iter()
        .find(|item| item["uri"] == status_uri)
        .expect("public timeline should include cached quoted status");
    assert_eq!(cached_quote["quote"]["uri"], quoted_target.uri);
}

#[tokio::test]
async fn test_public_timeline_since_id_filters_cached_remote_statuses_by_sort_order() {
    use chrono::{Duration, Utc};
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    let now = Utc::now();

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: "100".to_string(),
            uri: "https://remote.example/users/alice/statuses/cached-newest".to_string(),
            content: "<p>Newest cached public post</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: now,
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
        })
        .await;
    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: "300".to_string(),
            uri: "https://remote.example/users/alice/statuses/cached-oldest".to_string(),
            content: "<p>Oldest cached public post</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: now - Duration::seconds(30),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
        })
        .await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .query(&[("since_id", "300")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let entries = json.as_array().expect("timeline should be array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], "100");
}

#[tokio::test]
async fn test_public_timeline_max_id_cache_cursor_excludes_newer_local_statuses() {
    use chrono::{Duration, Utc};
    use rustresort::data::{CachedStatus, EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    let now = Utc::now();

    let local_status = Status {
        id: EntityId::new_string(),
        uri: server.public_url("/users/testuser/statuses/local-newer-public"),
        content: "<p>Newer local public post</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: now,
        fetched_at: None,
    };
    server.state.db.insert_status(&local_status).await.unwrap();

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: "cached-older-public".to_string(),
            uri: "https://remote.example/users/alice/statuses/cached-older-public".to_string(),
            content: "<p>Older cached public post</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: now - Duration::seconds(30),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
        })
        .await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .query(&[("max_id", "cached-older-public")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let entries = json.as_array().expect("timeline should be array");
    assert!(
        entries.is_empty(),
        "newer local rows must not be reintroduced when max_id points at a cache-only remote status"
    );
}

#[tokio::test]
async fn test_home_timeline_includes_persisted_remote_followee_status_after_cache_eviction() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/home-persisted".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let status_uri = "https://remote.example/users/alice/statuses/home-db-after-eviction";
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/home-db-after-eviction",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>Persisted followee post</p>",
            "published": "2026-01-04T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), 200);

    server.state.timeline_cache.remove_by_uri(status_uri).await;

    let timeline_response = server
        .client
        .get(server.url("/api/v1/timelines/home"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(timeline_response.status(), 200);
    let body: Value = timeline_response.json().await.unwrap();
    assert!(
        body.as_array()
            .expect("timeline should be array")
            .iter()
            .any(|item| item["uri"] == status_uri),
        "home timeline should still materialize persisted followee statuses after cache eviction"
    );
}

#[tokio::test]
async fn test_home_timeline_includes_persisted_remote_followee_announce_after_cache_eviction() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/home-announce-persisted"
                .to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let announced_status_uri = "https://remote.example/users/bob/statuses/boost-target";
    let announce_activity_uri = "https://remote.example/activities/home-announce-followee";
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": announce_activity_uri,
        "type": "Announce",
        "actor": REMOTE_ACTOR_ID,
        "object": announced_status_uri,
        "published": "2026-01-06T00:00:00Z",
        "to": ["https://www.w3.org/ns/activitystreams#Public"]
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), 200);

    let persisted = server
        .state
        .db
        .get_status_by_uri(announce_activity_uri)
        .await
        .unwrap()
        .expect("followee Announce should be persisted for restart-safe timelines");
    assert_eq!(
        persisted.boost_of_uri.as_deref(),
        Some(announced_status_uri)
    );

    server
        .state
        .timeline_cache
        .remove_by_uri(announce_activity_uri)
        .await;

    let timeline_response = server
        .client
        .get(server.url("/api/v1/timelines/home"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(timeline_response.status(), 200);
    let body: Value = timeline_response.json().await.unwrap();
    let entry = body
        .as_array()
        .expect("timeline should be array")
        .iter()
        .find(|item| item["uri"] == announce_activity_uri)
        .expect("home timeline should materialize persisted followee Announce rows");
    assert_eq!(entry["reblog"]["uri"], announced_status_uri);
}

#[tokio::test]
async fn test_public_timeline_includes_persisted_remote_status_after_cache_eviction() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let key_id = register_default_remote_key(&server);

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/public-persisted".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let status_uri = "https://remote.example/users/alice/statuses/public-db-after-eviction";
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/public-db-after-eviction",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>Persisted public followee post</p>",
            "published": "2026-01-05T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), 200);

    server.state.timeline_cache.remove_by_uri(status_uri).await;

    let timeline_response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();
    assert_eq!(timeline_response.status(), 200);
    let body: Value = timeline_response.json().await.unwrap();
    assert!(
        body.as_array()
            .expect("timeline should be array")
            .iter()
            .any(|item| item["uri"] == status_uri),
        "public timeline should still materialize persisted remote statuses after cache eviction"
    );
}

#[tokio::test]
async fn test_tag_timeline_includes_persisted_remote_followee_status() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/alice".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let status_uri = "https://remote.example/users/alice/statuses/tagged";
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/create-tagged",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>Hello #persistedtag</p>",
            "published": "2026-01-02T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), 200);

    let timeline_response = server
        .client
        .get(server.url("/api/v1/timelines/tag/persistedtag"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(timeline_response.status(), 200);
    let body: Value = timeline_response.json().await.unwrap();
    assert!(
        body.as_array()
            .expect("timeline should be array")
            .iter()
            .any(|item| item["uri"] == status_uri),
        "tag timeline should materialize persisted followee statuses"
    );
}

#[tokio::test]
async fn test_local_public_timeline_excludes_cached_remote_public_status() {
    use chrono::Utc;
    use rustresort::data::{CachedStatus, EntityId, Status};

    let server = TestServer::new().await;
    let remote_status_uri = "https://remote.example/users/alice/statuses/local-only-excluded";
    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_status_uri.to_string(),
            uri: remote_status_uri.to_string(),
            content: "<p>Remote cached public post</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
        })
        .await;

    let local_status = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/statuses/local-public".to_string(),
        content: "<p>Local public post</p>".to_string(),
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
    };
    server.state.db.insert_status(&local_status).await.unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/timelines/public?local=true"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let entries = json.as_array().expect("timeline should be array");
    assert!(entries.iter().any(|item| item["uri"] == local_status.uri));
    assert!(!entries.iter().any(|item| item["uri"] == remote_status_uri));
}

#[tokio::test]
async fn test_timeline_pagination() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create multiple statuses
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    for i in 0..5 {
        let status = Status {
            id: EntityId::new_string(),
            uri: format!("https://test.example.com/status/{}", i),
            content: format!("<p>Status {}</p>", i),
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
    }

    let response = server
        .client
        .get(server.url("/api/v1/timelines/public?limit=3"))
        .send()
        .await
        .unwrap();

    // Should return limited number of statuses
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
        if !json.as_array().unwrap().is_empty() {
            assert!(json.as_array().unwrap().len() <= 3);
        }
    }
}

#[tokio::test]
async fn test_hashtag_timeline() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let tagged_public = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/tagged-public".to_string(),
        content: "<p>Learning #rust today</p>".to_string(),
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
    };
    let tagged_private = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/tagged-private".to_string(),
        content: "<p>Private #rust note</p>".to_string(),
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
    };
    let untagged_public = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/untagged-public".to_string(),
        content: "<p>No hashtag here</p>".to_string(),
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
    };
    server.state.db.insert_status(&tagged_public).await.unwrap();
    server
        .state
        .db
        .insert_status(&tagged_private)
        .await
        .unwrap();
    server
        .state
        .db
        .insert_status(&untagged_public)
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/timelines/tag/rust"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Hashtag timeline should be accessible
    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert!(json.is_array());
    let ids: Vec<String> = json
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str().map(ToString::to_string))
        .collect();
    assert!(ids.contains(&tagged_public.id));
    assert!(!ids.contains(&tagged_private.id));
    assert!(!ids.contains(&untagged_public.id));
}

#[tokio::test]
async fn test_list_timeline_returns_statuses_for_list_accounts() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let list_id = server
        .state
        .db
        .create_list("Test list", "list")
        .await
        .unwrap();
    let local_address = format!("{}@{}", account.username, server.state.config.server.domain);
    let remote_address = "alice@example.com".to_string();
    server
        .state
        .db
        .add_accounts_to_list(&list_id, &[local_address.clone(), remote_address.clone()])
        .await
        .unwrap();

    let local_status = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/list-local".to_string(),
        content: "<p>Local list status</p>".to_string(),
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
    };
    let remote_status = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/status/list-remote".to_string(),
        content: "<p>Remote list status</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: remote_address.clone(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Favourited,
        created_at: Utc::now(),
        fetched_at: None,
    };
    let unrelated_status = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/status/list-unrelated".to_string(),
        content: "<p>Unrelated list status</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "bob@example.com".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Favourited,
        created_at: Utc::now(),
        fetched_at: None,
    };
    server.state.db.insert_status(&local_status).await.unwrap();
    server.state.db.insert_status(&remote_status).await.unwrap();
    server
        .state
        .db
        .insert_status(&unrelated_status)
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url(&format!("/api/v1/timelines/list/{}", list_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert!(json.is_array());
    let ids: Vec<String> = json
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str().map(ToString::to_string))
        .collect();
    assert!(ids.contains(&local_status.id));
    assert!(ids.contains(&remote_status.id));
    assert!(!ids.contains(&unrelated_status.id));
}

#[tokio::test]
async fn test_list_timeline_includes_persisted_remote_followee_status() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let key_id = register_default_remote_key(&server);

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: REMOTE_ACTOR_ADDRESS.to_string(),
            actor_uri: Some(REMOTE_ACTOR_ID.to_string()),
            uri: "https://test.example.com/users/testuser/follow/alice-list".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let list_id = server
        .state
        .db
        .create_list("Remote persisted list", "list")
        .await
        .unwrap();
    server
        .state
        .db
        .add_account_to_list(&list_id, REMOTE_ACTOR_ADDRESS)
        .await
        .unwrap();

    let status_uri = "https://remote.example/users/alice/statuses/list-persisted";
    let activity = serde_json::json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/create-list-persisted",
        "type": "Create",
        "actor": REMOTE_ACTOR_ID,
        "object": {
            "type": "Note",
            "attributedTo": REMOTE_ACTOR_ID,
            "id": status_uri,
            "content": "<p>Hello list timeline</p>",
            "published": "2026-01-03T00:00:00Z",
            "to": ["https://www.w3.org/ns/activitystreams#Public"]
        }
    });

    let response = server
        .post_signed_activity("/inbox", &activity, &key_id)
        .await;
    assert_eq!(response.status(), 200);

    let timeline_response = server
        .client
        .get(server.url(&format!("/api/v1/timelines/list/{}", list_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(timeline_response.status(), 200);
    let body: Value = timeline_response.json().await.unwrap();
    assert!(
        body.as_array()
            .expect("timeline should be array")
            .iter()
            .any(|item| item["uri"] == status_uri),
        "list timeline should materialize persisted remote statuses for listed accounts"
    );
}

#[tokio::test]
async fn test_list_timeline_excludes_direct_statuses_from_listed_accounts() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let list_id = server
        .state
        .db
        .create_list("Direct exclusion list", "list")
        .await
        .unwrap();
    server
        .state
        .db
        .add_accounts_to_list(&list_id, std::slice::from_ref(&account.id))
        .await
        .unwrap();

    let direct_status = Status {
        id: EntityId::new_string(),
        uri: server.public_url("/users/testuser/statuses/list-direct-hidden"),
        content: "<p>Direct list hidden</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Direct,
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    server.state.db.insert_status(&direct_status).await.unwrap();

    let response = server
        .client
        .get(server.url(&format!("/api/v1/timelines/list/{}", list_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert!(
        json.as_array()
            .expect("timeline should be array")
            .iter()
            .all(|item| item["id"] != direct_status.id),
        "direct statuses must not leak into list timelines"
    );
}

#[tokio::test]
async fn test_list_timeline_matches_local_account_added_by_id() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let list_id = server
        .state
        .db
        .create_list("Test list by id", "list")
        .await
        .unwrap();
    let account_id = account.id.to_string();
    server
        .state
        .db
        .add_accounts_to_list(&list_id, std::slice::from_ref(&account_id))
        .await
        .unwrap();

    let local_status = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/list-local-id".to_string(),
        content: "<p>Local list status by id</p>".to_string(),
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
    };
    server.state.db.insert_status(&local_status).await.unwrap();

    let response = server
        .client
        .get(server.url(&format!("/api/v1/timelines/list/{}", list_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let ids: Vec<String> = json
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str().map(ToString::to_string))
        .collect();
    assert!(ids.contains(&local_status.id));
}

#[tokio::test]
async fn test_list_timeline_respects_none_replies_policy() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let list_id = server
        .state
        .db
        .create_list("Replies none list", "none")
        .await
        .unwrap();
    let local_address = format!("{}@{}", account.username, server.state.config.server.domain);
    server
        .state
        .db
        .add_accounts_to_list(&list_id, &[local_address])
        .await
        .unwrap();

    let root = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/list-none-root".to_string(),
        content: "<p>Root status</p>".to_string(),
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
    };
    let reply = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/list-none-reply".to_string(),
        content: "<p>Reply status</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: Some(root.uri.clone()),
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    server.state.db.insert_status(&root).await.unwrap();
    server.state.db.insert_status(&reply).await.unwrap();

    let response = server
        .client
        .get(server.url(&format!("/api/v1/timelines/list/{}", list_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let ids: Vec<String> = json
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str().map(ToString::to_string))
        .collect();
    assert!(ids.contains(&root.id));
    assert!(!ids.contains(&reply.id));
}

#[tokio::test]
async fn test_list_timeline_none_policy_fetches_past_reply_only_page() {
    use chrono::Utc;
    use rustresort::data::Status;

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let list_id = server
        .state
        .db
        .create_list("Replies none pagination", "none")
        .await
        .unwrap();
    let local_address = format!("{}@{}", account.username, server.state.config.server.domain);
    server
        .state
        .db
        .add_accounts_to_list(&list_id, &[local_address])
        .await
        .unwrap();

    let root = Status {
        id: "100".to_string(),
        uri: "https://test.example.com/status/list-none-page-root".to_string(),
        content: "<p>Root status</p>".to_string(),
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
    };
    let reply_old = Status {
        id: "200".to_string(),
        uri: "https://test.example.com/status/list-none-page-reply-old".to_string(),
        content: "<p>Reply old</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: Some(root.uri.clone()),
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    let reply_new = Status {
        id: "300".to_string(),
        uri: "https://test.example.com/status/list-none-page-reply-new".to_string(),
        content: "<p>Reply new</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: Some(root.uri.clone()),
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    server.state.db.insert_status(&root).await.unwrap();
    server.state.db.insert_status(&reply_old).await.unwrap();
    server.state.db.insert_status(&reply_new).await.unwrap();

    let response = server
        .client
        .get(server.url(&format!("/api/v1/timelines/list/{}?limit=1", list_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    let ids: Vec<String> = json
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str().map(ToString::to_string))
        .collect();
    assert_eq!(ids, vec![root.id]);
}

#[tokio::test]
async fn test_timeline_with_max_id() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/home?max_id=123456"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should handle max_id parameter
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_timeline_with_since_id() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/timelines/home?since_id=123456"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should handle since_id parameter
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_home_timeline_since_id_filters_older_statuses() {
    use chrono::{Duration, Utc};
    use rustresort::data::{PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let now = Utc::now();
    for (id, seconds_ago) in [("100", 30), ("200", 20), ("300", 10)] {
        server
            .state
            .db
            .insert_status(&Status {
                id: id.to_string(),
                uri: format!("https://test.example.com/status/{}", id),
                content: format!("<p>Status {}</p>", id),
                content_warning: None,
                visibility: StatusVisibility::Public,
                language: Some("en".to_string()),
                account_address: "testuser@test.example.com".to_string(),
                is_local: true,
                in_reply_to_uri: None,
                boost_of_uri: None,
                quote_of_uri: None,
                persisted_reason: PersistedReason::Own,
                created_at: now - Duration::seconds(seconds_ago),
                fetched_at: None,
            })
            .await
            .unwrap();
    }

    let response = server
        .client
        .get(server.url("/api/v1/timelines/home?since_id=100"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let json: Value = response.json().await.unwrap();
    let ids: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(ids.contains(&"200"));
    assert!(ids.contains(&"300"));
    assert!(!ids.contains(&"100"));
}

#[tokio::test]
async fn test_muted_thread_is_hidden_from_public_timeline() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let root = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/thread-root".to_string(),
        content: "<p>Thread root</p>".to_string(),
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
    let reply = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/thread-reply".to_string(),
        content: "<p>Thread reply</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: Some(root.uri.clone()),
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    let other = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/other-thread".to_string(),
        content: "<p>Other thread</p>".to_string(),
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
    server.state.db.insert_status(&root).await.unwrap();
    server.state.db.insert_status(&reply).await.unwrap();
    server.state.db.insert_status(&other).await.unwrap();

    let mute_response = server
        .client
        .post(server.url(&format!("/api/v1/statuses/{}/mute", &reply.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(mute_response.status(), 200);

    let timeline_response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();
    let timeline_status = timeline_response.status();
    let timeline_body = timeline_response.text().await.unwrap();
    assert_eq!(timeline_status, 200, "timeline body: {}", timeline_body);
    let timeline: Value = serde_json::from_str(&timeline_body).unwrap();
    let ids: Vec<String> = timeline
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str().map(ToString::to_string))
        .collect();

    assert!(!ids.contains(&root.id));
    assert!(!ids.contains(&reply.id));
    assert!(ids.contains(&other.id));
}

#[tokio::test]
async fn test_public_timeline_backfills_when_newest_statuses_are_muted() {
    use chrono::{Duration, Utc};
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let base_time = Utc::now();
    let visible_a = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/visible-a".to_string(),
        content: "<p>Visible A</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Own,
        created_at: base_time,
        fetched_at: None,
    };
    let visible_b = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/visible-b".to_string(),
        content: "<p>Visible B</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Own,
        created_at: base_time + Duration::seconds(1),
        fetched_at: None,
    };
    let muted_root = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/status/muted-root".to_string(),
        content: "<p>Muted root</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: rustresort::data::PersistedReason::Own,
        created_at: base_time + Duration::seconds(2),
        fetched_at: None,
    };
    server.state.db.insert_status(&visible_a).await.unwrap();
    server.state.db.insert_status(&visible_b).await.unwrap();
    server.state.db.insert_status(&muted_root).await.unwrap();

    let mut mute_target_id = String::new();
    for index in 0..21 {
        let reply = Status {
            id: EntityId::new_string(),
            uri: format!("https://test.example.com/status/muted-reply-{index}"),
            content: format!("<p>Muted reply {index}</p>"),
            content_warning: None,
            visibility: rustresort::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "testuser@test.example.com".to_string(),
            is_local: true,
            in_reply_to_uri: Some(muted_root.uri.clone()),
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: rustresort::data::PersistedReason::Own,
            created_at: base_time + Duration::seconds((index + 3) as i64),
            fetched_at: None,
        };
        mute_target_id = reply.id.clone();
        server.state.db.insert_status(&reply).await.unwrap();
    }

    let mute_response = server
        .client
        .post(server.url(&format!("/api/v1/statuses/{}/mute", mute_target_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(mute_response.status(), 200);

    let timeline_response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();
    let timeline_status = timeline_response.status();
    let timeline_body = timeline_response.text().await.unwrap();
    assert_eq!(timeline_status, 200, "timeline body: {}", timeline_body);
    let timeline: Value = serde_json::from_str(&timeline_body).unwrap();

    let ids: Vec<String> = timeline
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str().map(ToString::to_string))
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&visible_a.id));
    assert!(ids.contains(&visible_b.id));
    assert!(!ids.contains(&muted_root.id));
    assert!(!ids.contains(&mute_target_id));
}
