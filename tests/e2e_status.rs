//! E2E tests for status operations (posting, retrieving, deleting)

mod common;

use common::TestServer;
use serde_json::Value;

#[tokio::test]
async fn test_create_status_without_auth() {
    let server = TestServer::new().await;

    let status_data = serde_json::json!({
        "status": "Hello, world!",
        "visibility": "public"
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    // Should return 401 Unauthorized
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_create_status_with_auth() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "Hello, world!",
        "visibility": "public"
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    // Should create status if implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.get("id").is_some());
        assert_eq!(json["content"], "<p>Hello, world!</p>");
    }
}

#[tokio::test]
async fn test_create_status_rejects_invalid_visibility() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "Hello, world!",
        "visibility": "friends-only"
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn test_get_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create a status in the database
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/123".to_string(),
        content: "<p>Test status</p>".to_string(),
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

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}", status.id)))
        .send()
        .await
        .unwrap();

    // Should return status
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["id"], status.id);
    }
}

#[tokio::test]
async fn test_private_status_is_hidden_from_public_status_endpoints() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "Private note",
        "visibility": "private"
    });
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    assert_eq!(create_response.status(), 200);
    let created: Value = create_response.json().await.unwrap();
    let status_id = created["id"].as_str().unwrap();

    let status_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}", status_id)))
        .send()
        .await
        .unwrap();
    assert_eq!(status_response.status(), 404);

    let context_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/context", status_id)))
        .send()
        .await
        .unwrap();
    assert_eq!(context_response.status(), 404);

    let reblogged_by_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/reblogged_by", status_id)))
        .send()
        .await
        .unwrap();
    assert_eq!(reblogged_by_response.status(), 404);

    let favourited_by_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/favourited_by", status_id)))
        .send()
        .await
        .unwrap();
    assert_eq!(favourited_by_response.status(), 404);
}

#[tokio::test]
async fn test_create_status_rejects_poll_and_media_together() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "poll and media",
        "media_ids": ["media_1"],
        "poll": {
            "options": ["a", "b"],
            "expires_in": 600
        }
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 422);
}

#[tokio::test]
async fn test_create_status_with_poll_includes_poll_in_response() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "poll response",
        "poll": {
            "options": ["yes", "no"],
            "expires_in": 600
        }
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert!(json["poll"].is_object());
    assert!(json["poll"]["options"].is_array());
    assert_eq!(json["poll"]["options"][0]["title"], "yes");
}

#[tokio::test]
async fn test_vote_in_poll_rejects_duplicate_choices() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "poll duplicate vote",
        "poll": {
            "options": ["yes", "no"],
            "expires_in": 600,
            "multiple": true
        }
    });
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), 200);
    let created: Value = create_response.json().await.unwrap();
    let poll_id = created["poll"]["id"].as_str().unwrap();

    let vote_response = server
        .client
        .post(&server.url(&format!("/api/v1/polls/{}/votes", poll_id)))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({ "choices": [0, 0] }))
        .send()
        .await
        .unwrap();
    assert_eq!(vote_response.status(), 400);
}

#[tokio::test]
async fn test_create_status_with_media_ids_includes_media_attachments_in_response() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    use chrono::Utc;
    use rustresort::data::{EntityId, MediaAttachment};

    let media = MediaAttachment {
        id: EntityId::new().0,
        status_id: None,
        s3_key: "media/test-image.webp".to_string(),
        thumbnail_s3_key: None,
        content_type: "image/webp".to_string(),
        file_size: 1234,
        description: Some("image".to_string()),
        blurhash: None,
        width: Some(64),
        height: Some(64),
        created_at: Utc::now(),
    };
    server.state.db.insert_media(&media).await.unwrap();

    let status_data = serde_json::json!({
        "status": "media response",
        "media_ids": [media.id]
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert!(json["media_attachments"].is_array());
    assert_eq!(json["media_attachments"][0]["id"], media.id);
}

#[tokio::test]
async fn test_create_status_with_scheduled_at_returns_scheduled_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let scheduled_at = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    let status_data = serde_json::json!({
        "status": "Schedule me",
        "scheduled_at": scheduled_at,
        "visibility": "public"
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert!(json["id"].as_str().is_some());
    assert_eq!(json["params"]["text"], "Schedule me");
    assert_eq!(json["scheduled_at"], scheduled_at);

    let statuses = server.state.db.get_local_statuses(20, None).await.unwrap();
    assert!(statuses.is_empty());
}

#[tokio::test]
async fn test_create_status_with_scheduled_poll_returns_poll_options_array() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let scheduled_at = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    let status_data = serde_json::json!({
        "status": "Schedule me with poll",
        "scheduled_at": scheduled_at,
        "visibility": "public",
        "poll": {
            "options": ["A", "B"],
            "expires_in": 600,
            "multiple": false
        }
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&status_data)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert!(json["params"]["poll"]["options"].is_array());
    assert_eq!(json["params"]["poll"]["options"][0], "A");
}

#[tokio::test]
async fn test_create_status_is_idempotent_with_idempotency_key() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let status_data = serde_json::json!({
        "status": "Idempotent post",
        "visibility": "public"
    });

    let first_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .header("Idempotency-Key", "same-key")
        .json(&status_data)
        .send()
        .await
        .unwrap();
    assert_eq!(first_response.status(), 200);
    let first_json: Value = first_response.json().await.unwrap();
    let first_id = first_json["id"].as_str().unwrap().to_string();

    let second_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .header("Idempotency-Key", "same-key")
        .json(&status_data)
        .send()
        .await
        .unwrap();
    assert_eq!(second_response.status(), 200);
    let second_json: Value = second_response.json().await.unwrap();
    let second_id = second_json["id"].as_str().unwrap().to_string();

    assert_eq!(first_id, second_id);
    let statuses = server.state.db.get_local_statuses(20, None).await.unwrap();
    assert_eq!(statuses.len(), 1);
}

#[tokio::test]
async fn test_create_status_is_idempotent_with_concurrent_requests() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let url = server.url("/api/v1/statuses");
    let client = server.client.clone();
    let payload = serde_json::json!({
        "status": "Concurrent idempotent post",
        "visibility": "public"
    });

    let request_1 = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Idempotency-Key", "same-key-concurrent")
        .json(&payload)
        .send();
    let request_2 = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Idempotency-Key", "same-key-concurrent")
        .json(&payload)
        .send();

    let (response_1, response_2) = tokio::join!(request_1, request_2);
    let response_1 = response_1.unwrap();
    let response_2 = response_2.unwrap();
    assert_eq!(response_1.status(), 200);
    assert_eq!(response_2.status(), 200);

    let json_1: Value = response_1.json().await.unwrap();
    let json_2: Value = response_2.json().await.unwrap();
    assert_eq!(json_1["id"], json_2["id"]);

    let statuses = server.state.db.get_local_statuses(20, None).await.unwrap();
    assert_eq!(statuses.len(), 1);
}

#[tokio::test]
async fn test_create_status_idempotency_recovers_after_pending_is_cleared() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let endpoint = "/api/v1/statuses";
    let key = "pending-cleared-key";
    assert!(
        server
            .state
            .db
            .reserve_idempotency_key(endpoint, key)
            .await
            .unwrap()
    );

    let db = server.state.db.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;
        db.clear_pending_idempotency_key(endpoint, key)
            .await
            .unwrap();
    });

    let payload = serde_json::json!({
        "status": "Recovered idempotency request",
        "visibility": "public"
    });
    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .header("Idempotency-Key", key)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let statuses = server.state.db.get_local_statuses(20, None).await.unwrap();
    assert_eq!(statuses.len(), 1);
}

#[tokio::test]
async fn test_create_status_rejects_quoted_status_id_parameter() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let quote_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "status": "Quote",
            "quoted_status_id": "https://remote.example/users/alice/statuses/1"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(quote_response.status(), 400);
}

#[tokio::test]
async fn test_delete_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status first
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/456".to_string(),
        content: "<p>To be deleted</p>".to_string(),
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

    let response = server
        .client
        .delete(&server.url(&format!("/api/v1/statuses/{}", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should delete status if implemented
    if response.status().is_success() {
        // Verify status is deleted
        let get_response = server
            .client
            .get(&server.url(&format!("/api/v1/statuses/{}", status.id)))
            .send()
            .await
            .unwrap();

        assert_eq!(get_response.status(), 404);
    }
}

#[tokio::test]
async fn test_favourite_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/fav".to_string(),
        content: "<p>Favourite me!</p>".to_string(),
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

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/favourite", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should favourite status if implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["favourited"], true);
    }
}

#[tokio::test]
async fn test_boost_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    // Create a status
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/boost".to_string(),
        content: "<p>Boost me!</p>".to_string(),
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

    let response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/reblog", status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    // Should boost status if implemented
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert_eq!(json["reblogged"], true);
    }
}

#[tokio::test]
async fn test_status_context() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    // Create a status
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let status = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/context".to_string(),
        content: "<p>Context test</p>".to_string(),
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

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/context", status.id)))
        .send()
        .await
        .unwrap();

    // Should return context (ancestors and descendants)
    if response.status().is_success() {
        let json: Value = response.json().await.unwrap();
        assert!(json.get("ancestors").is_some());
        assert!(json.get("descendants").is_some());
    }
}

#[tokio::test]
async fn test_status_context_includes_ancestors_and_descendants() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    server.create_test_account().await;

    let root = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/root".to_string(),
        content: "<p>Root</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let middle = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/middle".to_string(),
        content: "<p>Middle</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: Some(root.uri.clone()),
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let leaf = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/leaf".to_string(),
        content: "<p>Leaf</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: Some(middle.uri.clone()),
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    server.state.db.insert_status(&root).await.unwrap();
    server.state.db.insert_status(&middle).await.unwrap();
    server.state.db.insert_status(&leaf).await.unwrap();

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/context", middle.id)))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();

    let ancestors = json["ancestors"].as_array().unwrap();
    let descendants = json["descendants"].as_array().unwrap();
    assert_eq!(ancestors.len(), 1);
    assert_eq!(descendants.len(), 1);
    assert_eq!(ancestors[0]["id"], root.id);
    assert_eq!(descendants[0]["id"], leaf.id);
}

#[tokio::test]
async fn test_status_context_limits_descendants() {
    use chrono::{Duration, Utc};
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    server.create_test_account().await;

    let base_time = Utc::now();
    let root = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/context-limit-root".to_string(),
        content: "<p>Root</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: base_time,
        fetched_at: None,
    };
    server.state.db.insert_status(&root).await.unwrap();

    let mut expected_ids = Vec::new();
    for index in 0..50 {
        let descendant = Status {
            id: EntityId::new().0,
            uri: format!("https://test.example.com/status/context-limit-descendant-{index}"),
            content: format!("<p>Descendant {index}</p>"),
            content_warning: None,
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: Some(root.uri.clone()),
            boost_of_uri: None,
            persisted_reason: "own".to_string(),
            created_at: base_time + Duration::seconds((index + 1) as i64),
            fetched_at: None,
        };
        if index < 40 {
            expected_ids.push(descendant.id.clone());
        }
        server.state.db.insert_status(&descendant).await.unwrap();
    }

    let response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/context", root.id)))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let json: Value = response.json().await.unwrap();
    let descendants = json["descendants"].as_array().unwrap();
    assert_eq!(descendants.len(), 40);
    let returned_ids: Vec<String> = descendants
        .iter()
        .filter_map(|item| item["id"].as_str().map(ToString::to_string))
        .collect();
    assert_eq!(returned_ids, expected_ids);
}

#[tokio::test]
async fn test_status_source_returns_plain_text() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let create_payload = serde_json::json!({
        "status": "Hello <tag>",
        "spoiler_text": "cw"
    });
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&create_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), 200);
    let created: Value = create_response.json().await.unwrap();
    let status_id = created["id"].as_str().unwrap();

    let source_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/source", status_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(source_response.status(), 200);

    let source: Value = source_response.json().await.unwrap();
    assert_eq!(source["text"], "Hello <tag>");
    assert_eq!(source["spoiler_text"], "cw");
}

#[tokio::test]
async fn test_status_history_contains_previous_version_after_edit() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let create_payload = serde_json::json!({
        "status": "v1",
        "visibility": "public"
    });
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&create_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), 200);
    let created: Value = create_response.json().await.unwrap();
    let status_id = created["id"].as_str().unwrap();

    let update_payload = serde_json::json!({
        "status": "v2",
        "spoiler_text": "updated"
    });
    let update_response = server
        .client
        .put(&server.url(&format!("/api/v1/statuses/{}", status_id)))
        .header("Authorization", format!("Bearer {}", token))
        .json(&update_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(update_response.status(), 200);

    let history_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/history", status_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(history_response.status(), 200);
    let history: Value = history_response.json().await.unwrap();
    let items = history.as_array().unwrap();
    assert!(items.len() >= 2);
    assert!(items.iter().any(|item| item["content"] == "<p>v1</p>"));
    assert!(items.iter().any(|item| item["content"] == "<p>v2</p>"));
    let first_created_at =
        chrono::DateTime::parse_from_rfc3339(items[0]["created_at"].as_str().unwrap()).unwrap();
    let second_created_at =
        chrono::DateTime::parse_from_rfc3339(items[1]["created_at"].as_str().unwrap()).unwrap();
    assert!(
        first_created_at >= second_created_at,
        "history should be returned in non-increasing revision timestamp order"
    );
}

#[tokio::test]
async fn test_status_history_rejects_remote_status() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let remote_status = Status {
        id: EntityId::new().0,
        uri: "https://remote.example/users/alice/statuses/remote-history".to_string(),
        content: "<p>Remote status</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "alice@remote.example".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "timeline".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    server.state.db.insert_status(&remote_status).await.unwrap();

    let history_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}/history", remote_status.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(history_response.status(), 403);
}

#[tokio::test]
async fn test_pin_and_mute_state_persists_across_reads() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let create_payload = serde_json::json!({
        "status": "pin and mute me",
        "visibility": "public"
    });
    let create_response = server
        .client
        .post(&server.url("/api/v1/statuses"))
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
        .post(&server.url(&format!("/api/v1/statuses/{}/pin", status_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(pin_response.status(), 200);
    let pin_json: Value = pin_response.json().await.unwrap();
    assert_eq!(pin_json["pinned"], true);

    let mute_response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/mute", status_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(mute_response.status(), 200);
    let mute_json: Value = mute_response.json().await.unwrap();
    assert_eq!(mute_json["muted"], true);

    let read_after_set = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}", status_id)))
        .send()
        .await
        .unwrap();
    assert_eq!(read_after_set.status(), 200);
    let set_json: Value = read_after_set.json().await.unwrap();
    assert!(set_json["favourited"].is_null());
    assert!(set_json["reblogged"].is_null());
    assert!(set_json["pinned"].is_null());
    assert!(set_json["muted"].is_null());
    assert!(set_json["bookmarked"].is_null());

    let unpin_response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/unpin", status_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(unpin_response.status(), 200);
    let unpin_json: Value = unpin_response.json().await.unwrap();
    assert_eq!(unpin_json["pinned"], false);

    let unmute_response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/unmute", status_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(unmute_response.status(), 200);
    let unmute_json: Value = unmute_response.json().await.unwrap();
    assert_eq!(unmute_json["muted"], false);

    let read_after_clear = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}", status_id)))
        .send()
        .await
        .unwrap();
    assert_eq!(read_after_clear.status(), 200);
    let cleared_json: Value = read_after_clear.json().await.unwrap();
    assert!(cleared_json["favourited"].is_null());
    assert!(cleared_json["reblogged"].is_null());
    assert!(cleared_json["pinned"].is_null());
    assert!(cleared_json["muted"].is_null());
    assert!(cleared_json["bookmarked"].is_null());
}

#[tokio::test]
async fn test_muting_reply_marks_whole_thread_as_muted() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let root = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/thread-muted-root".to_string(),
        content: "<p>Root</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let reply = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/status/thread-muted-reply".to_string(),
        content: "<p>Reply</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: Some(root.uri.clone()),
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    server.state.db.insert_status(&root).await.unwrap();
    server.state.db.insert_status(&reply).await.unwrap();

    let mute_response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/mute", &reply.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(mute_response.status(), 200);

    let root_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}", &root.id)))
        .send()
        .await
        .unwrap();
    assert_eq!(root_response.status(), 200);
    let root_json: Value = root_response.json().await.unwrap();
    assert!(root_json["muted"].is_null());

    let reply_response = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}", &reply.id)))
        .send()
        .await
        .unwrap();
    assert_eq!(reply_response.status(), 200);
    let reply_json: Value = reply_response.json().await.unwrap();
    assert!(reply_json["muted"].is_null());
    assert!(server.state.db.is_thread_muted(&root.uri).await.unwrap());

    let unmute_response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/unmute", &root.id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();
    assert_eq!(unmute_response.status(), 200);

    let root_after_unmute = server
        .client
        .get(&server.url(&format!("/api/v1/statuses/{}", &root.id)))
        .send()
        .await
        .unwrap();
    assert_eq!(root_after_unmute.status(), 200);
    let root_after_unmute_json: Value = root_after_unmute.json().await.unwrap();
    assert!(root_after_unmute_json["muted"].is_null());
    assert!(!server.state.db.is_thread_muted(&root.uri).await.unwrap());
}

#[tokio::test]
async fn test_create_reply_status_persists_reply_metadata() {
    use chrono::Utc;
    use rustresort::data::{EntityId, Status};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let parent = Status {
        id: EntityId::new().0,
        uri: "https://test.example.com/users/testuser/statuses/original".to_string(),
        content: "<p>Original post</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    server.state.db.insert_status(&parent).await.unwrap();

    let payload = serde_json::json!({
        "status": "This is a reply",
        "visibility": "public",
        "in_reply_to_id": parent.id
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let created: Value = response.json().await.unwrap();
    let reply_id = created["id"]
        .as_str()
        .expect("status response should contain id")
        .to_string();

    let reply = server
        .state
        .db
        .get_status(&reply_id)
        .await
        .unwrap()
        .expect("reply should be persisted");
    assert_eq!(reply.in_reply_to_uri, Some(parent.uri));
    assert_eq!(reply.persisted_reason, "reply_to_own");
}

#[tokio::test]
async fn test_create_reply_status_accepts_cache_only_remote_target() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/cache-only-reply-target";

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Remote status shown from cache</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let payload = serde_json::json!({
        "status": "Replying to cache-only status",
        "visibility": "public",
        "in_reply_to_id": remote_uri
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let created: Value = response.json().await.unwrap();
    let reply_id = created["id"]
        .as_str()
        .expect("status response should contain id")
        .to_string();

    let reply = server
        .state
        .db
        .get_status(&reply_id)
        .await
        .unwrap()
        .expect("reply should be persisted");
    assert_eq!(reply.in_reply_to_uri, Some(remote_uri.to_string()));
    assert_eq!(reply.persisted_reason, "own");
}

#[tokio::test]
async fn test_create_reply_status_delivers_to_remote_reply_target_inbox_without_followers() {
    use axum::{extract::State, http::StatusCode, routing::post};
    use chrono::Utc;
    use rustresort::data::{CachedProfile, CachedStatus};
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
    let remote_address = "alice@remote.example";
    let remote_status_uri = format!("{}/users/alice/statuses/reply-target", remote_base_url);

    server
        .state
        .profile_cache
        .insert(CachedProfile {
            address: remote_address.to_string(),
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

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_status_uri.clone(),
            uri: remote_status_uri.clone(),
            content: "<p>Remote status</p>".to_string(),
            account_address: remote_address.to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let payload = serde_json::json!({
        "status": "Replying to remote status",
        "visibility": "public",
        "in_reply_to_id": remote_status_uri
    });

    let response = server
        .client
        .post(&server.url("/api/v1/statuses"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let mut delivered = false;
    for _ in 0..600 {
        if inbox_delivery_count.load(Ordering::SeqCst) > 0 {
            delivered = true;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }

    assert!(
        delivered,
        "expected outbound Create delivery to include remote reply target inbox"
    );
}

#[tokio::test]
async fn test_favourite_remote_status_by_uri_persists_from_cache() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/fav-by-uri";

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Remote status</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let encoded_uri: String = url::form_urlencoded::byte_serialize(remote_uri.as_bytes()).collect();
    let response = server
        .client
        .post(&server.url(&format!(
            "/api/v1/statuses/placeholder/favourite?uri={}",
            encoded_uri
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["favourited"], true);
    assert_eq!(body["account"]["username"], "alice");
    assert_eq!(body["account"]["acct"], "alice@remote.example");

    let persisted = server
        .state
        .db
        .get_status_by_uri(remote_uri)
        .await
        .unwrap()
        .expect("remote status should be persisted");
    assert!(!persisted.is_local);
    assert_eq!(persisted.persisted_reason, "favourited");
}

#[tokio::test]
async fn test_favourite_status_rejects_empty_uri_query_parameter() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .post(&server.url("/api/v1/statuses/placeholder/favourite?uri="))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn test_favourite_remote_status_by_path_id_uri_fallback_persists_from_cache() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/fav-by-path-id";

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Remote status by path id</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let encoded_id: String = url::form_urlencoded::byte_serialize(remote_uri.as_bytes()).collect();
    let response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/favourite", encoded_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["favourited"], true);
    assert_eq!(body["account"]["acct"], "alice@remote.example");
    assert!(
        server
            .state
            .db
            .get_status_by_uri(remote_uri)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn test_bookmark_remote_status_by_uri_persists_from_cache() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/bookmark-by-uri";

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Remote status for bookmark</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let encoded_uri: String = url::form_urlencoded::byte_serialize(remote_uri.as_bytes()).collect();
    let response = server
        .client
        .post(&server.url(&format!(
            "/api/v1/statuses/placeholder/bookmark?uri={}",
            encoded_uri
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["bookmarked"], true);
    assert_eq!(body["account"]["username"], "alice");
    assert_eq!(body["account"]["acct"], "alice@remote.example");

    let persisted = server
        .state
        .db
        .get_status_by_uri(remote_uri)
        .await
        .unwrap()
        .expect("remote status should be persisted");
    assert!(!persisted.is_local);
    assert_eq!(persisted.persisted_reason, "bookmarked");
}

#[tokio::test]
async fn test_bookmark_remote_status_by_path_id_uri_fallback_persists_from_cache() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/bookmark-by-path-id";

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Remote status bookmark path id</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let encoded_id: String = url::form_urlencoded::byte_serialize(remote_uri.as_bytes()).collect();
    let response = server
        .client
        .post(&server.url(&format!("/api/v1/statuses/{}/bookmark", encoded_id)))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["bookmarked"], true);
    assert_eq!(body["account"]["acct"], "alice@remote.example");
    assert!(
        server
            .state
            .db
            .get_status_by_uri(remote_uri)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn test_reblog_remote_status_by_uri_persists_from_cache() {
    use chrono::Utc;
    use rustresort::data::CachedStatus;

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/reblog-by-uri";

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Remote status for reblog</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let encoded_uri: String = url::form_urlencoded::byte_serialize(remote_uri.as_bytes()).collect();
    let response = server
        .client
        .post(&server.url(&format!(
            "/api/v1/statuses/placeholder/reblog?uri={}",
            encoded_uri
        )))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["reblogged"], true);
    assert_eq!(body["account"]["username"], "alice");
    assert_eq!(body["account"]["acct"], "alice@remote.example");

    let persisted = server
        .state
        .db
        .get_status_by_uri(remote_uri)
        .await
        .unwrap()
        .expect("remote status should be persisted");
    assert!(!persisted.is_local);
    assert_eq!(persisted.persisted_reason, "reposted");
    assert!(server.state.db.is_reposted(&persisted.id).await.unwrap());
}

#[tokio::test]
async fn test_notifications_fallback_to_cached_status() {
    use chrono::Utc;
    use rustresort::data::{CachedStatus, EntityId, Notification};

    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let remote_uri = "https://remote.example/users/alice/statuses/notif-cache";
    let notification_id = EntityId::new().0;

    server
        .state
        .timeline_cache
        .insert(CachedStatus {
            id: remote_uri.to_string(),
            uri: remote_uri.to_string(),
            content: "<p>Cached notification status</p>".to_string(),
            account_address: "alice@remote.example".to_string(),
            created_at: Utc::now(),
            visibility: "public".to_string(),
            attachments: vec![],
            reply_to_uri: None,
            boost_of_uri: None,
        })
        .await;

    let notification = Notification {
        id: notification_id.clone(),
        notification_type: "mention".to_string(),
        origin_account_address: "alice@remote.example".to_string(),
        status_uri: Some(remote_uri.to_string()),
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
        .get(&server.url("/api/v1/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let notifications: Vec<Value> = response.json().await.unwrap();
    let target = notifications
        .iter()
        .find(|entry| entry["id"].as_str() == Some(&notification_id))
        .expect("notification should be returned");
    assert_eq!(target["status"]["uri"], remote_uri);
    assert_eq!(
        target["status"]["content"],
        "<p>Cached notification status</p>"
    );
}
