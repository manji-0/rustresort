mod common;

use common::TestServer;
use reqwest::{StatusCode, header::LOCATION};
use serde_json::json;
use url::Url;

async fn create_app(server: &TestServer) -> serde_json::Value {
    let app_request = json!({
        "client_name": "OAuth Test Client",
        "redirect_uris": "https://client.example/callback",
        "scopes": "read write follow"
    });

    let response = server
        .client
        .post(server.url("/api/v1/apps"))
        .json(&app_request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.json().await.unwrap()
}

#[tokio::test]
async fn test_authorization_code_flow_works_and_prevents_replay() {
    let server = TestServer::new().await;
    let app = create_app(&server).await;

    let no_redirect_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let authorize_response = no_redirect_client
        .get(server.url("/oauth/authorize"))
        .query(&[
            ("response_type", "code"),
            ("client_id", app["client_id"].as_str().unwrap()),
            ("redirect_uri", "https://client.example/callback"),
            ("scope", "read write"),
            ("state", "abc123"),
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
    let state = redirect
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string())
        .unwrap();
    assert_eq!(state, "abc123");

    let token_request = json!({
        "grant_type": "authorization_code",
        "code": code,
        "client_id": app["client_id"],
        "client_secret": app["client_secret"],
        "redirect_uri": "https://client.example/callback"
    });

    let token_response = server
        .client
        .post(server.url("/oauth/token"))
        .json(&token_request)
        .send()
        .await
        .unwrap();
    assert_eq!(token_response.status(), StatusCode::OK);

    let replay_response = server
        .client
        .post(server.url("/oauth/token"))
        .json(&token_request)
        .send()
        .await
        .unwrap();
    assert_eq!(replay_response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_authorization_code_token_rejects_redirect_uri_mismatch() {
    let server = TestServer::new().await;
    let app = create_app(&server).await;

    let no_redirect_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let authorize_response = no_redirect_client
        .get(server.url("/oauth/authorize"))
        .query(&[
            ("response_type", "code"),
            ("client_id", app["client_id"].as_str().unwrap()),
            ("redirect_uri", "https://client.example/callback"),
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

    let bad_redirect_token_request = json!({
        "grant_type": "authorization_code",
        "code": code,
        "client_id": app["client_id"],
        "client_secret": app["client_secret"],
        "redirect_uri": "https://evil.example/callback"
    });

    let token_response = server
        .client
        .post(server.url("/oauth/token"))
        .json(&bad_redirect_token_request)
        .send()
        .await
        .unwrap();

    assert!(
        token_response.status() == StatusCode::UNAUTHORIZED
            || token_response.status() == StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn test_authorize_rejects_unregistered_redirect_uri() {
    let server = TestServer::new().await;
    let app = create_app(&server).await;

    let no_redirect_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let authorize_response = no_redirect_client
        .get(server.url("/oauth/authorize"))
        .query(&[
            ("response_type", "code"),
            ("client_id", app["client_id"].as_str().unwrap()),
            ("redirect_uri", "https://evil.example/callback"),
        ])
        .send()
        .await
        .unwrap();

    assert_eq!(
        authorize_response.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
