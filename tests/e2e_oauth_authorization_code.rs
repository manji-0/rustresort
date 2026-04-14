mod common;

use common::TestServer;
use reqwest::StatusCode;

#[tokio::test]
async fn test_oauth_authorize_redirects_to_login_when_session_is_missing() {
    let server = TestServer::new().await;
    let app = server
        .create_oauth_app("https://client.example/callback", "read:accounts")
        .await;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(server.url("/oauth/authorize"))
        .query(&[
            ("response_type", "code"),
            ("client_id", app["client_id"].as_str().unwrap()),
            ("redirect_uri", "https://client.example/callback"),
            ("scope", "read:accounts"),
            ("state", "oauth-state"),
        ])
        .send()
        .await
        .unwrap();

    assert!(
        response.status() == StatusCode::FOUND || response.status() == StatusCode::SEE_OTHER,
        "expected login redirect, got {}",
        response.status()
    );
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .unwrap();
    assert!(location.starts_with("/login?next="));
    assert!(location.contains("response_type%3Dcode"));
}

#[tokio::test]
async fn test_oauth_authorization_code_flow_works_end_to_end() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let access_token = server
        .create_oauth_authorization_code_token("read:accounts write:statuses")
        .await;

    let verify = server
        .client
        .get(server.url("/api/v1/accounts/verify_credentials"))
        .bearer_auth(&access_token)
        .send()
        .await
        .unwrap();
    assert_eq!(verify.status(), StatusCode::OK);

    let app_verify = server
        .client
        .get(server.url("/api/v1/apps/verify_credentials"))
        .bearer_auth(&access_token)
        .send()
        .await
        .unwrap();
    assert_eq!(app_verify.status(), StatusCode::OK);
    let app_body = app_verify.json::<serde_json::Value>().await.unwrap();
    assert_eq!(app_body["name"], "RustResort E2E Client");
    assert_eq!(app_body["scopes"], "read:accounts write:statuses");
}

#[tokio::test]
async fn test_oauth_oob_authorization_returns_code_page() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let app = server
        .create_oauth_app("urn:ietf:wg:oauth:2.0:oob", "read:accounts")
        .await;
    let (_, session_cookie) = server.login_password().await;

    let response = server
        .client
        .get(server.url("/oauth/authorize"))
        .query(&[
            ("response_type", "code"),
            ("client_id", app["client_id"].as_str().unwrap()),
            ("redirect_uri", "urn:ietf:wg:oauth:2.0:oob"),
            ("scope", "read:accounts"),
        ])
        .header("Cookie", session_cookie)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.unwrap();
    assert!(body.contains("Authorization Complete"));
    assert!(body.contains("RustResort E2E Client"));
}

#[tokio::test]
async fn test_public_account_creation_endpoint_is_not_exposed() {
    let server = TestServer::new().await;

    let response = server
        .client
        .post(server.url("/api/v1/accounts"))
        .json(&serde_json::json!({
            "username": "other-user",
            "password": "password123"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_password_login_token_cannot_access_mastodon_api_as_bearer() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let (token, _) = server.login_password().await;
    let verify = server
        .client
        .get(server.url("/api/v1/accounts/verify_credentials"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(verify.status(), StatusCode::UNAUTHORIZED);
}
