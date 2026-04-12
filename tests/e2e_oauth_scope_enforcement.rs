mod common;

use common::TestServer;
use reqwest::StatusCode;
use serde_json::json;

#[tokio::test]
async fn test_local_session_bearer_can_post_status_and_follow() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let create_status = server
        .client
        .post(server.url("/api/v1/statuses"))
        .bearer_auth(&token)
        .json(&json!({ "status": "local session write is allowed" }))
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
async fn test_local_session_bearer_can_access_streaming_routes_without_oauth_scopes() {
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
async fn test_password_login_and_cookie_session_both_authenticate() {
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
    let token = server.create_test_token().await;

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
