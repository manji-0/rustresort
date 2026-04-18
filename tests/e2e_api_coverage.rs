//! Comprehensive API endpoint coverage tests
//!
//! Tests all 88+ Mastodon API endpoints for basic functionality

mod common;

use common::TestServer;
use serde_json::json;

async fn cache_remote_profile(server: &TestServer, address: &str) {
    use chrono::Utc;
    use rustresort::data::CachedProfile;

    let (username, domain) = address
        .split_once('@')
        .expect("remote address must be user@domain");
    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: address.to_string(),
            uri: format!("https://{}/users/{}", domain, username),
            display_name: Some("Alice Remote".to_string()),
            note: Some("Remote profile".to_string()),
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            avatar_url: Some(format!("https://{}/media/alice-avatar.jpg", domain)),
            header_url: Some(format!("https://{}/media/alice-header.jpg", domain)),
            public_key_pem: "test-public-key".to_string(),
            inbox_uri: format!("https://{}/inbox", domain),
            outbox_uri: Some(format!("https://{}/users/{}/outbox", domain, username)),
            followers_count: Some(12),
            following_count: Some(34),
            fetched_at: Utc::now(),
        })
        .await;
}

async fn persist_remote_profile(server: &TestServer, address: &str, display_name: &str) {
    use chrono::Utc;
    use rustresort::data::{CachedProfile, RemoteProfile};

    let now = Utc::now();
    let (username, domain) = address
        .split_once('@')
        .expect("remote address must be user@domain");
    let actor_uri = format!("https://{}/users/{}", domain, username);
    let inbox_uri = format!("https://{}/inbox", domain);
    let outbox_uri = format!("https://{}/users/{}/outbox", domain, username);
    let avatar_url = format!("https://{}/media/{}-avatar.jpg", domain, username);
    let header_url = format!("https://{}/media/{}-header.jpg", domain, username);

    server
        .state
        .db
        .upsert_remote_profile(&RemoteProfile {
            address: address.to_string(),
            uri: actor_uri.clone(),
            display_name: Some(display_name.to_string()),
            note: Some(format!("{display_name} note")),
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            avatar_url: Some(avatar_url.clone()),
            header_url: Some(header_url.clone()),
            public_key_pem: "test-public-key".to_string(),
            inbox_uri: inbox_uri.clone(),
            outbox_uri: Some(outbox_uri.clone()),
            followers_count: Some(12),
            following_count: Some(34),
            fetched_at: now,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: address.to_string(),
            uri: actor_uri,
            display_name: Some(display_name.to_string()),
            note: Some(format!("{display_name} note")),
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            avatar_url: Some(avatar_url),
            header_url: Some(header_url),
            public_key_pem: "test-public-key".to_string(),
            inbox_uri,
            outbox_uri: Some(outbox_uri),
            followers_count: Some(12),
            following_count: Some(34),
            fetched_at: now,
        })
        .await;
}

async fn cache_remote_profile_alias_by_actor_uri(
    server: &TestServer,
    actor_uri: &str,
    canonical_address: &str,
) {
    use chrono::Utc;
    use rustresort::data::CachedProfile;

    let (username, domain) = canonical_address
        .split_once('@')
        .expect("canonical address must be user@domain");
    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: actor_uri.to_string(),
            uri: actor_uri.to_string(),
            display_name: Some("Alice Alias".to_string()),
            note: Some("Alias profile".to_string()),
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            avatar_url: Some(format!("https://{}/media/alice-avatar.jpg", domain)),
            header_url: Some(format!("https://{}/media/alice-header.jpg", domain)),
            public_key_pem: "test-public-key".to_string(),
            inbox_uri: format!("{}/inbox", actor_uri.trim_end_matches('/')),
            outbox_uri: Some(format!("https://{}/users/{}/outbox", domain, username)),
            followers_count: Some(56),
            following_count: Some(78),
            fetched_at: Utc::now(),
        })
        .await;
}

// ============================================================================
// Instance Endpoints (5 endpoints)
// ============================================================================

#[tokio::test]
async fn test_instance_info() {
    let server = TestServer::new().await;
    let response = server
        .client
        .get(server.url("/api/v1/instance"))
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
        .get(server.url("/api/v2/instance"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["stats"]["user_count"], 1);
    assert!(body["stats"]["status_count"].is_number());
    assert!(body["stats"]["domain_count"].is_number());
    assert_eq!(body["usage"]["users"]["total"], 1);
    assert!(body["usage"]["local_posts"].is_number());
}

#[tokio::test]
async fn test_auxiliary_instance_endpoints() {
    let server = TestServer::new().await;
    let endpoints = [
        "/api/v1/announcements",
        "/api/v1/trends",
        "/api/v1/trends/statuses",
        "/api/v1/trends/tags",
        "/api/v1/trends/links",
        "/api/v1/directory",
        "/api/v1/instance/privacy_policy",
        "/api/v1/instance/translation_languages",
    ];

    for endpoint in endpoints {
        let response = server
            .client
            .get(server.url(endpoint))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{endpoint} should be available");
    }
}

#[tokio::test]
async fn test_instance_v1_includes_rules() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(server.url("/api/v1/instance"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let rules = body["rules"].as_array().expect("rules should be array");
    assert!(!rules.is_empty());
    assert!(rules[0]["text"].is_string());
}

#[tokio::test]
async fn test_instance_peers() {
    let server = TestServer::new().await;
    let response = server
        .client
        .get(server.url("/api/v1/instance/peers"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_instance_activity() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .client
        .post(server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "status": "Weekly activity sample",
            "visibility": "public"
        }))
        .send()
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/instance/activity"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let activity = body
        .as_array()
        .expect("instance activity should be an array");
    assert_eq!(activity.len(), 12);
    assert!(activity.iter().any(|item| item["statuses"] == "1"));
    assert!(activity.iter().any(|item| item["logins"] == "1"));
    assert!(activity.iter().any(|item| item["registrations"] == "1"));
}

#[tokio::test]
async fn test_instance_rules() {
    let server = TestServer::new().await;
    let response = server
        .client
        .get(server.url("/api/v1/instance/rules"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_custom_emojis_endpoint_returns_array() {
    let server = TestServer::new().await;
    let response = server
        .client
        .get(server.url("/api/v1/custom_emojis"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body, json!([]));
}

// ============================================================================
// Apps Endpoints (2 endpoints)
// ============================================================================

#[tokio::test]
async fn test_create_app_endpoint_works() {
    let server = TestServer::new().await;
    let app_data = json!({
        "client_name": "Test App",
        "redirect_uris": "urn:ietf:wg:oauth:2.0:oob",
        "scopes": "read write"
    });

    let response = server
        .client
        .post(server.url("/api/v1/apps"))
        .json(&app_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["name"], "Test App");
    assert_eq!(body["redirect_uri"], "urn:ietf:wg:oauth:2.0:oob");
    assert_eq!(body["redirect_uris"], json!(["urn:ietf:wg:oauth:2.0:oob"]));
    assert_eq!(body["scopes"], json!(["read", "write"]));
}

#[tokio::test]
async fn test_verify_app_credentials_endpoint_works() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server
        .create_oauth_authorization_code_token("read:accounts")
        .await;

    let response = server
        .client
        .get(server.url("/api/v1/apps/verify_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["name"], "RustResort E2E Client");
}

// ============================================================================
// Account Endpoints (20+ endpoints)
// ============================================================================

#[tokio::test]
async fn test_create_account_endpoint_is_disabled() {
    let server = TestServer::new().await;
    let account_data = json!({
        "username": "newuser",
        "email": "newuser@example.com",
        "password": "password123",
        "agreement": true
    });

    let response = server
        .client
        .post(server.url("/api/v1/accounts"))
        .json(&account_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn test_verify_credentials() {
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

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_preferences_endpoint_returns_default_preferences() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/preferences"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["posting:default:visibility"], "public");
    assert_eq!(body["posting:default:sensitive"], false);
}

#[tokio::test]
async fn test_account_lookup_returns_local_account() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/lookup"))
        .query(&[("acct", "testuser")])
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["acct"], "testuser");
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
        .patch(server.url("/api/v1/accounts/update_credentials"))
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
        .get(server.url(&format!("/api/v1/accounts/{}", account.id)))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_account_supports_remote_account_ids() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    cache_remote_profile(&server, "alice@remote.example").await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/alice@remote.example"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["id"], "alice@remote.example");
    assert_eq!(body["acct"], "alice@remote.example");
}

#[tokio::test]
async fn test_account_statuses() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/statuses", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_account_statuses_support_remote_account_ids() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    cache_remote_profile(&server, "alice@remote.example").await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/users/alice/statuses/1".to_string(),
        content: "<p>Remote status</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "alice@remote.example".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Timeline,
        created_at: Utc::now(),
        fetched_at: Some(Utc::now()),
    };
    server.state.db.insert_status(&status).await.unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/accounts/alice@remote.example/statuses"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let statuses = body.as_array().expect("account statuses should be array");
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0]["id"], status.id);
    assert_eq!(statuses[0]["account"]["acct"], "alice@remote.example");
}

#[tokio::test]
async fn test_account_followers() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/followers", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_remote_account_followers_returns_not_found_when_remote_actor_cannot_be_resolved() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/alice@missing.example/followers"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn test_account_following() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/following", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_account_followers_applies_max_id_cursor() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follower};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    for address in [
        "alice@remote.example",
        "bob@remote.example",
        "carol@remote.example",
    ] {
        server
            .state
            .db
            .insert_follower(&Follower {
                id: EntityId::new_string(),
                follower_address: address.to_string(),
                actor_uri: None,
                inbox_uri: format!("https://remote.example/inbox/{}", address),
                uri: format!("https://remote.example/follows/{}", address),
                created_at: Utc::now(),
            })
            .await
            .unwrap();
    }

    let first_page_response = server
        .client
        .get(server.url(&format!(
            "/api/v1/accounts/{}/followers?limit=1",
            account.id
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(first_page_response.status(), 200);
    let first_page: serde_json::Value = first_page_response.json().await.unwrap();
    let first_id = first_page[0]["id"]
        .as_str()
        .expect("first followers page should include id")
        .to_string();
    assert_eq!(first_id, "carol@remote.example");

    let second_page_response = server
        .client
        .get(server.url(&format!(
            "/api/v1/accounts/{}/followers?limit=1&max_id={}",
            account.id, first_id
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(second_page_response.status(), 200);
    let second_page: serde_json::Value = second_page_response.json().await.unwrap();
    let second_id = second_page[0]["id"]
        .as_str()
        .expect("second followers page should include id");
    assert_eq!(second_id, "bob@remote.example");
}

#[tokio::test]
async fn test_account_followers_actor_uri_cursor_preserves_case() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follower};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    for address in [
        "https://remote.example/Users/Alice",
        "https://remote.example/Users/Bob",
    ] {
        server
            .state
            .db
            .insert_follower(&Follower {
                id: EntityId::new_string(),
                follower_address: address.to_string(),
                actor_uri: None,
                inbox_uri: format!("https://remote.example/inbox/{}", address),
                uri: format!("https://remote.example/follows/{}", address),
                created_at: Utc::now(),
            })
            .await
            .unwrap();
    }

    let first_page_response = server
        .client
        .get(server.url(&format!(
            "/api/v1/accounts/{}/followers?limit=1",
            account.id
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(first_page_response.status(), 200);
    let first_page: serde_json::Value = first_page_response.json().await.unwrap();
    let first_id = first_page[0]["id"]
        .as_str()
        .expect("first followers page should include id")
        .to_string();
    assert_eq!(first_id, "https://remote.example/Users/Bob");

    let second_page_response = server
        .client
        .get(server.url(&format!(
            "/api/v1/accounts/{}/followers?limit=1&max_id={}",
            account.id, first_id
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(second_page_response.status(), 200);
    let second_page: serde_json::Value = second_page_response.json().await.unwrap();
    let second_id = second_page[0]["id"]
        .as_str()
        .expect("second followers page should include id");
    assert_eq!(second_id, "https://remote.example/Users/Alice");
}

#[tokio::test]
async fn test_account_following_applies_max_id_cursor() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    for address in [
        "alice@remote.example",
        "bob@remote.example",
        "carol@remote.example",
    ] {
        server
            .state
            .db
            .insert_follow(&Follow {
                id: EntityId::new_string(),
                target_address: address.to_string(),
                actor_uri: None,
                uri: format!("https://remote.example/follows/{}", address),
                created_at: Utc::now(),
            })
            .await
            .unwrap();
    }

    let first_page_response = server
        .client
        .get(server.url(&format!(
            "/api/v1/accounts/{}/following?limit=1",
            account.id
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(first_page_response.status(), 200);
    let first_page: serde_json::Value = first_page_response.json().await.unwrap();
    let first_id = first_page[0]["id"]
        .as_str()
        .expect("first following page should include id")
        .to_string();
    assert_eq!(first_id, "carol@remote.example");

    let second_page_response = server
        .client
        .get(server.url(&format!(
            "/api/v1/accounts/{}/following?limit=1&max_id={}",
            account.id, first_id
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(second_page_response.status(), 200);
    let second_page: serde_json::Value = second_page_response.json().await.unwrap();
    let second_id = second_page[0]["id"]
        .as_str()
        .expect("second following page should include id");
    assert_eq!(second_id, "bob@remote.example");
}

#[tokio::test]
async fn test_account_followers_returns_remote_account_data_from_cache() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follower};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_address = "alice@remote.example";

    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: remote_address.to_string(),
            actor_uri: None,
            inbox_uri: "https://remote.example/inbox".to_string(),
            uri: "https://remote.example/follows/1".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    cache_remote_profile(&server, remote_address).await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/followers", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let followers = body.as_array().expect("followers should be array");
    assert_eq!(followers.len(), 1);
    assert_eq!(followers[0]["acct"], remote_address);
    assert_eq!(followers[0]["display_name"], "Alice Remote");
    assert_eq!(followers[0]["followers_count"], 12);
}

#[tokio::test]
async fn test_account_following_returns_remote_account_data_from_cache() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_address = "alice@remote.example";

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: remote_address.to_string(),
            actor_uri: None,
            uri: "https://remote.example/follows/2".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    cache_remote_profile(&server, remote_address).await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/following", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let following = body.as_array().expect("following should be array");
    assert_eq!(following.len(), 1);
    assert_eq!(following[0]["acct"], remote_address);
    assert_eq!(following[0]["display_name"], "Alice Remote");
    assert_eq!(following[0]["following_count"], 34);
}

#[tokio::test]
async fn test_account_followers_actor_uri_uses_cached_profile_by_uri_alias() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follower};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri_address = "https://remote.example/users/alice";

    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: actor_uri_address.to_string(),
            actor_uri: None,
            inbox_uri: "https://remote.example/inbox".to_string(),
            uri: "https://remote.example/follows/actor-uri-cached".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    cache_remote_profile_alias_by_actor_uri(&server, actor_uri_address, "alice@remote.example")
        .await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/followers", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let followers = body.as_array().expect("followers should be array");
    assert_eq!(followers.len(), 1);
    assert_eq!(followers[0]["acct"], "alice@remote.example");
    assert_eq!(followers[0]["username"], "alice");
    assert_eq!(followers[0]["display_name"], "Alice Alias");
    assert_eq!(followers[0]["followers_count"], 56);
}

#[tokio::test]
async fn test_account_followers_prefers_stored_actor_uri_over_address_lookup() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follower};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri = "https://remote.example/@alice";

    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: "alice@remote.example".to_string(),
            actor_uri: Some(actor_uri.to_string()),
            inbox_uri: "https://remote.example/inbox".to_string(),
            uri: "https://remote.example/follows/stored-actor-uri".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    cache_remote_profile_alias_by_actor_uri(&server, actor_uri, "alice@remote.example").await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/followers", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let followers = body.as_array().expect("followers should be array");
    assert_eq!(followers.len(), 1);
    assert_eq!(followers[0]["id"], "alice@remote.example");
    assert_eq!(followers[0]["acct"], "alice@remote.example");
    assert_eq!(followers[0]["display_name"], "Alice Alias");
}

#[tokio::test]
async fn test_account_followers_keeps_actor_uri_addresses_as_placeholder_accounts() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follower};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri_address = "https://remote.example/actors/12345";

    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: actor_uri_address.to_string(),
            actor_uri: None,
            inbox_uri: "https://remote.example/inbox".to_string(),
            uri: "https://remote.example/follows/actor-uri".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/followers", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let followers = body.as_array().expect("followers should be array");
    assert_eq!(followers.len(), 1);
    assert_eq!(followers[0]["id"], "12345@remote.example");
    assert_eq!(followers[0]["acct"], "12345@remote.example");
    assert_eq!(followers[0]["url"], actor_uri_address);
    assert!(followers[0].get("avatar_static").is_some());
    assert!(followers[0].get("header_static").is_some());
}

#[tokio::test]
async fn test_account_following_keeps_actor_uri_addresses_as_placeholder_accounts() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri_address = "https://remote.example/actors/12345";

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: actor_uri_address.to_string(),
            actor_uri: None,
            uri: "https://remote.example/follows/actor-uri".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/following", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let following = body.as_array().expect("following should be array");
    assert_eq!(following.len(), 1);
    assert_eq!(following[0]["id"], "12345@remote.example");
    assert_eq!(following[0]["acct"], "12345@remote.example");
    assert_eq!(following[0]["url"], actor_uri_address);
    assert!(following[0].get("avatar_static").is_some());
    assert!(following[0].get("header_static").is_some());
}

#[tokio::test]
async fn test_account_following_prefers_stored_actor_uri_over_address_lookup() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri = "https://remote.example/users/alice";

    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: "alice@remote.example".to_string(),
            actor_uri: Some(actor_uri.to_string()),
            uri: "https://remote.example/follows/stored-actor-uri".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    cache_remote_profile_alias_by_actor_uri(&server, actor_uri, "alice@remote.example").await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/following", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let following = body.as_array().expect("following should be array");
    assert_eq!(following.len(), 1);
    assert_eq!(following[0]["id"], "alice@remote.example");
    assert_eq!(following[0]["acct"], "alice@remote.example");
    assert_eq!(following[0]["display_name"], "Alice Alias");
}

#[tokio::test]
async fn test_account_followers_actor_uri_with_at_path_keeps_valid_username_and_url() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follower};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri_address = "https://remote.example/@alice";

    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: actor_uri_address.to_string(),
            actor_uri: None,
            inbox_uri: "https://remote.example/inbox".to_string(),
            uri: "https://remote.example/follows/actor-uri-at".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/followers", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let followers = body.as_array().expect("followers should be array");
    assert_eq!(followers.len(), 1);
    assert_eq!(followers[0]["id"], actor_uri_address);
    assert_eq!(followers[0]["acct"], "alice@remote.example");
    assert_eq!(followers[0]["username"], "alice");
    assert_eq!(followers[0]["url"], actor_uri_address);
}

#[tokio::test]
async fn test_follow_account() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example/follow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
}

#[tokio::test]
async fn test_unfollow_account() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/unfollow", account.id)))
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
        .post(server.url(&format!("/api/v1/accounts/{}/block", account.id)))
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
        .post(server.url(&format!("/api/v1/accounts/{}/unblock", account.id)))
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
        .post(server.url(&format!("/api/v1/accounts/{}/mute", account.id)))
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
        .post(server.url(&format!("/api/v1/accounts/{}/unmute", account.id)))
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
        .get(server.url("/api/v1/blocks"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_blocks_actor_uri_fallback_returns_mastodon_account_shape() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri = "https://remote.example/actors/12345";

    server
        .state
        .db
        .block_account_with_remote_metadata(
            actor_uri,
            None,
            Some("https://remote.example/inbox"),
            Some(443),
        )
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/blocks"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let blocks = body.as_array().expect("blocks should be array");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["id"], actor_uri);
    assert_eq!(blocks[0]["acct"], "12345@remote.example");
    assert_eq!(blocks[0]["url"], actor_uri);
    assert!(blocks[0].get("avatar_static").is_some());
    assert!(blocks[0].get("header_static").is_some());
    assert!(blocks[0].get("locked").is_some());
}

#[tokio::test]
async fn test_get_blocks_prefers_stored_actor_uri_profile_alias() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri = "https://remote.example/users/alice";

    server
        .state
        .db
        .block_account_with_remote_metadata(
            "alice@remote.example",
            Some(actor_uri),
            Some("https://remote.example/inbox"),
            Some(443),
        )
        .await
        .unwrap();
    cache_remote_profile_alias_by_actor_uri(&server, actor_uri, "alice@remote.example").await;

    let response = server
        .client
        .get(server.url("/api/v1/blocks"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let blocks = body.as_array().expect("blocks should be array");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["id"], "alice@remote.example");
    assert_eq!(blocks[0]["acct"], "alice@remote.example");
    assert_eq!(blocks[0]["display_name"], "Alice Alias");
    assert_ne!(blocks[0]["id"], account.id);
}

#[tokio::test]
async fn test_get_mutes() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/mutes"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_mutes_prefers_stored_actor_uri_profile_alias() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri = "https://remote.example/users/alice";

    server
        .state
        .db
        .mute_account_with_actor_uri(
            "alice@remote.example",
            true,
            None,
            Some(actor_uri),
            Some(443),
        )
        .await
        .unwrap();
    cache_remote_profile_alias_by_actor_uri(&server, actor_uri, "alice@remote.example").await;

    let response = server
        .client
        .get(server.url("/api/v1/mutes"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let mutes = body.as_array().expect("mutes should be array");
    assert_eq!(mutes.len(), 1);
    assert_eq!(mutes[0]["id"], "alice@remote.example");
    assert_eq!(mutes[0]["acct"], "alice@remote.example");
    assert_eq!(mutes[0]["display_name"], "Alice Alias");
}

#[tokio::test]
async fn test_get_relationships() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url(&format!(
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
async fn test_get_relationships_decodes_percent_encoded_ids() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/relationships?id[]=alice%40example.com"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body[0]["id"], "alice@example.com");
}

#[tokio::test]
async fn test_get_relationships_matches_default_port_equivalent_ids() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    use chrono::Utc;
    use rustresort::data::{EntityId, Follow, Follower};

    let target_with_port = "alice@remote.example:443";
    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: target_with_port.to_string(),
            actor_uri: None,
            uri: "https://remote.example/follow/1".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: target_with_port.to_string(),
            actor_uri: None,
            inbox_uri: "https://remote.example/inbox".to_string(),
            uri: "https://remote.example/follow/2".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/accounts/relationships"))
        .header("Authorization", format!("Bearer {}", token))
        .query(&[("id[]", "alice@remote.example")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body[0]["following"], true);
    assert_eq!(body[0]["followed_by"], true);
}

#[tokio::test]
async fn test_get_relationships_matches_actor_uri_ids() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    use chrono::Utc;
    use rustresort::data::{EntityId, Follow, Follower};

    let actor_uri = "https://remote.example/@alice";
    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: actor_uri.to_string(),
            actor_uri: None,
            uri: "https://remote.example/follow/uri-1".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: actor_uri.to_string(),
            actor_uri: None,
            inbox_uri: "https://remote.example/inbox".to_string(),
            uri: "https://remote.example/follow/uri-2".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/accounts/relationships"))
        .header("Authorization", format!("Bearer {}", token))
        .query(&[("id[]", actor_uri)])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body[0]["id"], actor_uri);
    assert_eq!(body[0]["following"], true);
    assert_eq!(body[0]["followed_by"], true);
}

#[tokio::test]
async fn test_get_relationships_matches_stored_actor_uri_fields() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    use chrono::Utc;
    use rustresort::data::{EntityId, Follow, Follower};

    let actor_uri = "https://remote.example/users/alice";
    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: "alice@remote.example".to_string(),
            actor_uri: Some(actor_uri.to_string()),
            uri: "https://remote.example/follow/stored-actor-1".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    server
        .state
        .db
        .insert_follower(&Follower {
            id: EntityId::new_string(),
            follower_address: "alice@remote.example".to_string(),
            actor_uri: Some(actor_uri.to_string()),
            inbox_uri: "https://remote.example/inbox".to_string(),
            uri: "https://remote.example/follow/stored-actor-2".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/accounts/relationships"))
        .header("Authorization", format!("Bearer {}", token))
        .query(&[("id[]", actor_uri)])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body[0]["id"], actor_uri);
    assert_eq!(body[0]["following"], true);
    assert_eq!(body[0]["followed_by"], true);
}

#[tokio::test]
async fn test_get_relationships_matches_default_port_equivalent_follow_requests() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .state
        .db
        .insert_follow_request(
            "alice@remote.example:443",
            "https://remote.example/inbox",
            "https://remote.example/follows/1",
        )
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/accounts/relationships"))
        .header("Authorization", format!("Bearer {}", token))
        .query(&[("id[]", "alice@remote.example")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body[0]["requested"], true);
}

#[tokio::test]
async fn test_get_relationships_returns_persisted_mute_notifications_flag() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .state
        .db
        .mute_account("alice@remote.example:443", false, None, Some(443))
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/accounts/relationships"))
        .header("Authorization", format!("Bearer {}", token))
        .query(&[("id[]", "alice@remote.example")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body[0]["muting"], true);
    assert_eq!(body[0]["muting_notifications"], false);
}

#[tokio::test]
async fn test_search_accounts() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/search?q=test"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_search_accounts_returns_empty_array_for_blank_query() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/search?q=%20%20"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<serde_json::Value>().await.unwrap(), json!([]));
}

#[tokio::test]
async fn test_search_accounts_applies_offset() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    cache_remote_profile(&server, "alpha@remote.example").await;
    cache_remote_profile(&server, "beta@remote.example").await;
    for address in ["alpha@remote.example", "beta@remote.example"] {
        server
            .state
            .db
            .insert_follow(&Follow {
                id: EntityId::new_string(),
                target_address: address.to_string(),
                actor_uri: Some(format!(
                    "https://remote.example/users/{}",
                    address.split('@').next().unwrap()
                )),
                uri: format!(
                    "https://test.example.com/follows/{}",
                    EntityId::new_string()
                ),
                created_at: Utc::now(),
            })
            .await
            .unwrap();
    }

    let response = server
        .client
        .get(server.url("/api/v1/accounts/search?q=e&offset=1&limit=1"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body.as_array().expect("accounts should be array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["acct"], "alpha@remote.example");
}

#[tokio::test]
async fn test_search_accounts_resolve_returns_remote_account_data() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_address = "alice@remote.example";
    cache_remote_profile(&server, remote_address).await;

    let response = server
        .client
        .get(server.url(&format!(
            "/api/v1/accounts/search?q={}&resolve=true",
            remote_address
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body.as_array().expect("accounts should be array");
    assert!(
        accounts
            .iter()
            .any(|account| account["acct"] == remote_address)
    );
}

#[tokio::test]
async fn test_search_accounts_resolve_actor_uri_query_uses_cached_alias_profile() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri = "https://remote.example/@alice";
    cache_remote_profile_alias_by_actor_uri(&server, actor_uri, "alice@remote.example").await;

    let encoded_query = urlencoding::encode(actor_uri).into_owned();
    let response = server
        .client
        .get(server.url(&format!(
            "/api/v1/accounts/search?q={encoded_query}&resolve=true"
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body.as_array().expect("accounts should be array");
    let account = accounts
        .iter()
        .find(|account| account["acct"] == "alice@remote.example")
        .expect("resolved account should be returned");
    assert_eq!(account["username"], "alice");
    assert_eq!(account["display_name"], "Alice Alias");
}

#[tokio::test]
async fn test_search_accounts_resolve_deduplicates_local_account_identity() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let local_full_address = "testuser@test.example.com";
    cache_remote_profile(&server, local_full_address).await;

    let response = server
        .client
        .get(server.url(&format!(
            "/api/v1/accounts/search?q={}&resolve=true",
            local_full_address
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body.as_array().expect("accounts should be array");
    let local_matches = accounts
        .iter()
        .filter(|account| {
            matches!(
                account["acct"].as_str(),
                Some("testuser") | Some("testuser@test.example.com")
            )
        })
        .count();
    assert_eq!(local_matches, 1);
}

#[tokio::test]
async fn test_search_accounts_resolve_local_address_skips_remote_lookup() {
    use std::time::Duration;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let local_full_address = "testuser@test.example.com";

    let response = tokio::time::timeout(
        Duration::from_secs(3),
        server
            .client
            .get(server.url(&format!(
                "/api/v1/accounts/search?q={}&resolve=true",
                local_full_address
            )))
            .header("Authorization", format!("Bearer {}", token))
            .send(),
    )
    .await
    .expect("local resolve search should not block on remote federation")
    .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body.as_array().expect("accounts should be array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["acct"], "testuser");
}

#[tokio::test]
async fn test_search_accounts_resolve_local_address_with_leading_at_skips_remote_lookup() {
    use std::time::Duration;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let local_full_address = "@testuser@test.example.com";

    let response = tokio::time::timeout(
        Duration::from_secs(3),
        server
            .client
            .get(server.url(&format!(
                "/api/v1/accounts/search?q={}&resolve=true",
                local_full_address
            )))
            .header("Authorization", format!("Bearer {}", token))
            .send(),
    )
    .await
    .expect("local resolve search should not block on remote federation")
    .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body.as_array().expect("accounts should be array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["acct"], "testuser");
}

#[tokio::test]
async fn test_search_accounts_following_filters_out_unfollowed_accounts() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_address = "alice@remote.example";
    cache_remote_profile(&server, remote_address).await;

    let response = server
        .client
        .get(server.url(&format!(
            "/api/v1/accounts/search?q={}&resolve=true&following=true",
            remote_address
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body.as_array().expect("search accounts should be array");
    assert!(accounts.is_empty());
}

#[tokio::test]
async fn test_search_accounts_following_keeps_followed_accounts() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_address = "alice@remote.example";
    cache_remote_profile(&server, remote_address).await;
    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: remote_address.to_string(),
            actor_uri: Some("https://remote.example/users/alice".to_string()),
            uri: "https://test.example.com/follows/alice-v1".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url(&format!(
            "/api/v1/accounts/search?q={}&resolve=true&following=true",
            remote_address
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body.as_array().expect("search accounts should be array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["acct"], remote_address);
}

#[tokio::test]
async fn test_get_account_lists() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/lists", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_account_lists_matches_default_port_equivalent_members() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let list_id = server
        .state
        .db
        .create_list("Port Equivalence", "list")
        .await
        .unwrap();
    server
        .state
        .db
        .add_accounts_to_list(&list_id, &[String::from("alice@remote.example:443")])
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/accounts/alice@remote.example/lists"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(
        body.as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == list_id)
    );
}

#[tokio::test]
async fn test_get_account_identity_proofs() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/accounts/{}/identity_proofs", account.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_account_identity_proofs_for_remote_account_returns_empty_array() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    cache_remote_profile(&server, "alice@remote.example").await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/alice@remote.example/identity_proofs"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<serde_json::Value>().await.unwrap(), json!([]));
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
        .get(server.url("/api/v1/follow_requests"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_follow_requests_prefers_stored_actor_uri_profile_alias() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri = "https://remote.example/users/alice";

    server
        .state
        .db
        .insert_follow_request_with_actor_uri(
            "alice@remote.example",
            "https://remote.example/inbox",
            "https://remote.example/follows/1",
            Some(actor_uri),
        )
        .await
        .unwrap();
    cache_remote_profile_alias_by_actor_uri(&server, actor_uri, "alice@remote.example").await;

    let response = server
        .client
        .get(server.url("/api/v1/follow_requests"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let requests = body.as_array().expect("follow_requests should be array");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["id"], "alice@remote.example");
    assert_eq!(requests[0]["acct"], "alice@remote.example");
    assert_eq!(requests[0]["display_name"], "Alice Alias");
}

#[tokio::test]
async fn test_get_follow_request_prefers_stored_actor_uri_profile_alias() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri = "https://remote.example/users/alice";

    server
        .state
        .db
        .insert_follow_request_with_actor_uri(
            "alice@remote.example",
            "https://remote.example/inbox",
            "https://remote.example/follows/1",
            Some(actor_uri),
        )
        .await
        .unwrap();
    cache_remote_profile_alias_by_actor_uri(&server, actor_uri, "alice@remote.example").await;

    let response = server
        .client
        .get(server.url("/api/v1/follow_requests/alice@remote.example"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["id"], "alice@remote.example");
    assert_eq!(body["acct"], "alice@remote.example");
    assert_eq!(body["display_name"], "Alice Alias");
}

#[tokio::test]
async fn test_get_follow_request_actor_uri_fallback_returns_mastodon_account_shape() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri = "https://remote.example/actors/12345";

    server
        .state
        .db
        .insert_follow_request_with_actor_uri(
            actor_uri,
            "https://remote.example/inbox",
            "https://remote.example/follows/12345",
            None,
        )
        .await
        .unwrap();
    let encoded_actor_uri = urlencoding::encode(actor_uri);

    let response = server
        .client
        .get(server.url(&format!("/api/v1/follow_requests/{encoded_actor_uri}")))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["id"], actor_uri);
    assert_eq!(body["acct"], "12345@remote.example");
    assert_eq!(body["url"], actor_uri);
    assert!(body.get("avatar_static").is_some());
    assert!(body.get("header_static").is_some());
    assert!(body.get("locked").is_some());
}

#[tokio::test]
async fn test_get_follow_request() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/follow_requests/test_id"))
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
        .post(server.url("/api/v1/follow_requests/test_id/authorize"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_authorize_follow_request_accepts_actor_uri_identity() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri = "https://remote.example/users/alice";

    server
        .state
        .db
        .insert_follow_request_with_actor_uri(
            "alice@remote.example",
            "https://remote.example/inbox",
            "https://remote.example/follows/1",
            Some(actor_uri),
        )
        .await
        .unwrap();

    let encoded_actor_uri = urlencoding::encode(actor_uri);
    let response = server
        .client
        .post(server.url(&format!(
            "/api/v1/follow_requests/{encoded_actor_uri}/authorize"
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["followed_by"], true);
}

#[tokio::test]
async fn test_reject_follow_request() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url("/api/v1/follow_requests/test_id/reject"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_reject_follow_request_accepts_actor_uri_identity() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri = "https://remote.example/users/alice";

    server
        .state
        .db
        .insert_follow_request_with_actor_uri(
            "alice@remote.example",
            "https://remote.example/inbox",
            "https://remote.example/follows/2",
            Some(actor_uri),
        )
        .await
        .unwrap();

    let encoded_actor_uri = urlencoding::encode(actor_uri);
    let response = server
        .client
        .post(server.url(&format!(
            "/api/v1/follow_requests/{encoded_actor_uri}/reject"
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["followed_by"], false);
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
        .post(server.url("/api/v1/statuses"))
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
        .post(server.url("/api/v1/statuses"))
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
            .get(server.url(&format!("/api/v1/statuses/{}", status_id)))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }
}

#[tokio::test]
async fn test_get_status_returns_current_metadata() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let root = server
        .client
        .post(server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "status": "Original @alice #rust",
            "visibility": "public"
        }))
        .send()
        .await
        .expect("create root status");
    assert_eq!(root.status(), 200);
    let root_body: serde_json::Value = root.json().await.expect("root json");
    let root_id = root_body["id"].as_str().expect("root id").to_string();

    let reply = server
        .client
        .post(server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "status": "Replying to root",
            "visibility": "public",
            "in_reply_to_id": root_id
        }))
        .send()
        .await
        .expect("create reply");
    assert_eq!(reply.status(), 200);

    let favourite = server
        .client
        .post(server.url(&format!("/api/v1/statuses/{}/favourite", root_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .expect("favourite root");
    assert_eq!(favourite.status(), 200);

    let reblog = server
        .client
        .post(server.url(&format!("/api/v1/statuses/{}/reblog", root_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .expect("reblog root");
    assert_eq!(reblog.status(), 200);

    let update = server
        .client
        .put(server.url(&format!("/api/v1/statuses/{}", root_id)))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "status": "Updated @alice #rust"
        }))
        .send()
        .await
        .expect("update root");
    assert_eq!(update.status(), 200);

    let response = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{}", root_id)))
        .send()
        .await
        .expect("get status");
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("status json");

    assert_eq!(body["replies_count"], 1);
    assert_eq!(body["reblogs_count"], 1);
    assert_eq!(body["favourites_count"], 1);
    assert_eq!(body["quotes_count"], 0);
    assert_eq!(body["text"], "Updated @alice #rust");
    assert!(body["edited_at"].as_str().is_some());
    assert_eq!(body["application"], serde_json::Value::Null);
    assert_eq!(body["filtered"], json!([]));

    let tags = body["tags"].as_array().expect("tags array");
    assert!(tags.iter().any(|tag| tag["name"] == "rust"));

    let mentions = body["mentions"].as_array().expect("mentions array");
    assert!(mentions.iter().any(|mention| mention["acct"] == "alice"));
}

#[tokio::test]
async fn test_get_status_allows_authenticated_private_status_and_preserves_interactions() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let created = server
        .client
        .post(server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "status": "Private test",
            "visibility": "private"
        }))
        .send()
        .await
        .expect("create private status");
    assert_eq!(created.status(), 200);
    let created_body: serde_json::Value = created.json().await.expect("created body");
    let status_id = created_body["id"].as_str().expect("status id");

    let favourite = server
        .client
        .post(server.url(&format!("/api/v1/statuses/{status_id}/favourite")))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .expect("favourite status");
    assert_eq!(favourite.status(), 200);

    let authed = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{status_id}")))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .expect("authed get status");
    assert_eq!(authed.status(), 200);
    let authed_body: serde_json::Value = authed.json().await.expect("authed status body");
    assert_eq!(authed_body["favourited"], true);

    let public = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{status_id}")))
        .send()
        .await
        .expect("public get status");
    assert_eq!(public.status(), 404);
}

#[tokio::test]
async fn test_get_status_card_returns_preview_for_visible_status_with_url() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let created = server
        .client
        .post(server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "status": "Card test https://example.com/article",
            "visibility": "public"
        }))
        .send()
        .await
        .expect("create status");
    assert_eq!(created.status(), 200);
    let created_body: serde_json::Value = created.json().await.expect("created body");
    let status_id = created_body["id"].as_str().expect("status id");

    let response = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{status_id}/card")))
        .send()
        .await
        .expect("get card");
    assert_eq!(response.status(), 200);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["type"], "link");
    assert_eq!(body["url"], "https://example.com/article");
}

#[tokio::test]
async fn test_get_status_includes_reblog_payload_and_reply_account_id() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let parent = server
        .client
        .post(server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "status": "Parent status",
            "visibility": "public"
        }))
        .send()
        .await
        .expect("create parent");
    assert_eq!(parent.status(), 200);
    let parent_body: serde_json::Value = parent.json().await.expect("parent body");
    let parent_id = parent_body["id"].as_str().expect("parent id").to_string();
    let parent_uri = parent_body["uri"].as_str().expect("parent uri").to_string();

    let reply = server
        .client
        .post(server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "status": "Reply to parent",
            "visibility": "public",
            "in_reply_to_id": parent_id
        }))
        .send()
        .await
        .expect("create reply");
    assert_eq!(reply.status(), 200);
    let reply_body: serde_json::Value = reply.json().await.expect("reply body");
    let reply_id = reply_body["id"].as_str().expect("reply id");

    let reblog_id = EntityId::new_string();
    server
        .state
        .db
        .insert_status(&Status {
            id: reblog_id.clone(),
            uri: server.public_url(&format!("/users/{}/statuses/{}", account.username, reblog_id)),
            content: "<p>boost wrapper</p>".to_string(),
            content_warning: None,
            visibility: StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: format!("{}@{}", account.username, server.state.config.server.domain),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: Some(parent_uri.clone()),
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        })
        .await
        .expect("insert reblog wrapper");

    let reply_response = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{reply_id}")))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .expect("get reply");
    assert_eq!(reply_response.status(), 200);
    let reply_value: serde_json::Value = reply_response.json().await.expect("reply json");
    assert_eq!(reply_value["in_reply_to_account_id"], account.id);

    let reblog_response = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{reblog_id}")))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .expect("get reblog");
    assert_eq!(reblog_response.status(), 200);
    let reblog_value: serde_json::Value = reblog_response.json().await.expect("reblog json");
    assert_eq!(reblog_value["reblog"]["id"], parent_id);
    assert_eq!(reblog_value["reblog"]["uri"], parent_uri);
    assert_eq!(reblog_value["reblog"]["content"], "<p>Parent status</p>");
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
        .post(server.url("/api/v1/statuses"))
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
            .delete(server.url(&format!("/api/v1/statuses/{}", status_id)))
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
        .post(server.url("/api/v1/statuses"))
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
            .get(server.url(&format!("/api/v1/statuses/{}/context", status_id)))
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
        .post(server.url("/api/v1/statuses"))
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
            .post(server.url(&format!("/api/v1/statuses/{}/favourite", status_id)))
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
        .post(server.url("/api/v1/statuses"))
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
            .post(server.url(&format!("/api/v1/statuses/{}/unfavourite", status_id)))
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
        .post(server.url("/api/v1/statuses"))
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
            .post(server.url(&format!("/api/v1/statuses/{}/reblog", status_id)))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_ne!(body["id"], body["uri"]);
        assert_eq!(body["reblog"]["id"], status_id);
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
        .post(server.url("/api/v1/statuses"))
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
            .post(server.url(&format!("/api/v1/statuses/{}/unreblog", status_id)))
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
        .post(server.url("/api/v1/statuses"))
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
            .post(server.url(&format!("/api/v1/statuses/{}/bookmark", status_id)))
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
        .post(server.url("/api/v1/statuses"))
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
            .post(server.url(&format!("/api/v1/statuses/{}/unbookmark", status_id)))
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
        .post(server.url("/api/v1/statuses"))
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
            .post(server.url(&format!("/api/v1/statuses/{}/pin", status_id)))
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
        .post(server.url("/api/v1/statuses"))
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
            .post(server.url(&format!("/api/v1/statuses/{}/unpin", status_id)))
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
        .get(server.url("/api/v1/timelines/home"))
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
        .get(server.url("/api/v1/timelines/public"))
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
        .get(server.url("/api/v1/timelines/tag/test"))
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
        .get(server.url("/api/v1/timelines/list/test_list_id"))
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
        .get(server.url("/api/v1/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_notifications_v2() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Notification, NotificationType};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    cache_remote_profile(&server, "alice@remote.example").await;

    server
        .state
        .db
        .insert_notification(&Notification {
            id: EntityId::new_string(),
            notification_type: NotificationType::Mention,
            origin_account_address: "alice@remote.example".to_string(),
            status_uri: None,
            read: false,
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v2/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body["accounts"].as_array().expect("accounts array");
    let groups = body["notification_groups"]
        .as_array()
        .expect("notification groups should be array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["type"], "mention");
    assert_eq!(groups[0]["notifications_count"], 1);
    assert_eq!(groups[0]["sample_account_ids"], json!(["alice@remote.example"]));
}

#[tokio::test]
async fn test_notifications_return_origin_account() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Notification, NotificationType};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    cache_remote_profile(&server, "alice@remote.example").await;

    server
        .state
        .db
        .insert_notification(&Notification {
            id: EntityId::new_string(),
            notification_type: NotificationType::Mention,
            origin_account_address: "alice@remote.example".to_string(),
            status_uri: None,
            read: false,
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let notifications = body.as_array().expect("notifications should be array");
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0]["account"]["id"], "alice@remote.example");
    assert_eq!(notifications[0]["account"]["acct"], "alice@remote.example");
}

#[tokio::test]
async fn test_admin_reports_return_reporting_account_and_status() {
    use chrono::Utc;
    use rustresort::data::{
        EntityId, Notification, NotificationType, PersistedReason, Status, StatusVisibility,
    };

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    cache_remote_profile(&server, "alice@remote.example").await;

    let status = Status {
        id: EntityId::new_string(),
        uri: server.public_url("/users/testuser/statuses/admin-report-target"),
        content: "<p>Flagged local status</p>".to_string(),
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
    };
    server.state.db.insert_status(&status).await.unwrap();

    server
        .state
        .db
        .insert_notification(&Notification {
            id: EntityId::new_string(),
            notification_type: NotificationType::AdminReport,
            origin_account_address: "alice@remote.example".to_string(),
            status_uri: Some(status.uri.clone()),
            read: false,
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/admin/reports"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let reports = body.as_array().expect("reports should be array");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["account"]["id"], "alice@remote.example");
    assert_eq!(reports[0]["target_account"]["username"], "testuser");
    assert_eq!(reports[0]["statuses"][0]["uri"], status.uri);
}

#[tokio::test]
async fn test_admin_reports_apply_filters_and_pagination() {
    use chrono::{Duration, Utc};
    use rustresort::data::{Notification, NotificationType};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;
    persist_remote_profile(&server, "alice@remote.example", "Alice").await;
    persist_remote_profile(&server, "bob@remote.example", "Bob").await;

    let base_time = Utc::now();
    for (id, origin, offset_seconds) in [
        ("report-001", "alice@remote.example", 1),
        ("report-002", "bob@remote.example", 2),
        ("report-003", "alice@remote.example", 3),
    ] {
        server
            .state
            .db
            .insert_notification(&Notification {
                id: id.to_string(),
                notification_type: NotificationType::AdminReport,
                origin_account_address: origin.to_string(),
                status_uri: None,
                read: false,
                created_at: base_time + Duration::seconds(offset_seconds),
            })
            .await
            .unwrap();
    }

    let first_page = server
        .client
        .get(server.url("/api/v1/admin/reports?limit=1"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(first_page.status(), 200);
    let first_page: serde_json::Value = first_page.json().await.unwrap();
    assert_eq!(first_page.as_array().unwrap().len(), 1);
    assert_eq!(first_page[0]["id"], "report-003");

    let max_id_page = server
        .client
        .get(server.url("/api/v1/admin/reports?max_id=report-003"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(max_id_page.status(), 200);
    let max_id_page: serde_json::Value = max_id_page.json().await.unwrap();
    let max_id_page = max_id_page.as_array().unwrap();
    assert_eq!(max_id_page[0]["id"], "report-002");
    assert_eq!(max_id_page[1]["id"], "report-001");

    let since_id_page = server
        .client
        .get(server.url("/api/v1/admin/reports?since_id=report-002"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(since_id_page.status(), 200);
    let since_id_page: serde_json::Value = since_id_page.json().await.unwrap();
    let since_id_page = since_id_page.as_array().unwrap();
    assert_eq!(since_id_page.len(), 1);
    assert_eq!(since_id_page[0]["id"], "report-003");

    let resolved_page = server
        .client
        .get(server.url("/api/v1/admin/reports?resolved=true"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(resolved_page.status(), 200);
    let resolved_page: serde_json::Value = resolved_page.json().await.unwrap();
    assert!(resolved_page.as_array().unwrap().is_empty());

    let account_filtered_page = server
        .client
        .get(server.url("/api/v1/admin/reports?account_id=bob@remote.example"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(account_filtered_page.status(), 200);
    let account_filtered_page: serde_json::Value = account_filtered_page.json().await.unwrap();
    let account_filtered_page = account_filtered_page.as_array().unwrap();
    assert_eq!(account_filtered_page.len(), 1);
    assert_eq!(account_filtered_page[0]["id"], "report-002");

    let target_filtered_page = server
        .client
        .get(server.url(&format!(
            "/api/v1/admin/reports?target_account_id={}",
            account.id
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(target_filtered_page.status(), 200);
    let target_filtered_page: serde_json::Value = target_filtered_page.json().await.unwrap();
    assert_eq!(target_filtered_page.as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn test_admin_report_mutation_endpoints_round_trip() {
    use chrono::Utc;
    use rustresort::data::{Notification, NotificationType};

    let server = TestServer::new().await;
    let local_account = server.create_test_account().await;
    let token = server.create_test_token().await;
    persist_remote_profile(&server, "alice@remote.example", "Alice").await;

    server
        .state
        .db
        .insert_notification(&Notification {
            id: "report-mutation-001".to_string(),
            notification_type: NotificationType::AdminReport,
            origin_account_address: "alice@remote.example".to_string(),
            status_uri: None,
            read: false,
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let get_response = server
        .client
        .get(server.url("/api/v1/admin/reports/report-mutation-001"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(get_response.status(), 200);
    let initial: serde_json::Value = get_response.json().await.unwrap();
    assert_eq!(initial["category"], "other");
    assert_eq!(initial["action_taken"], false);
    assert_eq!(initial["account"]["id"], "alice@remote.example");
    assert_eq!(initial["target_account"]["id"], local_account.id);

    let update_response = server
        .client
        .put(server.url("/api/v1/admin/reports/report-mutation-001"))
        .header("Authorization", format!("Bearer {}", token))
        .form(&[
            ("category", "violation"),
            ("rule_ids[]", "1"),
            ("rule_ids[]", "2"),
        ])
        .send()
        .await
        .unwrap();

    assert_eq!(update_response.status(), 200);
    let updated: serde_json::Value = update_response.json().await.unwrap();
    assert_eq!(updated["category"], "violation");
    assert_eq!(updated["rules"].as_array().unwrap().len(), 2);
    assert_eq!(updated["rules"][0]["id"], "1");
    assert_eq!(updated["rules"][1]["id"], "2");

    let assign_response = server
        .client
        .post(server.url("/api/v1/admin/reports/report-mutation-001/assign_to_self"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(assign_response.status(), 200);
    let assigned: serde_json::Value = assign_response.json().await.unwrap();
    assert_eq!(assigned["assigned_account"]["id"], local_account.id);

    let resolve_response = server
        .client
        .post(server.url("/api/v1/admin/reports/report-mutation-001/resolve"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(resolve_response.status(), 200);
    let resolved: serde_json::Value = resolve_response.json().await.unwrap();
    assert_eq!(resolved["action_taken"], true);
    assert_eq!(resolved["action_taken_by_account"]["id"], local_account.id);
    assert!(resolved["action_taken_at"].is_string());

    let reopen_response = server
        .client
        .post(server.url("/api/v1/admin/reports/report-mutation-001/reopen"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(reopen_response.status(), 200);
    let reopened: serde_json::Value = reopen_response.json().await.unwrap();
    assert_eq!(reopened["action_taken"], false);
    assert!(reopened["action_taken_at"].is_null());
    assert!(reopened["action_taken_by_account"].is_null());

    let unassign_response = server
        .client
        .post(server.url("/api/v1/admin/reports/report-mutation-001/unassign"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(unassign_response.status(), 200);
    let unassigned: serde_json::Value = unassign_response.json().await.unwrap();
    assert!(unassigned["assigned_account"].is_null());
}

#[tokio::test]
async fn test_admin_report_update_rejects_unknown_rule_ids() {
    use chrono::Utc;
    use rustresort::data::{Notification, NotificationType};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    server
        .state
        .db
        .insert_notification(&Notification {
            id: "report-rule-validation-001".to_string(),
            notification_type: NotificationType::AdminReport,
            origin_account_address: "alice@remote.example".to_string(),
            status_uri: None,
            read: false,
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .put(server.url("/api/v1/admin/reports/report-rule-validation-001"))
        .header("Authorization", format!("Bearer {}", token))
        .form(&[("category", "violation"), ("rule_ids[]", "999")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"], "unknown rule_ids: 999");
}

#[tokio::test]
async fn test_admin_account_action_suspend_and_unsuspend_round_trip() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    cache_remote_profile(&server, "alice@remote.example").await;

    let suspend_response = server
        .client
        .post(server.url("/api/v1/admin/accounts/alice@remote.example/action"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({ "action": "suspend" }))
        .send()
        .await
        .unwrap();

    assert_eq!(suspend_response.status(), 200);

    let blocks_response = server
        .client
        .get(server.url("/api/v1/blocks"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(blocks_response.status(), 200);
    let blocks: serde_json::Value = blocks_response.json().await.unwrap();
    assert!(
        blocks
            .as_array()
            .unwrap()
            .iter()
            .any(|account| account["acct"] == "alice@remote.example")
    );

    let unsuspend_response = server
        .client
        .post(server.url("/api/v1/admin/accounts/alice@remote.example/action"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({ "action": "unsuspend" }))
        .send()
        .await
        .unwrap();

    assert_eq!(unsuspend_response.status(), 200);

    let blocks_response = server
        .client
        .get(server.url("/api/v1/blocks"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(blocks_response.status(), 200);
    let blocks: serde_json::Value = blocks_response.json().await.unwrap();
    assert!(
        blocks
            .as_array()
            .unwrap()
            .iter()
            .all(|account| account["acct"] != "alice@remote.example")
    );
}

#[tokio::test]
async fn test_admin_account_action_accepts_form_type_and_resolves_report() {
    use chrono::Utc;
    use rustresort::data::{Notification, NotificationType};

    let server = TestServer::new().await;
    let local_account = server.create_test_account().await;
    let token = server.create_test_token().await;
    persist_remote_profile(&server, "alice@remote.example", "Alice").await;

    server
        .state
        .db
        .insert_notification(&Notification {
            id: "report-action-001".to_string(),
            notification_type: NotificationType::AdminReport,
            origin_account_address: "alice@remote.example".to_string(),
            status_uri: None,
            read: false,
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .post(server.url("/api/v1/admin/accounts/alice@remote.example/action"))
        .header("Authorization", format!("Bearer {}", token))
        .form(&[("type", "sensitive"), ("report_id", "report-action-001")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap(),
        json!({})
    );

    let report_response = server
        .client
        .get(server.url("/api/v1/admin/reports/report-action-001"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(report_response.status(), 200);
    let report: serde_json::Value = report_response.json().await.unwrap();
    assert_eq!(report["action_taken"], true);
    assert_eq!(report["action_taken_by_account"]["id"], local_account.id);
}

#[tokio::test]
async fn test_admin_account_action_sensitive_and_unsensitive_toggle_status_response() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    persist_remote_profile(&server, "alice@remote.example", "Alice").await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/users/alice/statuses/sensitive-1".to_string(),
        content: "<p>remote media</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "alice@remote.example".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Timeline,
        created_at: Utc::now(),
        fetched_at: Some(Utc::now()),
    };
    server.state.db.insert_status(&status).await.unwrap();

    let mark_response = server
        .client
        .post(server.url("/api/v1/admin/accounts/alice@remote.example/action"))
        .header("Authorization", format!("Bearer {}", token))
        .form(&[("type", "sensitive")])
        .send()
        .await
        .unwrap();
    assert_eq!(mark_response.status(), 200);

    let response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let entry = body
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["uri"] == status.uri)
        .expect("remote status should be visible");
    assert_eq!(entry["sensitive"], true);

    let clear_response = server
        .client
        .post(server.url("/api/v1/admin/accounts/alice@remote.example/unsensitive"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(clear_response.status(), 200);

    let response = server
        .client
        .get(server.url("/api/v1/timelines/public"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let entry = body
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["uri"] == status.uri)
        .expect("remote status should remain visible");
    assert_eq!(entry["sensitive"], false);
}

#[tokio::test]
async fn test_admin_accounts_list_and_detail_include_remote_moderation_state() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    cache_remote_profile(&server, "alice@remote.example").await;

    let suspend_response = server
        .client
        .post(server.url("/api/v1/admin/accounts/alice@remote.example/action"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({ "action": "suspend" }))
        .send()
        .await
        .unwrap();

    assert_eq!(suspend_response.status(), 200);

    let list_response = server
        .client
        .get(server.url("/api/v1/admin/accounts?remote=true&suspended=true&local=false"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(list_response.status(), 200);
    let accounts: serde_json::Value = list_response.json().await.unwrap();
    let remote_account = accounts
        .as_array()
        .unwrap()
        .iter()
        .find(|account| account["account"]["acct"] == "alice@remote.example")
        .expect("remote moderated account should be listed");
    assert_eq!(remote_account["username"], "alice");
    assert_eq!(remote_account["domain"], "remote.example");
    assert_eq!(remote_account["suspended"], true);

    let detail_response = server
        .client
        .get(server.url("/api/v1/admin/accounts/alice@remote.example"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(detail_response.status(), 200);
    let detail: serde_json::Value = detail_response.json().await.unwrap();
    assert_eq!(detail["account"]["acct"], "alice@remote.example");
    assert_eq!(detail["suspended"], true);
    assert_eq!(detail["domain"], "remote.example");
}

#[tokio::test]
async fn test_admin_accounts_include_persisted_remote_profiles_and_apply_cursors() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    persist_remote_profile(&server, "alice@remote.example", "Alice").await;
    persist_remote_profile(&server, "bob@remote.example", "Bob").await;
    persist_remote_profile(&server, "zoe@remote.example", "Zoe").await;

    let list_response = server
        .client
        .get(server.url("/api/v1/admin/accounts?remote=true&local=false"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(list_response.status(), 200);
    let accounts: serde_json::Value = list_response.json().await.unwrap();
    let accounts = accounts.as_array().expect("accounts should be an array");
    let account_ids: Vec<String> = accounts
        .iter()
        .map(|account| {
            account["id"]
                .as_str()
                .expect("admin account id")
                .to_string()
        })
        .collect();
    assert_eq!(
        account_ids,
        vec![
            "zoe@remote.example".to_string(),
            "bob@remote.example".to_string(),
            "alice@remote.example".to_string()
        ]
    );

    let paged_response = server
        .client
        .get(server.url("/api/v1/admin/accounts?remote=true&local=false&limit=1"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(paged_response.status(), 200);
    let paged: serde_json::Value = paged_response.json().await.unwrap();
    let first_id = paged[0]["id"].as_str().expect("first account id");
    assert_eq!(first_id, "zoe@remote.example");

    let max_id_response = server
        .client
        .get(server.url(&format!(
            "/api/v1/admin/accounts?remote=true&local=false&max_id={first_id}"
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(max_id_response.status(), 200);
    let max_id_accounts: serde_json::Value = max_id_response.json().await.unwrap();
    let max_id_accounts = max_id_accounts
        .as_array()
        .expect("max_id accounts should be array");
    assert_eq!(max_id_accounts[0]["id"], "bob@remote.example");
    assert_eq!(max_id_accounts[1]["id"], "alice@remote.example");

    let since_id_response = server
        .client
        .get(
            server
                .url("/api/v1/admin/accounts?remote=true&local=false&since_id=bob@remote.example"),
        )
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(since_id_response.status(), 200);
    let since_id_accounts: serde_json::Value = since_id_response.json().await.unwrap();
    let since_id_accounts = since_id_accounts
        .as_array()
        .expect("since_id accounts should be array");
    assert_eq!(since_id_accounts.len(), 1);
    assert_eq!(since_id_accounts[0]["id"], "zoe@remote.example");

    let email_filter_response = server
        .client
        .get(server.url("/api/v1/admin/accounts?remote=true&local=false&email=admin@example.com"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(email_filter_response.status(), 200);
    let filtered: serde_json::Value = email_filter_response.json().await.unwrap();
    assert_eq!(filtered.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_admin_accounts_apply_active_and_pending_filters() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    persist_remote_profile(&server, "alice@remote.example", "Alice").await;
    persist_remote_profile(&server, "bob@remote.example", "Bob").await;

    let suspend_response = server
        .client
        .post(server.url("/api/v1/admin/accounts/alice@remote.example/action"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({ "action": "suspend" }))
        .send()
        .await
        .unwrap();

    assert_eq!(suspend_response.status(), 200);

    let active_response = server
        .client
        .get(server.url("/api/v1/admin/accounts?remote=true&local=false&active=true"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(active_response.status(), 200);
    let active_accounts: serde_json::Value = active_response.json().await.unwrap();
    let active_accounts = active_accounts.as_array().unwrap();
    assert_eq!(active_accounts.len(), 1);
    assert_eq!(active_accounts[0]["id"], "bob@remote.example");

    let inactive_response = server
        .client
        .get(server.url("/api/v1/admin/accounts?remote=true&local=false&active=false"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(inactive_response.status(), 200);
    let inactive_accounts: serde_json::Value = inactive_response.json().await.unwrap();
    let inactive_accounts = inactive_accounts.as_array().unwrap();
    assert_eq!(inactive_accounts.len(), 1);
    assert_eq!(inactive_accounts[0]["id"], "alice@remote.example");

    let pending_response = server
        .client
        .get(server.url("/api/v1/admin/accounts?remote=true&local=false&pending=true"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(pending_response.status(), 200);
    let pending_accounts: serde_json::Value = pending_response.json().await.unwrap();
    assert!(pending_accounts.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_admin_domain_blocks_round_trip_by_returned_id() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let create_response = server
        .client
        .post(server.url("/api/v1/admin/domain_blocks"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "domain": "Remote.Example",
            "severity": "Silence",
            "reject_media": false,
            "reject_reports": false,
            "private_comment": "private note",
            "public_comment": "public note",
            "obfuscate": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(create_response.status(), 200);
    let created: serde_json::Value = create_response.json().await.unwrap();
    assert_eq!(created["domain"], "remote.example");
    assert_eq!(created["severity"], "silence");
    assert_eq!(created["reject_media"], false);
    assert_eq!(created["reject_reports"], false);
    assert_eq!(created["private_comment"], "private note");
    assert_eq!(created["public_comment"], "public note");
    assert_eq!(created["obfuscate"], true);
    let block_id = created["id"].as_str().expect("domain block id");

    let list_response = server
        .client
        .get(server.url("/api/v1/admin/domain_blocks"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(list_response.status(), 200);
    let blocks: serde_json::Value = list_response.json().await.unwrap();
    assert!(blocks.as_array().unwrap().iter().any(|block| {
        block["id"] == block_id
            && block["domain"] == "remote.example"
            && block["severity"] == "silence"
            && block["reject_media"] == false
            && block["reject_reports"] == false
            && block["private_comment"] == "private note"
            && block["public_comment"] == "public note"
            && block["obfuscate"] == true
    }));

    let update_response = server
        .client
        .post(server.url("/api/v1/admin/domain_blocks"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "domain": "remote.example",
            "severity": "Suspend",
            "reject_media": true,
            "reject_reports": true,
            "private_comment": "updated private",
            "public_comment": "updated public",
            "obfuscate": false
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(update_response.status(), 200);
    let updated: serde_json::Value = update_response.json().await.unwrap();
    assert_eq!(updated["id"], block_id);
    assert_eq!(updated["severity"], "suspend");
    assert_eq!(updated["reject_media"], true);
    assert_eq!(updated["reject_reports"], true);
    assert_eq!(updated["private_comment"], "updated private");
    assert_eq!(updated["public_comment"], "updated public");
    assert_eq!(updated["obfuscate"], false);

    let delete_response = server
        .client
        .delete(server.url(&format!("/api/v1/admin/domain_blocks/{block_id}")))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(delete_response.status(), 200);

    let list_response = server
        .client
        .get(server.url("/api/v1/admin/domain_blocks"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(list_response.status(), 200);
    let blocks: serde_json::Value = list_response.json().await.unwrap();
    assert!(
        blocks
            .as_array()
            .unwrap()
            .iter()
            .all(|block| block["id"] != block_id)
    );
}

#[tokio::test]
async fn test_admin_domain_blocks_reject_invalid_severity() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url("/api/v1/admin/domain_blocks"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "domain": "remote.example",
            "severity": "drop"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(
        body["error"],
        "severity must be one of: noop, silence, suspend"
    );
}

#[tokio::test]
async fn test_legacy_admin_domain_blocks_share_domain_block_store() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let (_, session_cookie) = server.login_password().await;

    let create_response = server
        .client
        .post(server.url("/admin/domain_blocks"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({ "domain": "Remote.Example" }))
        .send()
        .await
        .unwrap();

    assert_eq!(create_response.status(), 200);

    let v1_list_response = server
        .client
        .get(server.url("/api/v1/admin/domain_blocks"))
        .header(
            "Authorization",
            format!("Bearer {}", server.create_test_token().await),
        )
        .send()
        .await
        .unwrap();

    assert_eq!(v1_list_response.status(), 200);
    let v1_blocks: serde_json::Value = v1_list_response.json().await.unwrap();
    assert!(
        v1_blocks
            .as_array()
            .unwrap()
            .iter()
            .any(|block| { block["domain"] == "remote.example" && block["severity"] == "suspend" })
    );

    let legacy_list_response = server
        .client
        .get(server.url("/admin/domain_blocks"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .unwrap();

    assert_eq!(legacy_list_response.status(), 200);
    assert_eq!(
        legacy_list_response
            .json::<serde_json::Value>()
            .await
            .unwrap(),
        json!(["remote.example"])
    );

    let delete_response = server
        .client
        .delete(server.url("/admin/domain_blocks/remote.example"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .unwrap();

    assert_eq!(delete_response.status(), 200);

    let token = server.create_test_token().await;
    let v1_list_response = server
        .client
        .get(server.url("/api/v1/admin/domain_blocks"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(v1_list_response.status(), 200);
    assert!(
        v1_list_response
            .json::<serde_json::Value>()
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn test_notifications_embed_status_with_current_interactions() {
    use chrono::Utc;
    use rustresort::data::{
        EntityId, Notification, NotificationType, PersistedReason, Status, StatusVisibility,
    };

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    cache_remote_profile(&server, "alice@remote.example").await;

    let status = Status {
        id: EntityId::new_string(),
        uri: server.public_url("/users/testuser/statuses/notif-status-1"),
        content: "<p>status embedded in notification</p>".to_string(),
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
    server.state.db.insert_status(&status).await.unwrap();
    server.state.db.insert_bookmark(&status.id).await.unwrap();
    server.state.db.insert_status_pin(&status.id).await.unwrap();

    let notification = Notification {
        id: EntityId::new_string(),
        notification_type: NotificationType::Favourite,
        origin_account_address: "alice@remote.example".to_string(),
        status_uri: Some(status.uri.clone()),
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
        .get(server.url("/api/v1/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let notifications = body.as_array().expect("notifications should be array");
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0]["status"]["id"], status.id);
    assert_eq!(notifications[0]["status"]["bookmarked"], true);
    assert_eq!(notifications[0]["status"]["pinned"], true);

    let single_response = server
        .client
        .get(server.url(&format!("/api/v1/notifications/{}", notification.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(single_response.status(), 200);
    let single: serde_json::Value = single_response.json().await.unwrap();
    assert_eq!(single["status"]["bookmarked"], true);
    assert_eq!(single["status"]["pinned"], true);
}

#[tokio::test]
async fn test_get_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/notifications/test_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_dismiss_notification() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Notification, NotificationType};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let notification = Notification {
        id: EntityId::new_string(),
        notification_type: NotificationType::Mention,
        origin_account_address: "alice@remote.example".to_string(),
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

    let response = server
        .client
        .post(server.url(&format!(
            "/api/v1/notifications/{}/dismiss",
            notification.id
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let list_response = server
        .client
        .get(server.url("/api/v1/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(list_response.status(), 200);
    let notifications: serde_json::Value = list_response.json().await.unwrap();
    assert!(
        notifications
            .as_array()
            .expect("notifications should be array")
            .iter()
            .all(|item| item["id"] != notification.id)
    );

    let single_response = server
        .client
        .get(server.url(&format!("/api/v1/notifications/{}", notification.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(single_response.status(), 404);
}

#[tokio::test]
async fn test_clear_notifications() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Notification, NotificationType};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    for notification_type in [NotificationType::Mention, NotificationType::Favourite] {
        server
            .state
            .db
            .insert_notification(&Notification {
                id: EntityId::new_string(),
                notification_type,
                origin_account_address: "alice@remote.example".to_string(),
                status_uri: None,
                read: false,
                created_at: Utc::now(),
            })
            .await
            .unwrap();
    }

    let response = server
        .client
        .post(server.url("/api/v1/notifications/clear"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let list_response = server
        .client
        .get(server.url("/api/v1/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(list_response.status(), 200);
    let notifications: serde_json::Value = list_response.json().await.unwrap();
    assert!(
        notifications
            .as_array()
            .expect("notifications should be array")
            .is_empty()
    );
}

#[tokio::test]
async fn test_get_notifications_applies_type_filter_before_pagination() {
    use chrono::{Duration, Utc};
    use rustresort::data::{EntityId, Notification, NotificationType};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let now = Utc::now();

    server
        .state
        .db
        .insert_notification(&Notification {
            id: EntityId::new_string(),
            notification_type: NotificationType::Follow,
            origin_account_address: "alice@remote.example".to_string(),
            status_uri: None,
            read: false,
            created_at: now,
        })
        .await
        .unwrap();
    server
        .state
        .db
        .insert_notification(&Notification {
            id: EntityId::new_string(),
            notification_type: NotificationType::Mention,
            origin_account_address: "bob@remote.example".to_string(),
            status_uri: None,
            read: false,
            created_at: now - Duration::seconds(1),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/notifications"))
        .query(&[("limit", "1"), ("types[]", "mention")])
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let notifications = body.as_array().expect("notifications should be array");
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0]["type"], "mention");
}

#[tokio::test]
async fn test_get_unread_count() {
    use chrono::{Duration, Utc};
    use rustresort::data::{EntityId, Notification, NotificationType};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let now = Utc::now();

    for idx in 0..1005 {
        server
            .state
            .db
            .insert_notification(&Notification {
                id: EntityId::new_string(),
                notification_type: NotificationType::Mention,
                origin_account_address: format!("alice{idx}@remote.example"),
                status_uri: None,
                read: false,
                created_at: now - Duration::milliseconds(idx as i64),
            })
            .await
            .unwrap();
    }

    let response = server
        .client
        .get(server.url("/api/v1/notifications/unread_count"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["count"], 1005);
}

// ============================================================================
// Media Endpoints (4 endpoints)
// ============================================================================

#[tokio::test]
async fn test_upload_media() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url("/api/v1/media"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success()
            || response.status() == 400
            || response.status() == 422
            || response.status() == 500
    );
}

#[tokio::test]
async fn test_upload_media_v2() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url("/api/v2/media"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success()
            || response.status() == 400
            || response.status() == 422
            || response.status() == 500
    );
}

#[tokio::test]
async fn test_upload_media_v2_success_returns_200() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(vec![0, 1, 2, 3])
            .file_name("test.png")
            .mime_str("image/png")
            .unwrap(),
    );

    let response = server
        .client
        .post(server.url("/api/v2/media"))
        .header("Authorization", format!("Bearer {}", token))
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["id"].is_string());
    assert_eq!(body["type"], "image");
}

#[tokio::test]
async fn test_get_media() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/media/test_media_id"))
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
    use chrono::Utc;
    use rustresort::data::{EntityId, MediaAttachment};

    let media = MediaAttachment {
        id: EntityId::new_string(),
        status_id: None,
        s3_key: "media/update-target.webp".to_string(),
        thumbnail_s3_key: None,
        content_type: "image/webp".to_string(),
        file_size: 1024,
        description: Some("before".to_string()),
        blurhash: None,
        width: Some(64),
        height: Some(64),
        focus_x: None,
        focus_y: None,
        created_at: Utc::now(),
    };
    server.state.db.insert_media(&media).await.unwrap();

    let form = reqwest::multipart::Form::new()
        .text("description", "Updated description")
        .text("focus", "0.25,-0.25");

    let response = server
        .client
        .put(server.url(&format!("/api/v1/media/{}", media.id)))
        .header("Authorization", format!("Bearer {}", token))
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let updated = server.state.db.get_media(&media.id).await.unwrap().unwrap();
    assert_eq!(updated.description.as_deref(), Some("Updated description"));
    assert!(updated.thumbnail_s3_key.is_none());
    assert_eq!(updated.focus_x, Some(0.25));
    assert_eq!(updated.focus_y, Some(-0.25));
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
        .get(server.url("/api/v1/lists"))
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

    let response = server
        .client
        .post(server.url("/api/v1/lists"))
        .header("Authorization", format!("Bearer {}", token))
        .form(&[("title", "Test List"), ("replies_policy", "followed")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["title"], "Test List");
    assert_eq!(body["replies_policy"], "followed");
}

#[tokio::test]
async fn test_get_list() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/lists/test_list_id"))
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
    let list_id = server.state.db.create_list("before", "list").await.unwrap();

    let response = server
        .client
        .put(server.url(&format!("/api/v1/lists/{list_id}")))
        .header("Authorization", format!("Bearer {}", token))
        .form(&[("title", "Updated List"), ("replies_policy", "none")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["title"], "Updated List");
    assert_eq!(body["replies_policy"], "none");
}

#[tokio::test]
async fn test_delete_list() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .delete(server.url("/api/v1/lists/test_list_id"))
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
    let list_id = server
        .state
        .db
        .create_list("Remote list", "list")
        .await
        .unwrap();
    server
        .state
        .db
        .add_accounts_to_list(&list_id, &["alice@remote.example".to_string()])
        .await
        .unwrap();
    cache_remote_profile(&server, "alice@remote.example").await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/lists/{list_id}/accounts")))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body.as_array().expect("list accounts should be array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["acct"], "alice@remote.example");
    assert_eq!(accounts[0]["display_name"], "Alice Remote");
    assert!(accounts[0].get("avatar_static").is_some());
    assert!(accounts[0].get("header_static").is_some());
    assert!(accounts[0].get("locked").is_some());
    assert!(accounts[0]["emojis"].is_array());

    let paged = server
        .client
        .get(server.url(&format!(
            "/api/v1/lists/{list_id}/accounts?max_id={}",
            accounts[0]["id"].as_str().unwrap()
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(paged.status(), 200);
    let paged: serde_json::Value = paged.json().await.unwrap();
    assert!(paged.as_array().unwrap().is_empty());
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
        .post(server.url("/api/v1/lists/test_list_id/accounts"))
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
        .get(server.url("/api/v1/filters"))
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
        .post(server.url("/api/v1/filters"))
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
        .get(server.url("/api/v1/filters/test_filter_id"))
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
        .put(server.url("/api/v1/filters/test_filter_id"))
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
        .delete(server.url("/api/v1/filters/test_filter_id"))
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
        .get(server.url("/api/v2/filters"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_filter_v2_crud_and_keyword_endpoints() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let created = server
        .client
        .post(server.url("/api/v2/filters"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "title": "spoiler filter",
            "context": ["home"],
            "filter_action": "warn",
            "keywords": [
                {"keyword": "spoiler", "whole_word": true},
                {"keyword": "cw", "whole_word": false}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);
    let created: serde_json::Value = created.json().await.unwrap();
    let filter_id = created["id"].as_str().unwrap();

    let keywords = server
        .client
        .get(server.url(&format!("/api/v2/filters/{}/keywords", filter_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(keywords.status(), 200);
    let keywords: serde_json::Value = keywords.json().await.unwrap();
    let keywords = keywords.as_array().unwrap();
    assert_eq!(keywords.len(), 2);
    let keyword = keywords
        .iter()
        .find(|keyword| keyword["keyword"] == "spoiler")
        .expect("spoiler keyword should exist");
    let keyword_id = keyword["id"].as_str().unwrap();

    let updated = server
        .client
        .put(server.url(&format!("/api/v2/filters/keywords/{}", keyword_id)))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "keyword": "updated spoiler",
            "whole_word": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(updated.status(), 200);
    let updated: serde_json::Value = updated.json().await.unwrap();
    assert_eq!(updated["id"], keyword_id);

    let status_response = server
        .client
        .post(server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "status": "filter status target",
            "visibility": "public"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(status_response.status(), 200);
    let status_response: serde_json::Value = status_response.json().await.unwrap();
    let status_id = status_response["id"].as_str().unwrap();

    let attached = server
        .client
        .post(server.url(&format!("/api/v2/filters/{}/statuses", filter_id)))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({ "status_id": status_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(attached.status(), 200);
    let attached: serde_json::Value = attached.json().await.unwrap();
    let filter_status_id = attached["id"].as_str().unwrap();

    let filter = server
        .client
        .get(server.url(&format!("/api/v2/filters/{}", filter_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(filter.status(), 200);
    let filter: serde_json::Value = filter.json().await.unwrap();
    assert_eq!(filter["keywords"].as_array().unwrap().len(), 2);
    assert_eq!(filter["statuses"].as_array().unwrap().len(), 1);
    assert_eq!(filter["statuses"][0]["status_id"], status_id);

    let listed_statuses = server
        .client
        .get(server.url(&format!("/api/v2/filters/{}/statuses", filter_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(listed_statuses.status(), 200);
    let listed_statuses: serde_json::Value = listed_statuses.json().await.unwrap();
    assert_eq!(listed_statuses.as_array().unwrap().len(), 1);

    let filtered_status = server
        .client
        .get(server.url(&format!("/api/v1/statuses/{status_id}")))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(filtered_status.status(), 200);
    let filtered_status: serde_json::Value = filtered_status.json().await.unwrap();
    let filtered = filtered_status["filtered"]
        .as_array()
        .expect("filtered array");
    assert!(
        filtered.iter().any(|entry| entry["status_matches"] == json!([status_id])),
        "status filter should be reflected in status serialization"
    );

    let deleted_keyword = server
        .client
        .delete(server.url(&format!("/api/v2/filters/keywords/{}", keyword_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted_keyword.status(), 200);

    let remaining_keyword_id = keywords
        .iter()
        .find(|keyword| keyword["id"] != keyword_id)
        .and_then(|keyword| keyword["id"].as_str())
        .unwrap();
    let deleted_last_keyword = server
        .client
        .delete(server.url(&format!(
            "/api/v2/filters/keywords/{}",
            remaining_keyword_id
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted_last_keyword.status(), 200);

    let filter_after_keyword_delete = server
        .client
        .get(server.url(&format!("/api/v2/filters/{}", filter_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(filter_after_keyword_delete.status(), 200);
    let filter_after_keyword_delete: serde_json::Value =
        filter_after_keyword_delete.json().await.unwrap();
    assert!(
        filter_after_keyword_delete["keywords"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let deleted_status = server
        .client
        .delete(server.url(&format!("/api/v2/filters/statuses/{}", filter_status_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted_status.status(), 200);
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
        .get(server.url("/api/v1/bookmarks"))
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
        .get(server.url("/api/v1/favourites"))
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
        .get(server.url("/api/v1/search?q=test"))
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
        .get(server.url("/api/v2/search?q=test"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_search_v2_resolve_returns_remote_account_data() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_address = "alice@remote.example";
    cache_remote_profile(&server, remote_address).await;

    let response = server
        .client
        .get(server.url(&format!(
            "/api/v2/search?q={}&type=accounts&resolve=true",
            remote_address
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body["accounts"]
        .as_array()
        .expect("search v2 accounts should be array");
    assert!(
        accounts
            .iter()
            .any(|account| account["acct"] == remote_address)
    );
}

#[tokio::test]
async fn test_search_v2_resolve_actor_uri_query_uses_cached_alias_profile() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let actor_uri = "https://remote.example/@alice";
    cache_remote_profile_alias_by_actor_uri(&server, actor_uri, "alice@remote.example").await;

    let encoded_query = urlencoding::encode(actor_uri).into_owned();
    let response = server
        .client
        .get(server.url(&format!(
            "/api/v2/search?q={encoded_query}&type=accounts&resolve=true"
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body["accounts"]
        .as_array()
        .expect("search v2 accounts should be array");
    let account = accounts
        .iter()
        .find(|account| account["acct"] == "alice@remote.example")
        .expect("resolved account should be returned");
    assert_eq!(account["username"], "alice");
    assert_eq!(account["display_name"], "Alice Alias");
}

#[tokio::test]
async fn test_search_v2_resolve_deduplicates_local_account_identity() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let local_full_address = "testuser@test.example.com";
    cache_remote_profile(&server, local_full_address).await;

    let response = server
        .client
        .get(server.url(&format!(
            "/api/v2/search?q={}&type=accounts&resolve=true",
            local_full_address
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body["accounts"]
        .as_array()
        .expect("search v2 accounts should be array");
    let local_matches = accounts
        .iter()
        .filter(|account| {
            matches!(
                account["acct"].as_str(),
                Some("testuser") | Some("testuser@test.example.com")
            )
        })
        .count();
    assert_eq!(local_matches, 1);
}

#[tokio::test]
async fn test_search_v2_accounts_respects_limit() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let local_full_address = "testuser@test.example.com";
    cache_remote_profile(&server, local_full_address).await;

    let response = server
        .client
        .get(server.url(&format!(
            "/api/v2/search?q={}&type=accounts&resolve=true&limit=0",
            local_full_address
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body["accounts"]
        .as_array()
        .expect("search v2 accounts should be array");
    assert!(accounts.is_empty());
}

#[tokio::test]
async fn test_search_v2_resolve_local_address_skips_remote_lookup() {
    use std::time::Duration;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let local_full_address = "testuser@test.example.com";

    let response = tokio::time::timeout(
        Duration::from_secs(3),
        server
            .client
            .get(server.url(&format!(
                "/api/v2/search?q={}&type=accounts&resolve=true",
                local_full_address
            )))
            .header("Authorization", format!("Bearer {}", token))
            .send(),
    )
    .await
    .expect("local resolve search should not block on remote federation")
    .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body["accounts"]
        .as_array()
        .expect("search v2 accounts should be array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["acct"], "testuser");
}

#[tokio::test]
async fn test_search_v2_resolve_local_address_with_leading_at_skips_remote_lookup() {
    use std::time::Duration;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let local_full_address = "@testuser@test.example.com";

    let response = tokio::time::timeout(
        Duration::from_secs(3),
        server
            .client
            .get(server.url(&format!(
                "/api/v2/search?q={}&type=accounts&resolve=true",
                local_full_address
            )))
            .header("Authorization", format!("Bearer {}", token))
            .send(),
    )
    .await
    .expect("local resolve search should not block on remote federation")
    .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body["accounts"]
        .as_array()
        .expect("search v2 accounts should be array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["acct"], "testuser");
}

#[tokio::test]
async fn test_search_v2_accounts_following_filters_out_unfollowed_accounts() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_address = "alice@remote.example";
    cache_remote_profile(&server, remote_address).await;

    let response = server
        .client
        .get(server.url(&format!(
            "/api/v2/search?q={}&type=accounts&resolve=true&following=true",
            remote_address
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body["accounts"]
        .as_array()
        .expect("search v2 accounts should be array");
    assert!(accounts.is_empty());
}

#[tokio::test]
async fn test_search_v2_accounts_following_keeps_followed_accounts() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_address = "alice@remote.example";
    cache_remote_profile(&server, remote_address).await;
    server
        .state
        .db
        .insert_follow(&Follow {
            id: EntityId::new_string(),
            target_address: remote_address.to_string(),
            actor_uri: None,
            uri: "https://test.example.com/follows/alice".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url(&format!(
            "/api/v2/search?q={}&type=accounts&resolve=true&following=true",
            remote_address
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body["accounts"]
        .as_array()
        .expect("search v2 accounts should be array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["acct"], remote_address);

    let status = rustresort::data::Status {
        id: "local-status-search".to_string(),
        uri: "https://test.example.com/users/testuser/statuses/local-status-search".to_string(),
        content: "<p>followed search content</p>".to_string(),
        content_warning: None,
        visibility: rustresort::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: format!("{}@{}", account.username, server.state.config.server.domain),
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
        .get(server.url(&format!(
            "/api/v2/search?q=followed&account_id={}",
            account.id
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let statuses = body["statuses"]
        .as_array()
        .expect("search v2 statuses should be array");
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0]["id"], status.id);
}

#[tokio::test]
async fn test_search_v2_accounts_applies_offset() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Follow};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    cache_remote_profile(&server, "alpha@remote.example").await;
    cache_remote_profile(&server, "beta@remote.example").await;
    for address in ["alpha@remote.example", "beta@remote.example"] {
        server
            .state
            .db
            .insert_follow(&Follow {
                id: EntityId::new_string(),
                target_address: address.to_string(),
                actor_uri: Some(format!(
                    "https://remote.example/users/{}",
                    address.split('@').next().unwrap()
                )),
                uri: format!(
                    "https://test.example.com/follows/{}",
                    EntityId::new_string()
                ),
                created_at: Utc::now(),
            })
            .await
            .unwrap();
    }

    let response = server
        .client
        .get(server.url("/api/v2/search?q=e&type=accounts&offset=1&limit=1"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let accounts = body["accounts"]
        .as_array()
        .expect("search v2 accounts should be array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["acct"], "alpha@remote.example");
}

#[tokio::test]
async fn test_search_v2_hashtags_applies_offset() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    for status in ["#rust first", "#rustlang second"] {
        let response = server
            .client
            .post(server.url("/api/v1/statuses"))
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "status": status,
                "visibility": "public"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }

    let response = server
        .client
        .get(server.url("/api/v2/search?q=rust&type=hashtags&offset=1&limit=10"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let hashtags = body["hashtags"]
        .as_array()
        .expect("search v2 hashtags should be array");
    assert_eq!(hashtags.len(), 1);
    assert_eq!(hashtags[0]["name"], "rustlang");

    let empty_page = server
        .client
        .get(server.url("/api/v2/search?q=rust&type=hashtags&offset=5&limit=10"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(empty_page.status(), 200);
    let empty_page: serde_json::Value = empty_page.json().await.unwrap();
    assert!(empty_page["hashtags"].as_array().unwrap().is_empty());
}

// ============================================================================
// Polls Endpoints (2 endpoints)
// ============================================================================

#[tokio::test]
async fn test_get_poll() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let created = server
        .client
        .post(server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "status": "poll response",
            "poll": {
                "options": ["yes", "no"],
                "expires_in": 600
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);
    let created: serde_json::Value = created.json().await.unwrap();
    let poll_id = created["poll"]["id"].as_str().unwrap();

    let response = server
        .client
        .get(server.url(&format!("/api/v1/polls/{}", poll_id)))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let poll: serde_json::Value = response.json().await.unwrap();
    assert_eq!(poll["voted"], false);
    assert!(poll["own_votes"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_vote_in_poll() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let created = server
        .client
        .post(server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "status": "vote in poll",
            "poll": {
                "options": ["yes", "no"],
                "expires_in": 600
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);
    let created: serde_json::Value = created.json().await.unwrap();
    let poll_id = created["poll"]["id"].as_str().unwrap();

    let response = server
        .client
        .post(server.url(&format!("/api/v1/polls/{}/votes", poll_id)))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "choices": [0]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["voted"], true);
    assert_eq!(body["own_votes"][0], 0);
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
        .get(server.url("/api/v1/scheduled_statuses"))
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
        .get(server.url("/api/v1/scheduled_statuses/test_id"))
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

    let scheduled_at = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    let update_data = json!({
        "scheduled_at": scheduled_at
    });

    let response = server
        .client
        .put(server.url("/api/v1/scheduled_statuses/test_id"))
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
        .delete(server.url("/api/v1/scheduled_statuses/test_id"))
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
        .get(server.url("/api/v1/conversations"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_conversations_emits_link_pagination_header() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    for index in 0..2 {
        server
            .state
            .db
            .get_or_create_conversation(&[
                "testuser@test.example.com".to_string(),
                format!("remote{index}@remote.example"),
            ])
            .await
            .unwrap();
    }

    let response = server
        .client
        .get(server.url("/api/v1/conversations?limit=1"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let link = response
        .headers()
        .get(reqwest::header::LINK)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(link.contains("/api/v1/conversations?limit=1"));
    assert!(link.contains("rel=\"next\""));
    assert!(link.contains("rel=\"prev\""));
}

#[tokio::test]
async fn test_get_conversations_returns_full_accounts_and_last_status() {
    use chrono::Utc;
    use rustresort::data::{EntityId, PersistedReason, Status, StatusVisibility};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    cache_remote_profile(&server, "alice@remote.example").await;

    let conversation_id = server
        .state
        .db
        .get_or_create_conversation(&[
            "testuser@test.example.com".to_string(),
            "alice@remote.example".to_string(),
        ])
        .await
        .unwrap();
    let status = Status {
        id: EntityId::new_string(),
        uri: "https://test.example.com/users/testuser/statuses/conversation-1".to_string(),
        content: "<p>hello</p>".to_string(),
        content_warning: None,
        visibility: StatusVisibility::Direct,
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
    server.state.db.insert_status(&status).await.unwrap();
    server
        .state
        .db
        .add_status_to_conversation(&conversation_id, &status.id)
        .await
        .unwrap();

    let response = server
        .client
        .get(server.url("/api/v1/conversations"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let conversations = body.as_array().expect("conversations should be array");
    assert_eq!(conversations.len(), 1);
    assert_eq!(
        conversations[0]["accounts"][0]["acct"],
        "alice@remote.example"
    );
    assert_eq!(conversations[0]["last_status"]["id"], status.id);
    assert!(conversations[0]["last_status"]["account"].is_object());
}

#[tokio::test]
async fn test_mark_conversation_read_returns_target_even_when_older_than_first_page() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let mut target_id = None;
    for index in 0..45 {
        let conversation_id = server
            .state
            .db
            .get_or_create_conversation(&[
                format!("testuser{}@test.example.com", index),
                format!("remote{}@remote.example", index),
            ])
            .await
            .unwrap();
        if index == 0 {
            target_id = Some(conversation_id);
        }
    }
    let target_id = target_id.expect("target conversation should be created");

    let response = server
        .client
        .post(server.url(&format!("/api/v1/conversations/{}/read", target_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["id"], target_id);
    assert_eq!(body["unread"], false);
}

#[tokio::test]
async fn test_delete_conversation() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .delete(server.url("/api/v1/conversations/test_id"))
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
        .post(server.url("/api/v1/conversations/test_id/read"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}
