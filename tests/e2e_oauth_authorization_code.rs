mod common;

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD},
};
use common::TestServer;
use reqwest::StatusCode;
use sha2::Digest;

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
    assert_eq!(
        app_body["scopes"],
        serde_json::json!(["read:accounts", "write:statuses"])
    );
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
async fn test_oauth_authorize_redirects_back_with_oauth_error_for_invalid_scope() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let app = server
        .create_oauth_app("https://client.example/callback", "read:accounts")
        .await;
    let (_, session_cookie) = server.login_password().await;

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
            ("scope", "write:statuses"),
            ("state", "oauth-state"),
        ])
        .header("Cookie", session_cookie)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_redirection());
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .unwrap();
    let redirect = url::Url::parse(location).unwrap();
    let params = redirect
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        params.get("error").map(|value| value.as_ref()),
        Some("invalid_scope")
    );
    assert_eq!(
        params.get("state").map(|value| value.as_ref()),
        Some("oauth-state")
    );
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

#[tokio::test]
async fn test_oauth_authorization_code_flow_supports_pkce_s256() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let redirect_uri = "https://client.example/callback";
    let app = server.create_oauth_app(redirect_uri, "read:accounts").await;
    let client_id = app["client_id"].as_str().unwrap();
    let client_secret = app["client_secret"].as_str().unwrap();
    let (_, session_cookie) = server.login_password().await;

    let verifier = "pkce-verifier-1234567890";
    let challenge = URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let authorize = client
        .get(server.url("/oauth/authorize"))
        .query(&[
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("scope", "read:accounts"),
            ("state", "pkce-state"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ])
        .header("Cookie", session_cookie)
        .send()
        .await
        .unwrap();
    assert!(authorize.status().is_redirection());

    let redirect_location = authorize
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .unwrap();
    let redirect_url = url::Url::parse(redirect_location).unwrap();
    let code = redirect_url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .unwrap();

    let token = server
        .client
        .post(server.url("/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect_uri),
            ("code", code.as_str()),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(token.status(), StatusCode::OK);

    let body = token.json::<serde_json::Value>().await.unwrap();
    assert!(body["access_token"].as_str().is_some());
    assert!(body["refresh_token"].as_str().is_some());
}

#[tokio::test]
async fn test_oauth_authorization_accepts_granular_scopes_when_app_has_broad_scopes() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let redirect_uri = "https://client.example/callback";
    let app = server.create_oauth_app(redirect_uri, "read write").await;
    let client_id = app["client_id"].as_str().unwrap();
    let client_secret = app["client_secret"].as_str().unwrap();
    let (_, session_cookie) = server.login_password().await;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let authorize = client
        .get(server.url("/oauth/authorize"))
        .query(&[
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("scope", "read:accounts write:statuses"),
            ("state", "umbrella-state"),
        ])
        .header("Cookie", session_cookie)
        .send()
        .await
        .unwrap();
    assert!(authorize.status().is_redirection());

    let redirect_location = authorize
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .unwrap();
    let redirect_url = url::Url::parse(redirect_location).unwrap();
    let code = redirect_url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .unwrap();

    let token = server
        .client
        .post(server.url("/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect_uri),
            ("code", code.as_str()),
            ("scope", "read:accounts write:statuses"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(token.status(), StatusCode::OK);
    let body = token.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["scope"], "read:accounts write:statuses");
}

#[tokio::test]
async fn test_oauth_client_credentials_accepts_granular_scope_from_broad_app_scope() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let app = server
        .create_oauth_app("urn:ietf:wg:oauth:2.0:oob", "read write")
        .await;
    let client_id = app["client_id"].as_str().unwrap();
    let client_secret = app["client_secret"].as_str().unwrap();

    let token = server
        .client
        .post(server.url("/oauth/token"))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("scope", "read:accounts"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(token.status(), StatusCode::OK);
    let body = token.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["scope"], "read:accounts");
}

#[tokio::test]
async fn test_oauth_refresh_token_grant_rotates_tokens() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let redirect_uri = "https://client.example/callback";
    let app = server
        .create_oauth_app(redirect_uri, "read:accounts write:statuses")
        .await;
    let client_id = app["client_id"].as_str().unwrap();
    let client_secret = app["client_secret"].as_str().unwrap();
    let (_, session_cookie) = server.login_password().await;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let authorize = client
        .get(server.url("/oauth/authorize"))
        .query(&[
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("scope", "read:accounts write:statuses"),
        ])
        .header("Cookie", session_cookie)
        .send()
        .await
        .unwrap();
    assert!(authorize.status().is_redirection());
    let redirect_location = authorize
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .unwrap();
    let redirect_url = url::Url::parse(redirect_location).unwrap();
    let code = redirect_url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .unwrap();

    let token = server
        .client
        .post(server.url("/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect_uri),
            ("code", code.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(token.status(), StatusCode::OK);
    let body = token.json::<serde_json::Value>().await.unwrap();
    let access_token = body["access_token"].as_str().unwrap().to_string();
    let refresh_token = body["refresh_token"].as_str().unwrap().to_string();

    let refreshed = server
        .client
        .post(server.url("/oauth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(refreshed.status(), StatusCode::OK);
    let refreshed_body = refreshed.json::<serde_json::Value>().await.unwrap();
    let new_access_token = refreshed_body["access_token"].as_str().unwrap().to_string();
    let new_refresh_token = refreshed_body["refresh_token"]
        .as_str()
        .unwrap()
        .to_string();

    assert_ne!(new_access_token, access_token);
    assert_ne!(new_refresh_token, refresh_token);

    let reused = server
        .client
        .post(server.url("/oauth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(reused.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_oauth_token_supports_http_basic_client_auth() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let redirect_uri = "https://client.example/callback";
    let app = server.create_oauth_app(redirect_uri, "read:accounts").await;
    let client_id = app["client_id"].as_str().unwrap();
    let client_secret = app["client_secret"].as_str().unwrap();
    let (_, session_cookie) = server.login_password().await;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let authorize = client
        .get(server.url("/oauth/authorize"))
        .query(&[
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("scope", "read:accounts"),
        ])
        .header("Cookie", session_cookie)
        .send()
        .await
        .unwrap();
    assert!(authorize.status().is_redirection());
    let redirect_location = authorize
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .unwrap();
    let redirect_url = url::Url::parse(redirect_location).unwrap();
    let code = redirect_url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .unwrap();

    let basic = BASE64_STANDARD.encode(format!("{client_id}:{client_secret}"));
    let token = server
        .client
        .post(server.url("/oauth/token"))
        .header("Authorization", format!("Basic {}", basic))
        .form(&[
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri),
            ("code", code.as_str()),
        ])
        .send()
        .await
        .unwrap();

    assert_eq!(token.status(), StatusCode::OK);
    let body = token.json::<serde_json::Value>().await.unwrap();
    assert!(body["access_token"].as_str().is_some());
}

#[tokio::test]
async fn test_oauth_revoke_supports_http_basic_client_auth() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let redirect_uri = "https://client.example/callback";
    let app = server
        .create_oauth_app(redirect_uri, "read:accounts write:statuses")
        .await;
    let client_id = app["client_id"].as_str().unwrap();
    let client_secret = app["client_secret"].as_str().unwrap();
    let access_token = server
        .create_oauth_authorization_code_token_for_app(
            client_id,
            client_secret,
            redirect_uri,
            "read:accounts write:statuses",
        )
        .await;

    let basic = BASE64_STANDARD.encode(format!("{client_id}:{client_secret}"));
    let revoke = server
        .client
        .post(server.url("/oauth/revoke"))
        .header("Authorization", format!("Basic {}", basic))
        .form(&[("token", access_token.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::OK);

    let verify = server
        .client
        .get(server.url("/api/v1/accounts/verify_credentials"))
        .bearer_auth(access_token)
        .send()
        .await
        .unwrap();
    assert_eq!(verify.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_oauth_revoke_returns_oauth_error_for_invalid_client_credentials() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .post(server.url("/oauth/revoke"))
        .form(&[
            ("client_id", "invalid-client"),
            ("client_secret", "wrong-secret"),
            ("token", "bogus-token"),
        ])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["error"], "invalid_client");
}

#[tokio::test]
async fn test_oauth_revoke_rejects_foreign_token_with_unauthorized_client() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let first_app = server
        .create_oauth_app("https://client.example/callback", "read:accounts")
        .await;
    let second_app = server
        .create_oauth_app("https://other.example/callback", "read:accounts")
        .await;

    let token = server
        .create_oauth_authorization_code_token_for_app(
            first_app["client_id"].as_str().unwrap(),
            first_app["client_secret"].as_str().unwrap(),
            "https://client.example/callback",
            "read:accounts",
        )
        .await;

    let revoke = server
        .client
        .post(server.url("/oauth/revoke"))
        .form(&[
            ("client_id", second_app["client_id"].as_str().unwrap()),
            (
                "client_secret",
                second_app["client_secret"].as_str().unwrap(),
            ),
            ("token", token.as_str()),
        ])
        .send()
        .await
        .unwrap();

    assert_eq!(revoke.status(), StatusCode::FORBIDDEN);
    let body = revoke.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["error"], "unauthorized_client");
}

#[tokio::test]
async fn test_oauth_metadata_endpoint_exposes_mastodon_compatible_server_metadata() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(server.url("/.well-known/oauth-authorization-server"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(
        body["authorization_endpoint"],
        "https://test.example.com/oauth/authorize"
    );
    assert_eq!(
        body["app_registration_endpoint"],
        "https://test.example.com/api/v1/apps"
    );
    assert_eq!(
        body["token_endpoint_auth_methods_supported"],
        serde_json::json!(["client_secret_basic", "client_secret_post"])
    );
    assert_eq!(
        body["code_challenge_methods_supported"],
        serde_json::json!(["S256"])
    );
}

#[tokio::test]
async fn test_invalid_bearer_token_returns_mastodon_style_error_payload() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let response = server
        .client
        .get(server.url("/api/v1/accounts/verify_credentials"))
        .bearer_auth("bogus-token")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["error"], "The access token is invalid");
}
