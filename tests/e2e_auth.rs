//! E2E tests for built-in local auth endpoints.

mod common;

use chrono::Utc;
use common::TestServer;
use reqwest::StatusCode;
use rustresort::data::{EntityId, PasskeyCredential};
use serde_json::json;

fn no_redirect_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("failed to build no-redirect client")
}

#[tokio::test]
async fn test_login_page_renders_local_auth_options() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(server.url("/login"))
        .send()
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("response body");
    assert!(body.contains("Sign in with password"));
    assert!(body.contains("Sign in with passkey"));
}

#[tokio::test]
async fn test_password_login_returns_bearer_token_and_sets_session_cookie() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .post(server.url("/auth/login"))
        .json(&json!({
            "username": "testuser",
            "password": "test-password"
        }))
        .send()
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .expect("session cookie header");
    assert!(set_cookie.contains("session="));

    let body: serde_json::Value = response.json().await.expect("login json");
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["username"], "testuser");
    assert_eq!(body["auth_method"], "password");
    assert!(body["access_token"].as_str().is_some());
}

#[tokio::test]
async fn test_first_start_bootstraps_local_account_from_config() {
    let server = TestServer::new_unseeded().await;

    let response = server
        .client
        .post(server.url("/auth/login"))
        .json(&json!({
            "username": "testuser",
            "password": "test-password"
        }))
        .send()
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_password_login_rejects_bad_password() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .post(server.url("/auth/login"))
        .json(&json!({
            "username": "testuser",
            "password": "wrong-password"
        }))
        .send()
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_authenticated_session_endpoint_returns_local_session() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let response = server
        .client
        .get(server.url("/auth/session"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("session json");
    assert_eq!(body["username"], "testuser");
}

#[tokio::test]
async fn test_settings_page_is_available_with_session_auth() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let login = server
        .client
        .post(server.url("/auth/login"))
        .json(&json!({
            "username": "testuser",
            "password": "test-password"
        }))
        .send()
        .await
        .expect("login succeeds");
    let session_cookie = login
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .expect("session cookie header");

    let response = server
        .client
        .get(server.url("/settings"))
        .header("Cookie", session_cookie)
        .send()
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("html body");
    assert!(body.contains("User Settings"));
    assert!(body.contains("Manage your local profile, password, and passkeys."));
}

#[tokio::test]
async fn test_settings_page_redirects_to_login_without_cookie() {
    let server = TestServer::new().await;
    let client = no_redirect_client();

    let response = client
        .get(server.url("/settings"))
        .send()
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some("/login")
    );
}

#[tokio::test]
async fn test_integrated_ui_route_is_not_served_when_disabled() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(server.url("/ui"))
        .send()
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_integrated_ui_route_serves_shell_when_enabled() {
    let server = TestServer::with_ui_enabled().await;

    let response = server
        .client
        .get(server.url("/ui"))
        .send()
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("html body");
    assert!(body.contains("Loading RustResort UI"));
    assert!(body.contains("/ui/rustresort_ui.js"));
}

#[tokio::test]
async fn test_passkey_registration_start_requires_authenticated_session() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let unauthorized = server
        .client
        .post(server.url("/auth/passkeys/register/start"))
        .send()
        .await
        .expect("request succeeds");
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let token = server.create_test_token().await;
    let authorized = server
        .client
        .post(server.url("/auth/passkeys/register/start"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request succeeds");
    assert_eq!(authorized.status(), StatusCode::OK);

    let body: serde_json::Value = authorized.json().await.expect("challenge json");
    assert!(body["request_id"].as_str().is_some());
    assert!(body["publicKey"]["challenge"].as_str().is_some());
}

#[tokio::test]
async fn test_password_change_updates_login_secret() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;

    let change = server
        .client
        .post(server.url("/auth/password"))
        .bearer_auth(&token)
        .json(&json!({
            "current_password": "test-password",
            "new_password": "new-test-password"
        }))
        .send()
        .await
        .expect("password change succeeds");
    assert_eq!(change.status(), StatusCode::NO_CONTENT);

    let old_login = server
        .client
        .post(server.url("/auth/login"))
        .json(&json!({
            "username": "testuser",
            "password": "test-password"
        }))
        .send()
        .await
        .expect("old login request succeeds");
    assert_eq!(old_login.status(), StatusCode::UNAUTHORIZED);

    let new_login = server
        .client
        .post(server.url("/auth/login"))
        .json(&json!({
            "username": "testuser",
            "password": "new-test-password"
        }))
        .send()
        .await
        .expect("new login request succeeds");
    assert_eq!(new_login.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_passkey_name_can_be_updated() {
    let server = TestServer::new().await;
    let token = server.create_test_token().await;
    let now = Utc::now();
    let passkey = PasskeyCredential {
        id: EntityId::new_string(),
        credential_id: "credential-1".to_string(),
        name: Some("Old name".to_string()),
        passkey_json: "{}".to_string(),
        created_at: now,
        updated_at: now,
        last_used_at: None,
    };
    server.state.db.insert_passkey(&passkey).await.unwrap();

    let response = server
        .client
        .patch(server.url(&format!("/auth/passkeys/{}", passkey.id)))
        .bearer_auth(&token)
        .json(&json!({
            "name": "Desk key"
        }))
        .send()
        .await
        .expect("rename request succeeds");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("rename response json");
    assert_eq!(body["name"], "Desk key");

    let stored = server
        .state
        .db
        .get_passkey_by_id(&passkey.id)
        .await
        .unwrap()
        .expect("stored passkey");
    assert_eq!(stored.name.as_deref(), Some("Desk key"));
}

#[tokio::test]
async fn test_logout_clears_session_cookie() {
    let server = TestServer::new().await;
    let client = no_redirect_client();

    let response = client
        .post(server.url("/logout"))
        .header("Cookie", "session=dummy-session")
        .send()
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let set_cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .expect("set-cookie header");
    assert!(set_cookie.contains("session="));
}
