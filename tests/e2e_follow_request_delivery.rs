mod common;

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode as AxumStatusCode},
    routing::post,
};
use common::TestServer;
use reqwest::StatusCode;
use serde_json::json;
use std::collections::HashMap;
use tokio::{
    net::TcpListener,
    sync::mpsc,
    time::{Duration, timeout},
};

#[derive(Debug, Clone)]
struct CapturedActivityRequest {
    headers: HashMap<String, String>,
    body: serde_json::Value,
}

#[derive(Clone)]
struct CaptureState {
    tx: mpsc::UnboundedSender<CapturedActivityRequest>,
}

struct ActivityCaptureServer {
    addr: String,
    rx: mpsc::UnboundedReceiver<CapturedActivityRequest>,
}

impl ActivityCaptureServer {
    async fn new() -> Self {
        async fn capture(
            State(state): State<CaptureState>,
            headers: HeaderMap,
            body: Bytes,
        ) -> AxumStatusCode {
            let captured = CapturedActivityRequest {
                headers: headers
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .to_str()
                            .ok()
                            .map(|value| (name.as_str().to_string(), value.to_string()))
                    })
                    .collect(),
                body: serde_json::from_slice(&body).expect("ActivityPub JSON body"),
            };
            let _ = state.tx.send(captured);
            AxumStatusCode::ACCEPTED
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/inbox", post(capture))
            .with_state(CaptureState { tx });

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self {
            addr: format!("http://{}", addr),
            rx,
        }
    }

    fn inbox_url(&self) -> String {
        format!("{}/inbox", self.addr)
    }

    async fn recv(&mut self) -> CapturedActivityRequest {
        timeout(Duration::from_secs(5), self.rx.recv())
            .await
            .expect("timed out waiting for outbound activity")
            .expect("capture channel unexpectedly closed")
    }
}

#[tokio::test]
async fn test_authorize_follow_request_moves_to_followers_and_delivers_accept() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let mut capture = ActivityCaptureServer::new().await;
    let requester_address = "alice@remote.example";
    let follow_uri = "https://remote.example/activities/follow-authorize-1";

    server
        .state
        .db
        .insert_follow_request_with_actor_uri(
            requester_address,
            &capture.inbox_url(),
            follow_uri,
            Some("https://remote.example/users/alice"),
        )
        .await
        .unwrap();

    let response = server
        .client
        .post(server.url(&format!(
            "/api/v1/follow_requests/{requester_address}/authorize"
        )))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let relationship = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(relationship["followed_by"], true);

    let delivered = capture.recv().await;
    assert_eq!(delivered.body["type"], "Accept");
    assert_eq!(delivered.body["object"]["id"], follow_uri);
    assert!(delivered.headers.contains_key("signature"));

    assert!(
        server
            .state
            .db
            .get_follow_request(requester_address)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        server
            .state
            .db
            .get_follower(requester_address, Some(443))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn test_reject_follow_request_removes_request_and_delivers_reject() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let mut capture = ActivityCaptureServer::new().await;
    let requester_address = "bob@remote.example";
    let follow_uri = "https://remote.example/activities/follow-reject-1";

    server
        .state
        .db
        .insert_follow_request_with_actor_uri(
            requester_address,
            &capture.inbox_url(),
            follow_uri,
            Some("https://remote.example/users/bob"),
        )
        .await
        .unwrap();

    let response = server
        .client
        .post(server.url(&format!(
            "/api/v1/follow_requests/{requester_address}/reject"
        )))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap(),
        json!({
            "id": requester_address,
            "following": false,
            "followed_by": false,
            "blocking": false,
            "blocked_by": false,
            "muting": false,
            "muting_notifications": false,
            "requested": false,
            "domain_blocking": false,
            "showing_reblogs": true,
            "endorsed": false,
            "notifying": false,
            "note": ""
        })
    );

    let delivered = capture.recv().await;
    assert_eq!(delivered.body["type"], "Reject");
    assert_eq!(delivered.body["object"]["id"], follow_uri);
    assert!(delivered.headers.contains_key("signature"));

    assert!(
        server
            .state
            .db
            .get_follow_request(requester_address)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        server
            .state
            .db
            .get_follower(requester_address, Some(443))
            .await
            .unwrap()
            .is_none()
    );
}
