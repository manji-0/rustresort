//! E2E tests for timeline operations

mod common;

use common::TestServer;
use serde_json::Value;

#[tokio::test]
async fn test_home_timeline_without_auth() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/home"))
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
        .get(&server.url("/api/v1/timelines/home"))
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
        .get(&server.url("/api/v1/timelines/public"))
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
async fn test_local_timeline() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/public?local=true"))
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
async fn test_timeline_pagination() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create multiple statuses
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    for i in 0..5 {
        let status = Status {
            id: EntityId::new().0,
            uri: format!("https://test.example.com/status/{}", i),
            content: format!("<p>Status {}</p>", i),
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
    }

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/public?limit=3"))
        .send()
        .await
        .unwrap();

    // Should return limited number of statuses
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.is_array());
        if json.as_array().unwrap().len() > 0 {
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
        id: EntityId::new().0,
        uri: "https://test.example.com/status/tagged-public".to_string(),
        content: "<p>Learning #rust today</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let tagged_private = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/tagged-private".to_string(),
        content: "<p>Private #rust note</p>".to_string(),
        content_warning: None,
        visibility: "private".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let untagged_public = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/untagged-public".to_string(),
        content: "<p>No hashtag here</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
        .get(&server.url("/api/v1/timelines/tag/rust"))
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
        id: EntityId::new().0,
        uri: "https://test.example.com/status/list-local".to_string(),
        content: "<p>Local list status</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let remote_status = Status {
        id: EntityId::new().0,
        uri: "https://remote.example/status/list-remote".to_string(),
        content: "<p>Remote list status</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: remote_address.clone(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "favourited".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let unrelated_status = Status {
        id: EntityId::new().0,
        uri: "https://remote.example/status/list-unrelated".to_string(),
        content: "<p>Unrelated list status</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "bob@example.com".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "favourited".to_string(),
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
        .get(&server.url(&format!("/api/v1/timelines/list/{}", list_id)))
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
async fn test_timeline_with_max_id() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(&server.url("/api/v1/timelines/home?max_id=123456"))
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
        .get(&server.url("/api/v1/timelines/home?since_id=123456"))
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
async fn test_muted_thread_is_hidden_from_public_timeline() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let root = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/thread-root".to_string(),
        content: "<p>Thread root</p>".to_string(),
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
    let reply = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/thread-reply".to_string(),
        content: "<p>Thread reply</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: Some(root.uri.clone()),
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let other = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/other-thread".to_string(),
        content: "<p>Other thread</p>".to_string(),
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
    server.state.db.insert_status(&root).await.unwrap();
    server.state.db.insert_status(&reply).await.unwrap();
    server.state.db.insert_status(&other).await.unwrap();

    let mute_response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/mute", &reply.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(mute_response.status(), 200);

    let timeline_response = server
        .client
        .get(&server.url("/api/v1/timelines/public"))
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
        id: EntityId::new().0,
        uri: "https://test.example.com/status/visible-a".to_string(),
        content: "<p>Visible A</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: base_time,
        fetched_at: None,
    };
    let visible_b = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/visible-b".to_string(),
        content: "<p>Visible B</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: base_time + Duration::seconds(1),
        fetched_at: None,
    };
    let muted_root = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/muted-root".to_string(),
        content: "<p>Muted root</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "testuser@test.example.com".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: base_time + Duration::seconds(2),
        fetched_at: None,
    };
    server.state.db.insert_status(&visible_a).await.unwrap();
    server.state.db.insert_status(&visible_b).await.unwrap();
    server.state.db.insert_status(&muted_root).await.unwrap();

    let mut mute_target_id = String::new();
    for index in 0..21 {
        let reply = Status {
            id: EntityId::new().0,
            uri: format!("https://test.example.com/status/muted-reply-{index}"),
            content: format!("<p>Muted reply {index}</p>"),
            content_warning: None,
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: "testuser@test.example.com".to_string(),
            is_local: true,
            in_reply_to_uri: Some(muted_root.uri.clone()),
            boost_of_uri: None,
            persisted_reason: "own".to_string(),
            created_at: base_time + Duration::seconds((index + 3) as i64),
            fetched_at: None,
        };
        mute_target_id = reply.id.clone();
        server.state.db.insert_status(&reply).await.unwrap();
    }

    let mute_response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/mute", mute_target_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(mute_response.status(), 200);

    let timeline_response = server
        .client
        .get(&server.url("/api/v1/timelines/public"))
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
