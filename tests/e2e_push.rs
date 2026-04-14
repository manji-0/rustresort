//! E2E tests for Mastodon push subscription endpoints.

mod common;

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode as AxumStatusCode},
    routing::post,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use common::TestServer;
use openssl::{
    bn::BigNumContext,
    ec::{EcGroup, EcKey, PointConversionForm},
    nid::Nid,
    rand::rand_bytes,
};
use reqwest::StatusCode;
use serde_json::json;
use std::collections::HashMap;
use tokio::{
    net::TcpListener,
    sync::mpsc,
    time::{Duration, timeout},
};

#[derive(Debug, Clone)]
struct CapturedPushRequest {
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

#[derive(Clone)]
struct PushCaptureState {
    tx: mpsc::UnboundedSender<CapturedPushRequest>,
}

struct PushCaptureServer {
    addr: String,
    rx: mpsc::UnboundedReceiver<CapturedPushRequest>,
}

impl PushCaptureServer {
    async fn new() -> Self {
        async fn capture_push(
            State(state): State<PushCaptureState>,
            headers: HeaderMap,
            body: Bytes,
        ) -> AxumStatusCode {
            let captured = CapturedPushRequest {
                headers: headers
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .to_str()
                            .ok()
                            .map(|value| (name.as_str().to_string(), value.to_string()))
                    })
                    .collect(),
                body: body.to_vec(),
            };
            let _ = state.tx.send(captured);
            AxumStatusCode::CREATED
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let state = PushCaptureState { tx };
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind push capture listener");
        let addr = listener.local_addr().expect("push capture local addr");
        let app = Router::new()
            .route("/push", post(capture_push))
            .with_state(state);
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve push capture");
        });

        Self {
            addr: format!("http://{}", addr),
            rx,
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/push", self.addr)
    }

    async fn recv(&mut self) -> CapturedPushRequest {
        timeout(Duration::from_secs(5), self.rx.recv())
            .await
            .expect("timed out waiting for push request")
            .expect("push capture channel unexpectedly closed")
    }

    async fn expect_no_request(&mut self, duration: Duration) {
        assert!(
            timeout(duration, self.rx.recv()).await.is_err(),
            "unexpected push request was received"
        );
    }
}

fn generate_web_push_subscription_keys() -> (String, String) {
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).expect("load p256 group");
    let key = EcKey::generate(&group).expect("generate subscription keypair");
    let mut context = BigNumContext::new().expect("allocate bn context");
    let public_key = key
        .public_key()
        .to_bytes(&group, PointConversionForm::UNCOMPRESSED, &mut context)
        .expect("encode subscription public key");

    let mut auth_secret = [0u8; 16];
    rand_bytes(&mut auth_secret).expect("generate auth secret");

    (
        URL_SAFE_NO_PAD.encode(public_key),
        URL_SAFE_NO_PAD.encode(auth_secret),
    )
}

fn register_default_remote_key(server: &TestServer) -> String {
    let actor_id = "https://remote.example/users/alice";
    let key_id = format!("{actor_id}#main-key");
    server.register_inbound_public_key(&key_id, common::test_public_key_pem());
    key_id
}

#[tokio::test]
async fn test_push_subscription_crud() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let (p256dh, auth) = generate_web_push_subscription_keys();

    let create_response = server
        .client
        .post(server.url("/api/v1/push/subscription"))
        .bearer_auth(&token)
        .json(&json!({
            "subscription": {
                "endpoint": "https://push.example.test/subscription/1",
                "keys": {
                    "p256dh": p256dh,
                    "auth": auth
                },
                "standard": true
            },
            "data": {
                "policy": "all",
                "alerts": {
                    "follow": true,
                    "mention": true
                }
            }
        }))
        .send()
        .await
        .expect("create push subscription");

    assert_eq!(create_response.status(), StatusCode::OK);
    let created: serde_json::Value = create_response.json().await.expect("create json");
    assert_eq!(
        created["endpoint"],
        "https://push.example.test/subscription/1"
    );
    assert_eq!(created["policy"], "all");
    assert_eq!(created["standard"], true);
    assert_eq!(created["alerts"]["follow"], true);
    assert!(created["server_key"].as_str().is_some());

    let get_response = server
        .client
        .get(server.url("/api/v1/push/subscription"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("get push subscription");
    assert_eq!(get_response.status(), StatusCode::OK);
    let fetched: serde_json::Value = get_response.json().await.expect("get subscription json");
    assert_eq!(fetched["standard"], true);

    let update_response = server
        .client
        .put(server.url("/api/v1/push/subscription"))
        .bearer_auth(&token)
        .json(&json!({
            "data": {
                "policy": "followed",
                "alerts": {
                    "follow": false,
                    "mention": true,
                    "quote": true
                }
            }
        }))
        .send()
        .await
        .expect("update push subscription");
    assert_eq!(update_response.status(), StatusCode::OK);
    let updated: serde_json::Value = update_response.json().await.expect("update json");
    assert_eq!(updated["policy"], "followed");
    assert_eq!(updated["alerts"]["follow"], false);
    assert_eq!(updated["alerts"]["quote"], true);

    let delete_response = server
        .client
        .delete(server.url("/api/v1/push/subscription"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("delete push subscription");
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    let missing_response = server
        .client
        .get(server.url("/api/v1/push/subscription"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("get deleted push subscription");
    assert_eq!(missing_response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_push_delivery_for_remote_follow_notification() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let mut capture = PushCaptureServer::new().await;
    let (p256dh, auth) = generate_web_push_subscription_keys();

    let create_response = server
        .client
        .post(server.url("/api/v1/push/subscription"))
        .bearer_auth(&token)
        .json(&json!({
            "subscription": {
                "endpoint": capture.endpoint(),
                "keys": {
                    "p256dh": p256dh,
                    "auth": auth
                }
            },
            "data": {
                "policy": "all",
                "alerts": {
                    "follow": true
                }
            }
        }))
        .send()
        .await
        .expect("create push subscription");
    assert_eq!(create_response.status(), StatusCode::OK);

    let key_id = register_default_remote_key(&server);
    let follow = json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/push-follow-1",
        "type": "Follow",
        "actor": {
            "id": "https://remote.example/users/alice",
            "inbox": "https://remote.example/users/alice/inbox"
        },
        "object": server.public_url("/users/testuser")
    });

    let follow_response = server
        .post_signed_activity("/users/testuser/inbox", &follow, &key_id)
        .await;
    assert_eq!(follow_response.status(), StatusCode::OK);

    let request = capture.recv().await;
    assert_eq!(
        request.headers.get("content-encoding").map(String::as_str),
        Some("aes128gcm")
    );
    assert!(request.headers.contains_key("authorization"));
    assert!(request.headers.contains_key("ttl"));
    assert!(
        !request.body.is_empty(),
        "push body should be encrypted bytes"
    );
}

#[tokio::test]
async fn test_push_delivery_skips_disabled_alerts() {
    let server = TestServer::new().await;
    server.create_test_account().await;
    let token = server.create_test_token().await;
    let mut capture = PushCaptureServer::new().await;
    let (p256dh, auth) = generate_web_push_subscription_keys();

    let create_response = server
        .client
        .post(server.url("/api/v1/push/subscription"))
        .bearer_auth(&token)
        .json(&json!({
            "subscription": {
                "endpoint": capture.endpoint(),
                "keys": {
                    "p256dh": p256dh,
                    "auth": auth
                }
            },
            "data": {
                "policy": "all",
                "alerts": {
                    "mention": true
                }
            }
        }))
        .send()
        .await
        .expect("create push subscription");
    assert_eq!(create_response.status(), StatusCode::OK);

    let key_id = register_default_remote_key(&server);
    let follow = json!({
        "@context": "https://www.w3.org/ns/activitystreams",
        "id": "https://remote.example/activities/push-follow-disabled",
        "type": "Follow",
        "actor": {
            "id": "https://remote.example/users/alice",
            "inbox": "https://remote.example/users/alice/inbox"
        },
        "object": server.public_url("/users/testuser")
    });

    let follow_response = server
        .post_signed_activity("/users/testuser/inbox", &follow, &key_id)
        .await;
    assert_eq!(follow_response.status(), StatusCode::OK);

    capture.expect_no_request(Duration::from_millis(750)).await;
}
