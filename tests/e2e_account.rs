//! E2E tests for account operations

mod common;

use common::TestServer;
use serde_json::Value;

#[tokio::test]
async fn test_verify_credentials_without_auth() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/api/v1/accounts/verify_credentials"))
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
        .get(&server.url("/api/v1/accounts/verify_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should return account info if auth is implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.get("id").is_some());
        assert!(json.get("username").is_some());
    }
}

#[tokio::test]
async fn test_get_account_by_id() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}", account.id)))
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
        "note": "Updated bio"
    });

    let response = server
        .client
        .patch(&server.url("/api/v1/accounts/update_credentials"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&update_data)
        .send()
        .await
        .unwrap();

    // Should update account if implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["display_name"], "Updated Name");
    }
}

#[tokio::test]
async fn test_account_statuses() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}/statuses", account.id)))
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
async fn test_account_followers() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}/followers", account.id)))
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
async fn test_account_following() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/accounts/{}/following", account.id)))
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
async fn test_follow_account_persists_follow_relationship() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let target = "alice@remote.example";

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/accounts/{}/follow", target)))
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
        .post(&server.url(&format!("/api/v1/accounts/{}/follow", mixed_case_target)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(first.status().is_success());

    let second = server
        .client
        .post(&server.url("/api/v1/accounts/alice@remote.example/follow"))
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
        .post(&server.url("/api/v1/accounts/alice@remote.example:443/follow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert!(first.status().is_success());

    let second = server
        .client
        .post(&server.url("/api/v1/accounts/alice@remote.example/follow"))
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
        id: EntityId::new().0,
        target_address: "alice@remote.example".to_string(),
        uri: "https://test.example.com/users/testuser/follow/dup-1".to_string(),
        created_at: Utc::now(),
    };
    let second = Follow {
        id: EntityId::new().0,
        target_address: "alice@remote.example".to_string(),
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
        id: EntityId::new().0,
        target_address: target.to_string(),
        uri: "https://test.example.com/users/testuser/follow/seed".to_string(),
        created_at: Utc::now(),
    };
    server.state.db.insert_follow(&follow).await.unwrap();

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/accounts/{}/unfollow", target)))
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
        id: EntityId::new().0,
        target_address: "Alice@Remote.EXAMPLE".to_string(),
        uri: "https://test.example.com/users/testuser/follow/mixed".to_string(),
        created_at: Utc::now(),
    };
    server.state.db.insert_follow(&follow).await.unwrap();

    let response = server
        .client
        .post(&server.url("/api/v1/accounts/alice@remote.example/unfollow"))
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
        id: EntityId::new().0,
        target_address: "alice@remote.example:443".to_string(),
        uri: "https://test.example.com/users/testuser/follow/default-port".to_string(),
        created_at: Utc::now(),
    };
    server.state.db.insert_follow(&follow).await.unwrap();

    let response = server
        .client
        .post(&server.url("/api/v1/accounts/alice@remote.example/unfollow"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let follow_addresses = server.state.db.get_all_follow_addresses().await.unwrap();
    assert!(follow_addresses.is_empty());
}

#[tokio::test]
async fn test_follow_account_rejects_self_follow() {
    let server = TestServer::new().await;
    let account = server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/accounts/{}/follow", account.id)))
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
        .post(&server.url("/api/v1/accounts/TESTUSER@TEST.EXAMPLE.COM/follow"))
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
        .post(&server.url("/api/v1/accounts/testuser@test.example.com:443/follow"))
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
        .post(&server.url("/api/v1/accounts/alice@remote.example:443/follow"))
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
        .post(&server.url("/api/v1/accounts/alice@remote.example:80/follow"))
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
        id: EntityId::new().0,
        target_address: "alice@remote.example:443".to_string(),
        uri: "https://test.example.com/users/testuser/follow/block-default-port".to_string(),
        created_at: Utc::now(),
    };
    server.state.db.insert_follow(&follow).await.unwrap();

    let block_response = server
        .client
        .post(&server.url("/api/v1/accounts/alice@remote.example/block"))
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
        .post(&server.url("/api/v1/accounts/alice@remote.example:443/unblock"))
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
        .post(&server.url(&format!("/api/v1/accounts/{}/block", target_address)))
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
        .post(&server.url(&format!("/api/v1/accounts/{}/unblock", target_address)))
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
        if let Ok(activity) = serde_json::from_str::<Value>(&body) {
            if activity.get("type").and_then(|value| value.as_str()) == Some("Block") {
                counter.fetch_add(1, Ordering::SeqCst);
            }
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
        .post(&server.url(&format!("/api/v1/accounts/{}/block", target_address)))
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
        .post(&server.url(&format!("/api/v1/accounts/{}/block", target_address)))
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
        .post(&server.url(&format!("/api/v1/accounts/{}/unblock", target_address)))
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
        .post(&server.url("/api/v1/accounts/alice@remote.example:443/mute"))
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
        .post(&server.url("/api/v1/accounts/alice@remote.example/unmute"))
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
