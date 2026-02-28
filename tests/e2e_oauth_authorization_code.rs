mod common;

use common::TestServer;
use reqwest::{StatusCode, header::LOCATION, header::SET_COOKIE};
use serde_json::json;
use url::Url;

async fn create_app(server: &TestServer) -> serde_json::Value {
    create_app_with_redirect(server, "https://client.example/callback").await
}

async fn create_app_with_redirect(server: &TestServer, redirect_uri: &str) -> serde_json::Value {
    let app_request = json!({
        "client_name": "OAuth Test Client",
        "redirect_uris": redirect_uri,
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

async fn authorize_with_consent(
    no_redirect_client: &reqwest::Client,
    server: &TestServer,
    session_token: &str,
    params: &[(&str, &str)],
) -> reqwest::Response {
    let consent_response = no_redirect_client
        .get(server.url("/oauth/authorize"))
        .bearer_auth(session_token)
        .query(params)
        .send()
        .await
        .unwrap();
    assert_eq!(consent_response.status(), StatusCode::OK);
    let confirm_token = extract_confirm_token(&consent_response);

    no_redirect_client
        .get(server.url("/oauth/authorize"))
        .bearer_auth(session_token)
        .header(
            "Cookie",
            format!("oauth_authorize_confirm_{}=1", confirm_token),
        )
        .query(params)
        .query(&[("approve", "true"), ("confirm", confirm_token.as_str())])
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn test_authorization_code_flow_works_and_prevents_replay() {
    let server = TestServer::new().await;
    let app = create_app(&server).await;
    let session_token = server.create_test_token().await;

    let no_redirect_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let authorize_response = authorize_with_consent(
        &no_redirect_client,
        &server,
        &session_token,
        &[
            ("response_type", "code"),
            ("client_id", app["client_id"].as_str().unwrap()),
            ("redirect_uri", "https://client.example/callback"),
            ("scope", "read write"),
            ("state", "abc123"),
        ],
    )
    .await;
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
    let session_token = server.create_test_token().await;

    let no_redirect_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let authorize_response = authorize_with_consent(
        &no_redirect_client,
        &server,
        &session_token,
        &[
            ("response_type", "code"),
            ("client_id", app["client_id"].as_str().unwrap()),
            ("redirect_uri", "https://client.example/callback"),
        ],
    )
    .await;
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
    let session_token = server.create_test_token().await;

    let no_redirect_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let authorize_response = no_redirect_client
        .get(server.url("/oauth/authorize"))
        .bearer_auth(&session_token)
        .query(&[
            ("response_type", "code"),
            ("client_id", app["client_id"].as_str().unwrap()),
            ("redirect_uri", "https://evil.example/callback"),
        ])
        .send()
        .await
        .unwrap();

    assert_eq!(authorize_response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_authorize_redirect_keeps_fragment_and_sets_query_code() {
    let server = TestServer::new().await;
    let app = create_app_with_redirect(&server, "https://client.example/callback#frag").await;
    let session_token = server.create_test_token().await;

    let no_redirect_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let authorize_response = authorize_with_consent(
        &no_redirect_client,
        &server,
        &session_token,
        &[
            ("response_type", "code"),
            ("client_id", app["client_id"].as_str().unwrap()),
            ("redirect_uri", "https://client.example/callback#frag"),
            ("state", "st"),
        ],
    )
    .await;
    assert_eq!(authorize_response.status(), StatusCode::SEE_OTHER);

    let location = authorize_response
        .headers()
        .get(LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let redirect = Url::parse(location).unwrap();
    assert_eq!(redirect.fragment(), Some("frag"));
    assert!(redirect.query_pairs().any(|(k, _)| k == "code"));
    assert!(
        redirect
            .query_pairs()
            .any(|(k, v)| k == "state" && v == "st")
    );
}
