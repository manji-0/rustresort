mod common;

use common::TestServer;
use reqwest::StatusCode;
use serde_json::json;

#[tokio::test]
async fn test_broad_oauth_bearer_can_post_status_and_follow() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let create_status = server
        .client
        .post(server.url("/api/v1/statuses"))
        .bearer_auth(&token)
        .json(&json!({ "status": "broad oauth write is allowed" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_status.status(), StatusCode::OK);

    let follow = server
        .client
        .post(server.url("/api/v1/accounts/alice@remote.example/follow"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(follow.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_broad_oauth_bearer_can_access_streaming_routes_with_required_scopes() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let user_stream = server
        .client
        .get(server.url("/api/v1/streaming/user"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(user_stream.status(), StatusCode::OK);

    let direct_stream = server
        .client
        .get(server.url("/api/v1/streaming/direct"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(direct_stream.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_oauth_read_only_token_cannot_post_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server
        .create_oauth_authorization_code_token("read:accounts read:statuses")
        .await;

    let create_status = server
        .client
        .post(server.url("/api/v1/statuses"))
        .bearer_auth(&token)
        .json(&json!({ "status": "this should be forbidden" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_status.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_oauth_write_statuses_token_can_post_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server
        .create_oauth_authorization_code_token("read:accounts write:statuses")
        .await;

    let create_status = server
        .client
        .post(server.url("/api/v1/statuses"))
        .bearer_auth(&token)
        .json(&json!({ "status": "oauth write token works" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_status.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_oauth_profile_scope_can_verify_credentials() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server
        .create_oauth_authorization_code_token("profile")
        .await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/verify_credentials"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_oauth_user_stream_requires_all_user_stream_scopes() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let insufficient = server
        .create_oauth_authorization_code_token("read:statuses")
        .await;
    let sufficient = server
        .create_oauth_authorization_code_token("read:statuses read:notifications")
        .await;

    let denied = server
        .client
        .get(server.url("/api/v1/streaming/user"))
        .bearer_auth(&insufficient)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let allowed = server
        .client
        .get(server.url("/api/v1/streaming/user"))
        .bearer_auth(&sufficient)
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_push_subscription_requires_push_scope() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let denied = server
        .create_oauth_authorization_code_token("read:notifications")
        .await;
    let allowed = server.create_oauth_authorization_code_token("push").await;
    let payload = json!({
        "subscription": {
            "endpoint": "https://push.example/subscriptions/1",
            "keys": {
                "p256dh": "BEl6x8m6S1zJ7_P6M9_yi4M0Z3nJmN9t67wS0JYw1G4qj1bRk7a8E5zv8i3Q1nD6hM2qT2Q0wI9nQdFQ0Q4j1sA",
                "auth": "m5vD8f3U1nQ2mA4b7cD9eQ"
            }
        },
        "data": {
            "alerts": {
                "mention": true
            },
            "policy": "all"
        }
    });

    let denied_response = server
        .client
        .post(server.url("/api/v1/push/subscription"))
        .bearer_auth(&denied)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(denied_response.status(), StatusCode::FORBIDDEN);

    let allowed_response = server
        .client
        .post(server.url("/api/v1/push/subscription"))
        .bearer_auth(&allowed)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(allowed_response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_follow_requests_use_follow_read_scope() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    server
        .state
        .db
        .insert_follow_request(
            "alice@remote.example",
            "https://remote.example/inbox",
            "https://remote.example/follows/req-1",
        )
        .await
        .unwrap();

    let denied = server
        .create_oauth_authorization_code_token("read:accounts")
        .await;
    let allowed = server
        .create_oauth_authorization_code_token("read:follows")
        .await;

    let denied_response = server
        .client
        .get(server.url("/api/v1/follow_requests"))
        .bearer_auth(&denied)
        .send()
        .await
        .unwrap();
    assert!(denied_response.status().is_success());

    let allowed_response = server
        .client
        .get(server.url("/api/v1/follow_requests"))
        .bearer_auth(&allowed)
        .send()
        .await
        .unwrap();
    assert_eq!(allowed_response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_invalid_bearer_token_is_rejected() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/verify_credentials"))
        .bearer_auth("not-a-valid-local-session")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_cookie_session_authenticates_browser_api_requests() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let (_, session_cookie) = server.login_password().await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/verify_credentials"))
        .header("Cookie", session_cookie)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_passkey_registration_listing_starts_empty() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_session_token().await;

    let response = server
        .client
        .get(server.url("/auth/passkeys"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn test_client_credentials_token_can_verify_app_but_cannot_access_user_endpoints() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server
        .create_oauth_client_credentials_token("read:accounts write:statuses")
        .await;

    let app_verify = server
        .client
        .get(server.url("/api/v1/apps/verify_credentials"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(app_verify.status(), StatusCode::OK);

    let verify = server
        .client
        .get(server.url("/api/v1/accounts/verify_credentials"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(verify.status(), StatusCode::FORBIDDEN);

    let create_status = server
        .client
        .post(server.url("/api/v1/statuses"))
        .bearer_auth(&token)
        .json(&json!({ "status": "client credentials must not post" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_status.status(), StatusCode::FORBIDDEN);
}
