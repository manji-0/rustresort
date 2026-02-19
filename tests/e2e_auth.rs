//! E2E tests for GitHub OAuth and session endpoints

mod common;

use common::TestServer;

fn no_redirect_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("failed to build no-redirect client")
}

#[tokio::test]
async fn test_login_page_renders() {
    let server = TestServer::new().await;

    let response = server
        .client
        .get(server.url("/login"))
        .send()
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), 200);
    let body = response.text().await.expect("response body");
    assert!(body.contains("Sign in with GitHub"));
}

#[tokio::test]
async fn test_github_redirect_sets_csrf_cookie_and_redirects() {
    let server = TestServer::new().await;
    let client = no_redirect_client();

    let response = client
        .get(server.url("/auth/github"))
        .send()
        .await
        .expect("request succeeds");

    assert!(response.status().is_redirection());
    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("location header");
    assert!(location.starts_with("https://github.com/login/oauth/authorize?"));
    assert!(location.contains("client_id=test-client-id"));
    assert!(location.contains("scope=read%3Auser"));
    assert!(location.contains("state="));

    let set_cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .expect("set-cookie header");
    assert!(set_cookie.contains("oauth_state="));
}

#[tokio::test]
async fn test_github_callback_rejects_missing_csrf_cookie() {
    let server = TestServer::new().await;
    let client = no_redirect_client();

    let response = client
        .get(server.url("/auth/github/callback?code=dummy&state=dummy"))
        .send()
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_logout_clears_session_cookies() {
    let server = TestServer::new().await;
    let client = no_redirect_client();

    let response = client
        .post(server.url("/logout"))
        .header("Cookie", "session=dummy-session; oauth_state=dummy-state")
        .send()
        .await
        .expect("request succeeds");

    assert!(response.status().is_redirection());
    let set_cookie_values: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok().map(ToString::to_string))
        .collect();
    assert!(
        set_cookie_values
            .iter()
            .any(|v| v.contains("session=") || v.contains("oauth_state=")),
        "expected cookie removal headers, got: {set_cookie_values:?}"
    );
}
