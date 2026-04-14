use reqwest::Client;
use rustresort::federation::sign_request;
use serde_json::json;
use std::env;
use url::Url;

const TEST_PRIVATE_KEY_PEM: &str = include_str!("../../tests/fixtures/test_private_key.pem");
const REMOTE_ACTOR_ID: &str = "https://remote.example/users/alice";
const REMOTE_INBOX: &str = "https://remote.example/inbox";

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].clone())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args().collect::<Vec<_>>();
    let base_url =
        arg_value(&args, "--base-url").unwrap_or_else(|| "http://127.0.0.1:3011".to_string());
    let fixture = arg_value(&args, "--fixture").unwrap_or_else(|| "mention".to_string());
    let local_username =
        arg_value(&args, "--local-username").unwrap_or_else(|| "admin".to_string());
    let object_uri = arg_value(&args, "--object-uri");

    let activity = build_activity(&fixture, &base_url, &local_username, object_uri.as_deref())?;
    post_signed_activity(&base_url, "/inbox", &activity).await?;
    println!("ui-playwright-activity: sent {fixture}");
    Ok(())
}

fn build_activity(
    fixture: &str,
    base_url: &str,
    local_username: &str,
    object_uri: Option<&str>,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let suffix = chrono::Utc::now().timestamp_millis();
    let local_origin = local_origin(base_url)?;
    let local_actor = format!("{}/users/{}/", local_origin, local_username);
    let local_actor_tag = format!("{}/users/{}", local_origin, local_username);
    let local_acct = format!("{local_username}@{}", local_origin_host(base_url)?);

    match fixture {
        "mention" => Ok(json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "id": format!("https://remote.example/activities/create-mention-{suffix}"),
            "type": "Create",
            "actor": {
                "id": REMOTE_ACTOR_ID,
                "inbox": REMOTE_INBOX
            },
            "object": {
                "type": "Note",
                "attributedTo": REMOTE_ACTOR_ID,
                "id": format!("https://remote.example/users/alice/statuses/mention-{suffix}"),
                "content": format!("<p>Hello @{local_acct} from Playwright</p>"),
                "published": "2026-01-01T00:00:00Z",
                "to": local_actor,
                "tag": [{
                    "type": "Mention",
                    "href": local_actor_tag
                }]
            }
        })),
        "like" => Ok(json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "id": format!("https://remote.example/likes/{suffix}"),
            "type": "Like",
            "actor": REMOTE_ACTOR_ID,
            "object": object_uri.ok_or("like fixture requires --object-uri")?
        })),
        "announce" => Ok(json!({
            "@context": "https://www.w3.org/ns/activitystreams",
            "id": format!("https://remote.example/announces/{suffix}"),
            "type": "Announce",
            "actor": REMOTE_ACTOR_ID,
            "object": object_uri.ok_or("announce fixture requires --object-uri")?
        })),
        _ => Err(format!("unsupported fixture: {fixture}").into()),
    }
}

fn local_origin(base_url: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut url = Url::parse(base_url)?;
    if matches!(url.host_str(), Some("127.0.0.1") | Some("::1")) {
        url.set_host(Some("localhost"))?;
    }
    Ok(format!(
        "{}://{}",
        url.scheme(),
        url.authority().trim_end_matches('/')
    ))
}

fn local_origin_host(base_url: &str) -> Result<String, Box<dyn std::error::Error>> {
    let origin = local_origin(base_url)?;
    let url = Url::parse(&origin)?;
    Ok(url.authority().to_string())
}

async fn post_signed_activity(
    base_url: &str,
    path: &str,
    activity: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let body = serde_json::to_vec(activity)?;
    let key_id = format!("{REMOTE_ACTOR_ID}#main-key");
    let signed = sign_request("POST", &url, Some(&body), TEST_PRIVATE_KEY_PEM, &key_id)?;
    let parsed_url = url::Url::parse(&url)?;

    let mut request = Client::new()
        .post(&url)
        .header("Content-Type", "application/activity+json")
        .header("Host", parsed_url.host_str().ok_or("missing host")?)
        .header("Date", signed.date)
        .header("Signature", signed.signature)
        .body(body);

    if let Some(digest) = signed.digest {
        request = request.header("Digest", digest);
    }

    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("activity post failed: {status} {body}").into());
    }
    Ok(())
}
