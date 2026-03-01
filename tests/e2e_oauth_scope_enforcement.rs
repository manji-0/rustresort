mod common;

use common::TestServer;
use reqwest::{StatusCode, header::LOCATION, header::SET_COOKIE};
use serde_json::json;
use url::Url;

async fn create_oauth_app(server: &TestServer) -> serde_json::Value {
    let app_request = json!({
        "client_name": "Scope Test App",
        "redirect_uris": "https://client.example/callback",
        "scopes": "read read:accounts read:statuses read:notifications read:lists read:filters read:search write write:accounts write:statuses write:favourites write:notifications write:media write:lists write:filters follow push"
    });

    let app_response = server
        .client
        .post(server.url("/api/v1/apps"))
        .json(&app_request)
        .send()
        .await
        .unwrap();
    assert_eq!(app_response.status(), StatusCode::OK);
    app_response.json().await.unwrap()
}

fn extract_confirm_token(authorize_response: &reqwest::Response) -> String {
    authorize_response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .find_map(|value| {
            let raw = value.to_str().ok()?;
            let cookie_pair = raw.split(';').next()?;
            let (name, _) = cookie_pair.split_once('=')?;
            name.strip_prefix("oauth_authorize_confirm_")
                .map(ToString::to_string)
        })
        .expect("missing oauth authorize confirm cookie")
}

async fn create_scoped_oauth_token(server: &TestServer, scope: &str) -> String {
    let app = create_oauth_app(server).await;

    let session_token = server.create_test_token().await;
    let no_redirect_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let consent_response = no_redirect_client
        .get(server.url("/oauth/authorize"))
        .bearer_auth(&session_token)
        .query(&[
            ("response_type", "code"),
            ("client_id", app["client_id"].as_str().unwrap()),
            ("redirect_uri", "https://client.example/callback"),
            ("scope", scope),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(consent_response.status(), StatusCode::OK);
    let confirm_token = extract_confirm_token(&consent_response);

    let authorize_response = no_redirect_client
        .get(server.url("/oauth/authorize"))
        .bearer_auth(&session_token)
        .header(
            "Cookie",
            format!("oauth_authorize_confirm_{}=1", confirm_token),
        )
        .query(&[
            ("response_type", "code"),
            ("client_id", app["client_id"].as_str().unwrap()),
            ("redirect_uri", "https://client.example/callback"),
            ("scope", scope),
            ("approve", "true"),
            ("confirm", confirm_token.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(authorize_response.status(), StatusCode::SEE_OTHER);

    let location = authorize_response
        .headers()
        .get(LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let redirect = Url::parse(location).unwrap();
    let code = redirect
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.to_string())
        .unwrap();

    let token_request = json!({
        "grant_type": "authorization_code",
        "code": code,
        "client_id": app["client_id"],
        "client_secret": app["client_secret"],
        "redirect_uri": "https://client.example/callback",
        "scope": scope
    });

    let token_response = server
        .client
        .post(server.url("/oauth/token"))
        .json(&token_request)
        .send()
        .await
        .unwrap();
    assert_eq!(token_response.status(), StatusCode::OK);

    let token_json: serde_json::Value = token_response.json().await.unwrap();
    token_json["access_token"].as_str().unwrap().to_string()
}

async fn create_client_credentials_oauth_token(server: &TestServer, scope: &str) -> String {
    let app = create_oauth_app(server).await;

    let token_request = json!({
        "grant_type": "client_credentials",
        "client_id": app["client_id"],
        "client_secret": app["client_secret"],
        "scope": scope
    });

    let token_response = server
        .client
        .post(server.url("/oauth/token"))
        .json(&token_request)
        .send()
        .await
        .unwrap();
    assert_eq!(token_response.status(), StatusCode::OK);

    let token_json: serde_json::Value = token_response.json().await.unwrap();
    token_json["access_token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_client_credentials_tokens_cannot_access_user_endpoints() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let app_token = create_client_credentials_oauth_token(&server, "write:statuses").await;

    let create_status_response = server
        .client
        .post(server.url("/api/v1/statuses"))
        .bearer_auth(&app_token)
        .json(&json!({ "status": "must be rejected for app token" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_status_response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_scope_read_accounts_cannot_write_statuses() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let token = create_scoped_oauth_token(&server, "read:accounts").await;

    let verify_response = server
        .client
        .get(server.url("/api/v1/accounts/verify_credentials"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(verify_response.status(), StatusCode::OK);

    let create_status_response = server
        .client
        .post(server.url("/api/v1/statuses"))
        .bearer_auth(&token)
        .json(&json!({ "status": "blocked by scope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_status_response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_scope_write_statuses_allows_post_status() {
    let server = TestServer::new().await;
    server.create_test_account().await;

    let token = create_scoped_oauth_token(&server, "write:statuses").await;

    let create_status_response = server
        .client
        .post(server.url("/api/v1/statuses"))
        .bearer_auth(&token)
        .json(&json!({ "status": "allowed by write:statuses" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_status_response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_scope_follow_required_for_follow_endpoints() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let target = "alice@remote.example";

    let no_follow_token = create_scoped_oauth_token(&server, "read:accounts").await;
    let follow_token = create_scoped_oauth_token(&server, "follow").await;

    let blocked_follow_response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/follow", target)))
        .bearer_auth(&no_follow_token)
        .send()
        .await
        .unwrap();
    assert_eq!(blocked_follow_response.status(), StatusCode::FORBIDDEN);

    let allowed_follow_response = server
        .client
        .post(server.url(&format!("/api/v1/accounts/{}/follow", target)))
        .bearer_auth(&follow_token)
        .send()
        .await
        .unwrap();
    assert_eq!(allowed_follow_response.status(), StatusCode::OK);
}
