//! E2E tests for account operations

mod common;

use common::TestServer;
use rustresort::data::CachedProfile;
use serde_json::Value;

#[tokio::test]
async fn test_verify_credentials_without_auth() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/verify_credentials"))
        .send()
        .await
        .unwrap();

    // Should return 401 Unauthorized without token
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_verify_credentials_with_auth() {
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

    // Should return account info if auth is implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.get("id").is_some());
        assert!(json.get("username").is_some());
        assert!(json.get("uri").is_some());
        assert!(json.get("source").is_some());
    }
}

#[tokio::test]
async fn test_get_account_by_id() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}", account.id)))
        .send()
        .await
        .unwrap();

    // Should return account info
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["username"], "testuser");
    }
}

#[tokio::test]
async fn test_update_credentials() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let update_data = serde_json::json!({
        "display_name": "Updated Name",
        "note": "Updated bio",
        "fields_attributes": {
            "0": {
                "name": "Website",
                "value": "https://example.com/@testuser"
            },
            "1": {
                "name": "Project",
                "value": "RustResort"
            }
        },
        "locked": true,
        "bot": true,
        "discoverable": false,
        "indexable": false
    });

    let response = server
        .client
        .patch(server.url("/api/v1/accounts/update_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&update_data)
        .send()
        .await
        .unwrap();

    // Should update account if implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["display_name"], "Updated Name");
        assert_eq!(json["locked"], true);
        assert_eq!(json["bot"], true);
        assert_eq!(json["discoverable"], false);
        assert_eq!(json["indexable"], false);
        assert_eq!(json["fields"][0]["name"], "Website");
        assert_eq!(
            json["fields"][0]["value"],
            "<a href=\"https://example.com/@testuser\" rel=\"me nofollow noopener noreferrer\" target=\"_blank\">https://example.com/@testuser</a>"
        );
        assert_eq!(
            json["source"]["fields"][0]["value"],
            "https://example.com/@testuser"
        );
        assert_eq!(json["source"]["fields"][1]["value"], "RustResort");
    }
}

#[tokio::test]
async fn test_update_credentials_accepts_form_urlencoded() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .patch(server.url("/api/v1/accounts/update_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(concat!(
            "display_name=Form+Name&",
            "note=Form+bio&",
            "locked=true&",
            "fields_attributes%5B0%5D%5Bname%5D=Website&",
            "fields_attributes%5B0%5D%5Bvalue%5D=https%3A%2F%2Fexample.com%2F%40testuser"
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert_eq!(json["display_name"], "Form Name");
    assert_eq!(json["locked"], true);
    assert_eq!(json["fields"][0]["name"], "Website");
    assert_eq!(
        json["source"]["fields"][0]["value"],
        "https://example.com/@testuser"
    );
}

#[tokio::test]
async fn test_update_credentials_persists_source_defaults_and_preferences() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .patch(server.url("/api/v1/accounts/update_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("source%5Bprivacy%5D=private&source%5Bsensitive%5D=true&source%5Blanguage%5D=ja")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let updated: Value = response.json().await.unwrap();
    assert_eq!(updated["source"]["privacy"], "private");
    assert_eq!(updated["source"]["sensitive"], true);
    assert_eq!(updated["source"]["language"], "ja");

    let verify = server
        .client
        .get(server.url("/api/v1/accounts/verify_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(verify.status(), 200);
    let verify_json: Value = verify.json().await.unwrap();
    assert_eq!(verify_json["source"]["privacy"], "private");
    assert_eq!(verify_json["source"]["sensitive"], true);
    assert_eq!(verify_json["source"]["language"], "ja");

    let preferences = server
        .client
        .get(server.url("/api/v1/preferences"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(preferences.status(), 200);
    let preferences_json: Value = preferences.json().await.unwrap();
    assert_eq!(preferences_json["posting:default:visibility"], "private");
    assert_eq!(preferences_json["posting:default:sensitive"], true);
    assert_eq!(preferences_json["posting:default:language"], "ja");
    assert_eq!(
        preferences_json["posting:default:content_type"],
        "text/plain"
    );
    assert_eq!(preferences_json["notifications:follow"], true);
}

#[tokio::test]
async fn test_follow_locked_remote_account_returns_requested_relationship() {
    use chrono::Utc;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: "alice@remote.example".to_string(),
            uri: "https://remote.example/users/alice".to_string(),
            display_name: Some("Alice".to_string()),
            note: None,
            profile_fields_json: None,
            locked: true,
            bot: false,
            discoverable: true,
            indexable: true,
            avatar_url: None,
            header_url: None,
            public_key_pem: common::test_public_key_pem().to_string(),
            inbox_uri: "https://remote.example/inbox".to_string(),
            outbox_uri: None,
            followers_count: Some(1),
            following_count: Some(1),
            fetched_at: Utc::now(),
        })
        .await;

    let response = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example/follow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["following"], false);
    assert_eq!(body["requested"], true);

    let relationships = server
        .client
        .get(server.url("/api/v1/accounts/relationships?id[]=alice@remote.example"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(relationships.status(), 200);
    let relationships_json: Value = relationships.json().await.unwrap();
    let relationship = relationships_json.as_array().unwrap().first().unwrap();
    assert_eq!(relationship["following"], false);
    assert_eq!(relationship["requested"], true);
}

#[tokio::test]
async fn test_remote_follow_collections_include_known_local_relationships() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow, Follower};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: "alice@remote.example".to_string(),
            uri: "https://remote.example/users/alice".to_string(),
            display_name: Some("Alice".to_string()),
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            avatar_url: None,
            header_url: None,
            public_key_pem: common::test_public_key_pem().to_string(),
            inbox_uri: "https://remote.example/inbox".to_string(),
            outbox_uri: None,
            followers_count: Some(10),
            following_count: Some(20),
            fetched_at: Utc::now(),
        })
        .await;

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: "alice@remote.example".to_string(),
            actor_uri: Some("https://remote.example/users/alice".to_string()),
            uri: "https://test.example.com/users/testuser/follow/known-remote".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    server
        .state
        .db
        .mark_follow_accepted(
            "alice@remote.example",
            "https://remote.example/users/alice",
            Some(443),
        )
        .await
        .unwrap();
    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: "alice@remote.example".to_string(),
            actor_uri: Some("https://remote.example/users/alice".to_string()),
            inbox_uri: "https://remote.example/inbox".to_string(),
            uri: "https://remote.example/follows/local".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let followers = server
        .client
        .get(server.url("/api/v1/accounts/alice@remote.example/followers"))
        .send()
        .await
        .unwrap();
    assert_eq!(followers.status(), 200);
    let followers_json: Value = followers.json().await.unwrap();
    let follower_accounts = followers_json.as_array().unwrap();
    assert_eq!(follower_accounts.len(), 1);
    assert_eq!(follower_accounts[0]["id"], account.id.as_str());

    let following = server
        .client
        .get(server.url("/api/v1/accounts/alice@remote.example/following"))
        .send()
        .await
        .unwrap();
    assert_eq!(following.status(), 200);
    let following_json: Value = following.json().await.unwrap();
    let following_accounts = following_json.as_array().unwrap();
    assert_eq!(following_accounts.len(), 1);
    assert_eq!(following_accounts[0]["id"], account.id.as_str());
}

#[tokio::test]
async fn test_update_credentials_accepts_multipart_form_data() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let form = reqwest::multipart::Form::new()
        .text("display_name", "Multipart Name")
        .text("locked", "true")
        .text("fields_attributes[0][name]", "Project")
        .text("fields_attributes[0][value]", "RustResort");

    let response = server
        .client
        .patch(server.url("/api/v1/accounts/update_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert_eq!(json["display_name"], "Multipart Name");
    assert_eq!(json["locked"], true);
    assert_eq!(json["source"]["fields"][0]["name"], "Project");
    assert_eq!(json["source"]["fields"][0]["value"], "RustResort");
}

#[tokio::test]
async fn test_remote_lookup_returns_cached_profile_fields() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let profile_fields_json = serde_json::to_string(&vec![serde_json::json!({
        "name": "Website",
        "value": "<a href=\"https://alice.example\" rel=\"me\">alice.example</a>",
        "verified_at": serde_json::Value::Null
    })])
    .unwrap();

    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: "alice@remote.example".to_string(),
            uri: "https://remote.example/users/alice".to_string(),
            display_name: Some("Alice".to_string()),
            note: Some("Remote".to_string()),
            profile_fields_json: Some(profile_fields_json),
            locked: true,
            bot: true,
            discoverable: false,
            indexable: false,
            avatar_url: None,
            header_url: None,
            public_key_pem: "-----BEGIN PUBLIC KEY-----\nMIIB\n-----END PUBLIC KEY-----"
                .to_string(),
            inbox_uri: "https://remote.example/users/alice/inbox".to_string(),
            outbox_uri: Some("https://remote.example/users/alice/outbox".to_string()),
            followers_count: Some(3),
            following_count: Some(4),
            fetched_at: chrono::Utc::now(),
        })
        .await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/lookup"))
        .header("Authorization", format!("Bearer {}", token))
        .query(&[("acct", "alice@remote.example")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert_eq!(json["acct"], "alice@remote.example");
    assert_eq!(json["locked"], true);
    assert_eq!(json["bot"], true);
    assert_eq!(json["discoverable"], false);
    assert_eq!(json["indexable"], false);
    assert_eq!(json["fields"][0]["name"], "Website");
    assert_eq!(
        json["fields"][0]["value"],
        "<a href=\"https://alice.example\" rel=\"me\">alice.example</a>"
    );
}

#[tokio::test]
async fn test_update_credentials_sets_moved_to_and_delivers_move() {
    use axum::{extract::State, http::StatusCode, routing::post};
    use rustresort::data::{EntityId, Follower};
    use std::sync::Arc;
    use tokio::{
        net::TcpListener,
        sync::Mutex,
        time::{Duration, sleep},
    };

    async fn record_move(
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
        .route("/users/remote/inbox", post(record_move))
        .with_state(received.clone());
    let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_addr = remote_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(remote_listener, remote_router).await.unwrap();
    });

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: "remote@followers.example".to_string(),
            actor_uri: Some("https://followers.example/users/remote".to_string()),
            inbox_uri: format!("http://{remote_addr}/users/remote/inbox"),
            uri: "https://followers.example/follows/1".to_string(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let moved_to_uri = "https://new.example/users/testuser";
    let response = server
        .client
        .patch(server.url("/api/v1/accounts/update_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "display_name": "Updated Name",
            "moved_to_account_id": moved_to_uri
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["moved"]["uri"], moved_to_uri);

    let updated = server.state.db.get_account().await.unwrap().unwrap();
    assert_eq!(updated.id, account.id);
    assert_eq!(updated.display_name.as_deref(), Some("Updated Name"));
    assert_eq!(updated.moved_to_uri.as_deref(), Some(moved_to_uri));

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

    let delivered = delivered.expect("expected Move delivery to follower inbox");
    assert_eq!(delivered["type"], "Move");
    assert_eq!(delivered["target"], moved_to_uri);
    assert_eq!(delivered["object"], server.public_url("/users/testuser"));
}

#[tokio::test]
async fn test_verify_credentials_includes_moved_account() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let moved_to_uri = "https://new.example/users/testuser";
    server
        .state
        .db
        .patch_account_migration(
            &server.state.db.get_account().await.unwrap().unwrap().id,
            None,
            Some(Some(moved_to_uri)),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/accounts/verify_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert_eq!(json["moved"]["uri"], moved_to_uri);
}

#[tokio::test]
async fn test_update_credentials_invalid_avatar_does_not_apply_profile_changes() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let update_data = serde_json::json!({
        "display_name": "Changed Name",
        "avatar": "not-base64"
    });

    let response = server
        .client
        .patch(server.url("/api/v1/accounts/update_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&update_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);

    let account = server.state.db.get_account().await.unwrap().unwrap();
    assert_eq!(account.display_name.as_deref(), Some("Test User"));
    assert!(account.avatar_s3_key.is_none());
}

#[tokio::test]
async fn test_update_credentials_invalid_header_does_not_apply_avatar_or_profile_changes() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let update_data = serde_json::json!({
        "display_name": "Changed Name",
        "avatar": "UklGRhoAAABXRUJQVlA4TA4AAAAvAAAAEM1VICIC0f+IBA==",
        "header": "data:image/webp,not-base64"
    });

    let response = server
        .client
        .patch(server.url("/api/v1/accounts/update_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&update_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);

    let account = server.state.db.get_account().await.unwrap().unwrap();
    assert_eq!(account.display_name.as_deref(), Some("Test User"));
    assert!(account.avatar_s3_key.is_none());
    assert!(account.header_s3_key.is_none());
}

#[tokio::test]
async fn test_account_statuses() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/statuses", account.id)))
        .send()
        .await
        .unwrap();

    // Should return array of statuses
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_account_statuses_include_pinned_state() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let create_payload = serde_json::json!({
        "status": "pin me in account timeline",
        "visibility": "public"
    });
    let create_response = server
        .client
        .post(server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&create_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), 200);
    let created: Value = create_response.json().await.unwrap();
    let status_id = created["id"].as_str().unwrap();

    let pin_response = server
        .client
        .post(server.url(&format!("/api/v1/statuses/{}/pin", status_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(pin_response.status(), 200);

    let statuses_response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/statuses", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(statuses_response.status(), 200);
    let statuses: Value = statuses_response.json().await.unwrap();
    let items = statuses.as_array().unwrap();

    let pinned_item = items
        .iter()
        .find(|item| item["id"].as_str() == Some(status_id))
        .expect("created status should appear in account statuses");
    assert_eq!(pinned_item["pinned"], true);
}

#[tokio::test]
async fn test_account_statuses_only_media_pages_until_limit() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    use chrono::{Duration, Utc};
    use rustresort::data::{MediaAttachment, Status};

    let now = Utc::now();
    let mut media_status_ids = Vec::new();

    for idx in 0..12 {
        let status_id = format!("{:03}", 120 - idx);
        let status = Status {
            id: status_id.clone(),
            uri: format!("https://test.example.com/status/{}", status_id),
            content: format!("<p>Status {}</p>", idx),
            content_warning: None,
            visibility: rustresort::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "testuser@test.example.com".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: rustresort::data::PersistedReason::Own,
            created_at: now - Duration::seconds(idx as i64),
            fetched_at: None,
        };
        server.state.db.insert_status(&status).await.unwrap();

        if idx >= 10 {
            let media_id = format!("media-{}", status_id);
            let media = MediaAttachment {
                id: media_id,
                status_id: Some(status_id.clone()),
                s3_key: format!("media/{}.webp", status_id),
                thumbnail_s3_key: None,
                content_type: "image/webp".to_string(),
                file_size: 1024,
                description: None,
                blurhash: None,
                width: Some(64),
                height: Some(64),
                focus_x: None,
                focus_y: None,
                created_at: now,
            };
            server.state.db.insert_media(&media).await.unwrap();
            media_status_ids.push(status_id);
        }
    }

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/statuses", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .query(&[("only_media", "true"), ("limit", "2")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let items: Value = response.json().await.unwrap();
    let items = items.as_array().expect("array response expected");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], media_status_ids[0]);
    assert_eq!(items[1]["id"], media_status_ids[1]);
}

#[tokio::test]
async fn test_account_followers() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/followers", account.id)))
        .send()
        .await
        .unwrap();

    // Should return array of followers
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_account_followers_is_public() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/followers", account.id)))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_account_following() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/following", account.id)))
        .send()
        .await
        .unwrap();

    // Should return array of following
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
    }
}

#[tokio::test]
async fn test_account_following_is_public() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/following", account.id)))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_lookup_account_returns_not_found_for_unresolved_remote_account() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/lookup"))
        .header("Authorization", format!("Bearer {}", token))
        .query(&[("acct", "missing-user@missing.example")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn test_follow_account_persists_follow_relationship() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let target = "alice@remote.example";

    let response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/follow", target)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "follow endpoint failed with status {}",
        response.status()
    );

    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert!(follow_addresses.contains(&target.to_string()));
}

#[tokio::test]
async fn test_follow_account_normalizes_address_and_avoids_case_duplicate() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let mixed_case_target = "Alice@Remote.EXAMPLE";

    let first = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/follow", mixed_case_target)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(first.status().is_success());

    let second = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example/follow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(second.status().is_success());

    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert_eq!(follow_addresses.len(), 1);
    assert_eq!(follow_addresses[0], "alice@remote.example");
}

#[tokio::test]
async fn test_follow_account_avoids_default_port_variant_duplicate() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let first = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example:443/follow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(first.status().is_success());

    let second = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example/follow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(second.status().is_success());

    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert_eq!(
        follow_addresses,
        vec!["alice@remote.example:443".to_string()]
    );
}

#[tokio::test]
async fn test_insert_follow_is_idempotent_for_duplicate_target_address() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;

    let first = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example".to_string(),
        actor_uri: None,
        uri: "https://test.example.com/users/testuser/follow/dup-1".to_string(),
        created_at: Utc::now(),
    };
    let second = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example".to_string(),
        actor_uri: None,
        uri: "https://test.example.com/users/testuser/follow/dup-2".to_string(),
        created_at: Utc::now(),
    };

    server.state.db.insert_follow(&first).await.unwrap();
    server.state.db.insert_follow(&second).await.unwrap();

    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert_eq!(follow_addresses, vec!["alice@remote.example".to_string()]);
}

#[tokio::test]
async fn test_unfollow_account_removes_follow_relationship() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let target = "alice@remote.example";

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: target.to_string(),
        actor_uri: None,
        uri: "https://test.example.com/users/testuser/follow/seed".to_string(),
        created_at: Utc::now(),
    };
    server.state.db.insert_follow(&follow).await.unwrap();

    let response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/unfollow", target)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "unfollow endpoint failed with status {}",
        response.status()
    );

    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert!(!follow_addresses.contains(&target.to_string()));
}

#[tokio::test]
async fn test_unfollow_account_matches_case_insensitively() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: "Alice@Remote.EXAMPLE".to_string(),
        actor_uri: None,
        uri: "https://test.example.com/users/testuser/follow/mixed".to_string(),
        created_at: Utc::now(),
    };
    server.state.db.insert_follow(&follow).await.unwrap();

    let response = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example/unfollow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert!(follow_addresses.is_empty());
}

#[tokio::test]
async fn test_unfollow_account_matches_default_https_port_variants() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example:443".to_string(),
        actor_uri: None,
        uri: "https://test.example.com/users/testuser/follow/default-port".to_string(),
        created_at: Utc::now(),
    };
    server.state.db.insert_follow(&follow).await.unwrap();

    let response = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example/unfollow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert!(follow_addresses.is_empty());
}

#[tokio::test]
async fn test_unfollow_account_uses_stored_actor_uri_alias_for_delivery() {
    use axum::{extract::State, http::StatusCode, routing::post};
    use chrono::Utc;
    use rustresort::data::{CachedProfile, EntityId, Follow};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::net::TcpListener;
    use tokio::time::{Duration, sleep};

    async fn record_inbox_delivery(
        State(counter): State<Arc<AtomicUsize>>,
        body: String,
    ) -> StatusCode {
        if let Ok(activity) = serde_json::from_str::<Value>(&body)
            && activity.get("type").and_then(|value| value.as_str()) == Some("Undo")
        {
            counter.fetch_add(1, Ordering::SeqCst);
        }
        StatusCode::ACCEPTED
    }

    let undo_delivery_count = Arc::new(AtomicUsize::new(0));
    let remote_router = axum::Router::new()
        .route("/users/alice/inbox", post(record_inbox_delivery))
        .with_state(undo_delivery_count.clone());
    let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_addr = remote_listener.local_addr().unwrap();
    let remote_base_url = format!("http://{}", remote_addr);

    tokio::spawn(async move {
        axum::serve(remote_listener, remote_router).await.unwrap();
    });

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let target_address = "alice@remote.example";
    let actor_uri = format!("{}/users/alice", remote_base_url);

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: target_address.to_string(),
            actor_uri: Some(actor_uri.clone()),
            uri: "https://test.example.com/users/testuser/follow/seed".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    // Only cache the canonical actor URI alias, not the account address.
    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: actor_uri.clone(),
            uri: actor_uri.clone(),
            display_name: Some("Alice".to_string()),
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
            inbox_uri: format!("{}/users/alice/inbox", remote_base_url),
            outbox_uri: None,
            followers_count: None,
            following_count: None,
            fetched_at: Utc::now(),
        })
        .await;

    let response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/unfollow", target_address)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let mut undo_delivered = false;
    for _ in 0..600 {
        if undo_delivery_count.load(Ordering::SeqCst) > 0 {
            undo_delivered = true;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(undo_delivered, "expected outbound Undo(Follow) delivery");
}

#[tokio::test]
async fn test_follow_account_rejects_self_follow() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/follow", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert!(follow_addresses.is_empty());
}

#[tokio::test]
async fn test_follow_account_rejects_self_follow_case_insensitive_address() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url("/api/v1/accounts/TESTUSER@TEST.EXAMPLE.COM/follow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert!(follow_addresses.is_empty());
}

#[tokio::test]
async fn test_follow_account_rejects_self_follow_with_default_https_port() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url("/api/v1/accounts/testuser@test.example.com:443/follow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert!(follow_addresses.is_empty());
}

#[tokio::test]
async fn test_follow_account_preserves_explicit_port_in_target_address() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example:443/follow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert_eq!(
        follow_addresses,
        vec!["alice@remote.example:443".to_string()]
    );
}

#[tokio::test]
async fn test_follow_account_preserves_explicit_non_default_port_in_target_address() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example:80/follow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert_eq!(
        follow_addresses,
        vec!["alice@remote.example:80".to_string()]
    );
}

#[tokio::test]
async fn test_block_account_matches_default_https_port_variants() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example:443".to_string(),
        actor_uri: None,
        uri: "https://test.example.com/users/testuser/follow/block-default-port".to_string(),
        created_at: Utc::now(),
    };
    server.state.db.insert_follow(&follow).await.unwrap();

    let block_response = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example/block"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(block_response.status().is_success());

    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert!(follow_addresses.is_empty());
    assert!(
        server
            .state
            .db
            .is_account_blocked("alice@remote.example:443", Some(443))
            .await
            .unwrap()
    );

    let unblock_response = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example:443/unblock"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(unblock_response.status().is_success());
    assert!(
        !server
            .state
            .db
            .is_account_blocked("alice@remote.example", Some(443))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_block_and_unblock_account_deliver_outbound_activities() {
    use axum::{extract::State, http::StatusCode, routing::post};
    use chrono::Utc;
    use rustresort::data::CachedProfile;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::net::TcpListener;
    use tokio::time::{Duration, sleep};

    #[derive(Clone)]
    struct InboxCounters {
        blocks: Arc<AtomicUsize>,
        undo_blocks: Arc<AtomicUsize>,
    }

    async fn record_inbox_delivery(
        State(counters): State<InboxCounters>,
        body: String,
    ) -> StatusCode {
        if let Ok(activity) = serde_json::from_str::<Value>(&body) {
            match activity.get("type").and_then(|value| value.as_str()) {
                Some("Block") => {
                    counters.blocks.fetch_add(1, Ordering::SeqCst);
                }
                Some("Undo")
                    if activity
                        .get("object")
                        .and_then(|value| value.get("type"))
                        .and_then(|value| value.as_str())
                        == Some("Block") =>
                {
                    counters.undo_blocks.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
        StatusCode::ACCEPTED
    }

    let counters = InboxCounters {
        blocks: Arc::new(AtomicUsize::new(0)),
        undo_blocks: Arc::new(AtomicUsize::new(0)),
    };
    let remote_router = axum::Router::new()
        .route("/users/alice/inbox", post(record_inbox_delivery))
        .with_state(counters.clone());
    let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_addr = remote_listener.local_addr().unwrap();
    let remote_base_url = format!("http://{}", remote_addr);

    tokio::spawn(async move {
        axum::serve(remote_listener, remote_router).await.unwrap();
    });

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let target_address = "alice@remote.example";

    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: target_address.to_string(),
            uri: format!("{}/users/alice", remote_base_url),
            display_name: Some("Alice".to_string()),
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
            inbox_uri: format!("{}/users/alice/inbox", remote_base_url),
            outbox_uri: None,
            followers_count: None,
            following_count: None,
            fetched_at: Utc::now(),
        })
        .await;

    let block_response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/block", target_address)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(block_response.status().is_success());

    let mut block_delivered = false;
    for _ in 0..600 {
        if counters.blocks.load(Ordering::SeqCst) > 0 {
            block_delivered = true;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(block_delivered, "expected outbound Block delivery");

    let unblock_response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/unblock", target_address)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(unblock_response.status().is_success());

    let mut undo_delivered = false;
    for _ in 0..600 {
        if counters.undo_blocks.load(Ordering::SeqCst) > 0 {
            undo_delivered = true;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(undo_delivered, "expected outbound Undo(Block) delivery");
}

#[tokio::test]
async fn test_block_and_unblock_use_stored_actor_uri_alias_for_delivery() {
    use axum::{extract::State, http::StatusCode, routing::post};
    use chrono::Utc;
    use rustresort::data::{CachedProfile, EntityId, Follow};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::net::TcpListener;
    use tokio::time::{Duration, sleep};

    #[derive(Clone)]
    struct InboxCounters {
        blocks: Arc<AtomicUsize>,
        undo_blocks: Arc<AtomicUsize>,
    }

    async fn record_inbox_delivery(
        State(counters): State<InboxCounters>,
        body: String,
    ) -> StatusCode {
        if let Ok(activity) = serde_json::from_str::<Value>(&body) {
            match activity.get("type").and_then(|value| value.as_str()) {
                Some("Block") => {
                    counters.blocks.fetch_add(1, Ordering::SeqCst);
                }
                Some("Undo")
                    if activity
                        .get("object")
                        .and_then(|value| value.get("type"))
                        .and_then(|value| value.as_str())
                        == Some("Block") =>
                {
                    counters.undo_blocks.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
        StatusCode::ACCEPTED
    }

    let counters = InboxCounters {
        blocks: Arc::new(AtomicUsize::new(0)),
        undo_blocks: Arc::new(AtomicUsize::new(0)),
    };
    let remote_router = axum::Router::new()
        .route("/users/alice/inbox", post(record_inbox_delivery))
        .with_state(counters.clone());
    let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_addr = remote_listener.local_addr().unwrap();
    let remote_base_url = format!("http://{}", remote_addr);
    let actor_uri = format!("{}/users/alice", remote_base_url);

    tokio::spawn(async move {
        axum::serve(remote_listener, remote_router).await.unwrap();
    });

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let target_address = "alice@remote.example";

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: target_address.to_string(),
            actor_uri: Some(actor_uri.clone()),
            uri: "https://test.example.com/users/testuser/follow/block-alias".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    // Only cache the actor URI alias, not the account address.
    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: actor_uri.clone(),
            uri: actor_uri.clone(),
            display_name: Some("Alice".to_string()),
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
            inbox_uri: format!("{}/users/alice/inbox", remote_base_url),
            outbox_uri: None,
            followers_count: None,
            following_count: None,
            fetched_at: Utc::now(),
        })
        .await;

    let block_response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/block", target_address)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(block_response.status().is_success());

    let mut block_delivered = false;
    for _ in 0..600 {
        if counters.blocks.load(Ordering::SeqCst) > 0 {
            block_delivered = true;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(block_delivered, "expected outbound Block delivery");

    let unblock_response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/unblock", target_address)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(unblock_response.status().is_success());

    let mut undo_delivered = false;
    for _ in 0..600 {
        if counters.undo_blocks.load(Ordering::SeqCst) > 0 {
            undo_delivered = true;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(undo_delivered, "expected outbound Undo(Block) delivery");
}

#[tokio::test]
async fn test_block_account_when_already_blocked_skips_duplicate_outbound_delivery() {
    use axum::{extract::State, http::StatusCode, routing::post};
    use chrono::Utc;
    use rustresort::data::CachedProfile;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::net::TcpListener;
    use tokio::time::{Duration, sleep};

    async fn record_inbox_delivery(
        State(counter): State<Arc<AtomicUsize>>,
        body: String,
    ) -> StatusCode {
        if let Ok(activity) = serde_json::from_str::<Value>(&body)
            && activity.get("type").and_then(|value| value.as_str()) == Some("Block")
        {
            counter.fetch_add(1, Ordering::SeqCst);
        }
        StatusCode::ACCEPTED
    }

    let block_delivery_count = Arc::new(AtomicUsize::new(0));
    let remote_router = axum::Router::new()
        .route("/users/alice/inbox", post(record_inbox_delivery))
        .with_state(block_delivery_count.clone());
    let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_addr = remote_listener.local_addr().unwrap();
    let remote_base_url = format!("http://{}", remote_addr);

    tokio::spawn(async move {
        axum::serve(remote_listener, remote_router).await.unwrap();
    });

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let target_address = "alice@remote.example";

    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: target_address.to_string(),
            uri: format!("{}/users/alice", remote_base_url),
            display_name: Some("Alice".to_string()),
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
            inbox_uri: format!("{}/users/alice/inbox", remote_base_url),
            outbox_uri: None,
            followers_count: None,
            following_count: None,
            fetched_at: Utc::now(),
        })
        .await;

    let first_response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/block", target_address)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(first_response.status().is_success());

    let mut first_delivery_observed = false;
    for _ in 0..600 {
        if block_delivery_count.load(Ordering::SeqCst) > 0 {
            first_delivery_observed = true;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(
        first_delivery_observed,
        "expected initial outbound Block delivery"
    );

    sleep(Duration::from_millis(100)).await;
    let before_second_block = block_delivery_count.load(Ordering::SeqCst);

    let second_response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/block", target_address)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(second_response.status().is_success());

    sleep(Duration::from_millis(300)).await;
    assert_eq!(
        block_delivery_count.load(Ordering::SeqCst),
        before_second_block,
        "unexpected duplicate outbound Block delivery"
    );
}

#[tokio::test]
async fn test_unblock_account_without_existing_block_skips_outbound_undo_delivery() {
    use axum::{extract::State, http::StatusCode, routing::post};
    use chrono::Utc;
    use rustresort::data::CachedProfile;
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
    let target_address = "alice@remote.example";

    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: target_address.to_string(),
            uri: format!("{}/users/alice", remote_base_url),
            display_name: Some("Alice".to_string()),
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
            inbox_uri: format!("{}/users/alice/inbox", remote_base_url),
            outbox_uri: None,
            followers_count: None,
            following_count: None,
            fetched_at: Utc::now(),
        })
        .await;

    let unblock_response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/unblock", target_address)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(unblock_response.status().is_success());

    sleep(Duration::from_millis(300)).await;
    assert_eq!(
        inbox_delivery_count.load(Ordering::SeqCst),
        0,
        "unexpected outbound delivery for unblock without existing block"
    );
}

#[tokio::test]
async fn test_mute_account_matches_default_https_port_variants() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let mute_response = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example:443/mute"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(mute_response.status().is_success());
    assert!(
        server
            .state
            .db
            .is_account_muted("alice@remote.example", Some(443))
            .await
            .unwrap()
    );

    let unmute_response = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example/unmute"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(unmute_response.status().is_success());
    assert!(
        !server
            .state
            .db
            .is_account_muted("alice@remote.example:443", Some(443))
            .await
            .unwrap()
    );
}
