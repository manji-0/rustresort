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

// ============================================================================
// Apps Endpoints (2 endpoints)
// ============================================================================

#[tokio::test]
async fn test_create_app() {
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
}

#[tokio::test]
async fn test_verify_app_credentials() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/apps/verify_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(
        body.get("client_id").is_none(),
        "session fallback must not leak unrelated app credentials"
    );
}

// ============================================================================
// Account Endpoints (20+ endpoints)
// ============================================================================

#[tokio::test]
async fn test_create_account() {
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

    assert!(response.status().is_success() || response.status() == 422);
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
    assert_ne!(second_id, first_id);
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
    assert_ne!(second_id, first_id);
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
    assert_ne!(second_id, first_id);
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
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url("/api/v1/notifications/test_id/dismiss"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success() || response.status() == 404);
}

#[tokio::test]
async fn test_clear_notifications() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(server.url("/api/v1/notifications/clear"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_get_unread_count() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/api/v1/notifications/unread_count"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
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
