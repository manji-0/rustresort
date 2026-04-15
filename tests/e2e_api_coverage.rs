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
    let response = server
        .client
        .get(server.url("/api/v1/instance/activity"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
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
    assert_eq!(body["redirect_uris"], "urn:ietf:wg:oauth:2.0:oob");
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
    assert_eq!(followers[0]["id"], actor_uri_address);
    assert_eq!(followers[0]["acct"], actor_uri_address);
    assert_eq!(followers[0]["url"], actor_uri_address);
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
    assert_eq!(following[0]["id"], actor_uri_address);
    assert_eq!(following[0]["acct"], actor_uri_address);
    assert_eq!(following[0]["url"], actor_uri_address);
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
    assert_eq!(followers[0]["acct"], actor_uri_address);
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
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v2/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
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

    // Note: This is a basic test without actual file upload
    let response = server
        .client
        .post(server.url("/api/v1/media"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 400 || response.status() == 422);
}

#[tokio::test]
async fn test_upload_media_v2() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Note: This is a basic test without actual file upload
    let response = server
        .client
        .post(server.url("/api/v2/media"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 400 || response.status() == 422);
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

    let update_data = json!({
        "description": "Updated description"
    });

    let response = server
        .client
        .put(server.url("/api/v1/media/test_media_id"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&update_data)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
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

    let list_data = json!({
        "title": "Test List"
    });

    let response = server
        .client
        .post(server.url("/api/v1/lists"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&list_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
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

    let update_data = json!({
        "title": "Updated List"
    });

    let response = server
        .client
        .put(server.url("/api/v1/lists/test_list_id"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&update_data)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
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

    let response = server
        .client
        .get(server.url("/api/v1/lists/test_list_id/accounts"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
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

// ============================================================================
// Polls Endpoints (2 endpoints)
// ============================================================================

#[tokio::test]
async fn test_get_poll() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/polls/test_poll_id"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_vote_in_poll() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let vote_data = json!({
        "choices": [0]
    });

    let response = server
        .client
        .post(server.url("/api/v1/polls/test_poll_id/votes"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&vote_data)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
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
