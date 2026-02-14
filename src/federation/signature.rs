#![allow(dead_code)]
//! HTTP Signatures for ActivityPub
//!
//! Implements signing and verification per:
//! https://docs.joinmastodon.org/spec/security/

use crate::error::AppError;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use rsa::pkcs8::DecodePublicKey;
use rsa::signature::Verifier;
use rsa::{RsaPublicKey, pkcs1v15::Signature as Pkcs1v15Signature};
use sha2::{Digest, Sha256};
use std::net::IpAddr;

fn is_disallowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_multicast()
                || v6.is_unspecified()
        }
    }
}

fn is_disallowed_host(host: &str) -> bool {
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    if normalized == "localhost" || normalized.ends_with(".localhost") {
        return true;
    }

    normalized
        .parse::<IpAddr>()
        .map(is_disallowed_ip)
        .unwrap_or(false)
}

async fn validate_resolved_host_ips(host: &str, port: u16) -> Result<(), AppError> {
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();

    let mut resolved_any = false;
    let lookup = tokio::net::lookup_host((normalized.as_str(), port))
        .await
        .map_err(|e| AppError::Federation(format!("Failed to resolve actor host: {}", e)))?;

    for addr in lookup {
        resolved_any = true;
        if is_disallowed_ip(addr.ip()) {
            return Err(AppError::Forbidden);
        }
    }

    if !resolved_any {
        return Err(AppError::Federation(
            "No DNS records for actor host".to_string(),
        ));
    }

    Ok(())
}

/// Extract and validate remote actor domain from an actor URL or key ID URL.
///
/// This rejects non-HTTP(S) URLs and obvious local/private hosts.
pub fn extract_actor_domain(actor_or_key_id: &str) -> Result<String, AppError> {
    let actor_url = actor_or_key_id.split('#').next().unwrap_or(actor_or_key_id);
    let parsed = url::Url::parse(actor_url)
        .map_err(|e| AppError::Validation(format!("Invalid actor URL: {}", e)))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(AppError::Validation(format!(
                "Unsupported actor URL scheme: {}",
                scheme
            )));
        }
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::Validation("Missing host in actor URL".to_string()))?
        .to_ascii_lowercase();

    if is_disallowed_host(&host) {
        return Err(AppError::Forbidden);
    }

    Ok(host)
}

/// Sign an HTTP request
///
/// Creates HTTP Signature header for outgoing requests.
///
/// # Arguments
/// * `method` - HTTP method (e.g., "POST")
/// * `url` - Full URL being requested
/// * `body` - Request body (for digest)
/// * `private_key_pem` - RSA private key in PEM format
/// * `key_id` - Full URL to the public key (actor#main-key)
///
/// # Returns
/// Map of headers to add: Signature, Date, Digest (if body present)
///
/// # Example
/// ```ignore
/// let headers = sign_request(
///     "POST",
///     "https://remote.server/inbox",
///     Some(&body),
///     &private_key,
///     "https://my.server/users/me#main-key",
/// )?;
/// ```
pub fn sign_request(
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    private_key_pem: &str,
    key_id: &str,
) -> Result<SignatureHeaders, AppError> {
    // 1. Parse URL to get host and path
    let parsed_url =
        url::Url::parse(url).map_err(|e| AppError::Validation(format!("Invalid URL: {}", e)))?;

    let host = parsed_url
        .host_str()
        .ok_or_else(|| AppError::Validation("Missing host in URL".to_string()))?;

    let path = parsed_url.path();
    let query = parsed_url.query();
    let path_and_query = if let Some(q) = query {
        format!("{}?{}", path, q)
    } else {
        path.to_string()
    };

    // 2. Generate Date header (RFC 2822 format)
    let date = chrono::Utc::now()
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();

    // 3. Generate Digest if body present
    let digest = body.map(generate_digest);

    // 4. Build signing string
    let request_target = format!("{} {}", method.to_lowercase(), path_and_query);

    let mut signing_parts = vec![
        format!("(request-target): {}", request_target),
        format!("host: {}", host),
        format!("date: {}", date),
    ];

    let mut headers_list = vec!["(request-target)", "host", "date"];

    if let Some(ref digest_value) = digest {
        signing_parts.push(format!("digest: {}", digest_value));
        headers_list.push("digest");
    }

    let signing_string = signing_parts.join("\n");

    // 5. Sign with RSA-SHA256
    use rsa::pkcs8::DecodePrivateKey;
    use rsa::signature::{RandomizedSigner, SignatureEncoding};

    let private_key = rsa::RsaPrivateKey::from_pkcs8_pem(private_key_pem)
        .map_err(|e| AppError::Validation(format!("Invalid private key: {}", e)))?;

    let signing_key = rsa::pkcs1v15::SigningKey::<Sha256>::new_unprefixed(private_key);
    let mut rng = rand::thread_rng();
    let signature = signing_key.sign_with_rng(&mut rng, signing_string.as_bytes());
    let signature_b64 = BASE64.encode(signature.to_bytes());

    // 6. Build Signature header
    let signature_header = format!(
        "keyId=\"{}\",algorithm=\"rsa-sha256\",headers=\"{}\",signature=\"{}\"",
        key_id,
        headers_list.join(" "),
        signature_b64
    );

    Ok(SignatureHeaders {
        signature: signature_header,
        date,
        digest,
    })
}

/// Headers to add for signed request
#[derive(Debug, Clone)]
pub struct SignatureHeaders {
    /// Signature header value
    pub signature: String,
    /// Date header value (RFC 2616)
    pub date: String,
    /// Digest header value (if body present)
    pub digest: Option<String>,
}

/// Verify an HTTP request signature
///
/// # Arguments
/// * `method` - HTTP method
/// * `path` - Request path
/// * `headers` - All request headers
/// * `body` - Request body (for digest verification)
/// * `public_key_pem` - RSA public key in PEM format
///
/// # Returns
/// Ok if signature is valid
///
/// # Errors
/// - InvalidSignature if verification fails
/// - AppError::Federation if key fetch fails
pub fn verify_signature(
    method: &str,
    path: &str,
    headers: &http::HeaderMap,
    body: Option<&[u8]>,
    public_key_pem: &str,
) -> Result<(), AppError> {
    // 1. Parse Signature header
    let signature_header = headers
        .get("signature")
        .ok_or_else(|| AppError::Validation("Missing Signature header".to_string()))?
        .to_str()
        .map_err(|_| AppError::Validation("Invalid Signature header".to_string()))?;

    let parsed = parse_signature_header(signature_header)?;

    // 2. Validate algorithm and required signed headers.
    if parsed.algorithm != "rsa-sha256" && parsed.algorithm != "hs2019" {
        return Err(AppError::Validation(format!(
            "Unsupported signature algorithm: {}",
            parsed.algorithm
        )));
    }

    for required in ["(request-target)", "host", "date"] {
        if !parsed.headers.iter().any(|h| h == required) {
            return Err(AppError::Validation(format!(
                "Signed headers must include: {}",
                required
            )));
        }
    }

    if body.is_some() && !parsed.headers.iter().any(|h| h == "digest") {
        return Err(AppError::Validation(
            "Signed headers must include: digest".to_string(),
        ));
    }

    // 3. Verify Date is recent (within 5 minutes).
    let date_header = headers
        .get("date")
        .ok_or_else(|| AppError::Validation("Missing Date header".to_string()))?;
    let date_str = date_header
        .to_str()
        .map_err(|_| AppError::Validation("Invalid Date header".to_string()))?;

    let date = DateTime::parse_from_rfc2822(date_str)
        .map_err(|_| AppError::Validation("Invalid Date format".to_string()))?;

    let now = Utc::now();
    let diff = (now.timestamp() - date.timestamp()).abs();

    if diff > 300 {
        return Err(AppError::Validation(
            "Date header too old or in future".to_string(),
        ));
    }

    // 4. If body present, verify Digest.
    if let Some(body_data) = body {
        let digest_header = headers
            .get("digest")
            .ok_or_else(|| AppError::Validation("Missing Digest header".to_string()))?;
        let digest_str = digest_header
            .to_str()
            .map_err(|_| AppError::Validation("Invalid Digest header".to_string()))?;

        let expected_digest = generate_digest(body_data);
        if digest_str != expected_digest {
            return Err(AppError::Validation("Digest mismatch".to_string()));
        }
    }

    // 5. Reconstruct signing string.
    let mut signing_parts = Vec::new();

    for header_name in &parsed.headers {
        let value = match header_name.as_str() {
            "(request-target)" => format!("{} {}", method.to_lowercase(), path),
            "host" => headers
                .get("host")
                .ok_or_else(|| AppError::Validation("Missing host header".to_string()))?
                .to_str()
                .map_err(|_| AppError::Validation("Invalid host header".to_string()))?
                .to_string(),
            "date" => headers
                .get("date")
                .ok_or_else(|| AppError::Validation("Missing date header".to_string()))?
                .to_str()
                .map_err(|_| AppError::Validation("Invalid date header".to_string()))?
                .to_string(),
            "digest" => headers
                .get("digest")
                .ok_or_else(|| AppError::Validation("Missing digest header".to_string()))?
                .to_str()
                .map_err(|_| AppError::Validation("Invalid digest header".to_string()))?
                .to_string(),
            _ => {
                return Err(AppError::Validation(format!(
                    "Unsupported header in signature: {}",
                    header_name
                )));
            }
        };

        signing_parts.push(format!("{}: {}", header_name, value));
    }

    let signing_string = signing_parts.join("\n");

    // 6. Verify RSA signature.
    let signature_bytes = BASE64
        .decode(&parsed.signature)
        .map_err(|_| AppError::Validation("Invalid signature encoding".to_string()))?;

    // Parse the public key.
    let public_key = RsaPublicKey::from_public_key_pem(public_key_pem)
        .map_err(|e| AppError::Validation(format!("Invalid public key: {}", e)))?;

    // Create verifier (use new_unprefixed for compatibility).
    let verifier = rsa::pkcs1v15::VerifyingKey::<Sha256>::new_unprefixed(public_key);

    // Parse signature.
    let signature = Pkcs1v15Signature::try_from(signature_bytes.as_slice())
        .map_err(|e| AppError::Validation(format!("Invalid signature format: {}", e)))?;

    // Verify.
    verifier
        .verify(signing_string.as_bytes(), &signature)
        .map_err(|_| AppError::Validation("Signature verification failed".to_string()))?;

    Ok(())
}

/// Extract keyId from Signature header.
pub fn extract_signature_key_id(headers: &http::HeaderMap) -> Result<String, AppError> {
    let signature_header = headers
        .get("signature")
        .ok_or_else(|| AppError::Validation("Missing Signature header".to_string()))?
        .to_str()
        .map_err(|_| AppError::Validation("Invalid Signature header".to_string()))?;

    let parsed = parse_signature_header(signature_header)?;
    Ok(parsed.key_id)
}

/// Validate that signature keyId points to the same actor as the activity actor.
pub fn key_id_matches_actor(key_id: &str, actor_id: &str) -> bool {
    let key_actor = key_id.split('#').next().unwrap_or(key_id);
    let actor = actor_id.split('#').next().unwrap_or(actor_id);
    key_actor == actor
}

/// Parsed Signature header
#[derive(Debug, Clone)]
pub struct ParsedSignature {
    /// Key ID (URL to public key)
    pub key_id: String,
    /// Algorithm (usually rsa-sha256)
    pub algorithm: String,
    /// Signed header names
    pub headers: Vec<String>,
    /// Base64-encoded signature
    pub signature: String,
}

/// Parse Signature header value
///
/// # Format
/// ```text
/// keyId="...",algorithm="...",headers="...",signature="..."
/// ```
pub fn parse_signature_header(header: &str) -> Result<ParsedSignature, AppError> {
    let mut key_id = None;
    let mut algorithm = None;
    let mut headers = None;
    let mut signature = None;

    // Split by comma and parse key=value pairs
    for part in header.split(',') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            let key = key.trim();
            // Remove quotes from value
            let value = value.trim().trim_matches('"');

            match key {
                "keyId" => key_id = Some(value.to_string()),
                "algorithm" => algorithm = Some(value.to_string()),
                "headers" => {
                    headers = Some(
                        value
                            .split_whitespace()
                            .map(|s| s.to_ascii_lowercase())
                            .collect(),
                    )
                }
                "signature" => signature = Some(value.to_string()),
                _ => {} // Ignore unknown fields
            }
        }
    }

    Ok(ParsedSignature {
        key_id: key_id.ok_or_else(|| AppError::Validation("Missing keyId".to_string()))?,
        algorithm: algorithm
            .ok_or_else(|| AppError::Validation("Missing algorithm".to_string()))?,
        headers: headers.ok_or_else(|| AppError::Validation("Missing headers".to_string()))?,
        signature: signature
            .ok_or_else(|| AppError::Validation("Missing signature".to_string()))?,
    })
}

/// Generate SHA-256 digest for body
///
/// # Returns
/// `SHA-256=base64(hash)`
pub fn generate_digest(body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body);
    let hash = hasher.finalize();
    format!("SHA-256={}", BASE64.encode(hash))
}

/// Fetch public key from key ID URL
///
/// # Arguments
/// * `key_id` - Full URL to the key (e.g., actor#main-key)
/// * `http_client` - HTTP client
///
/// # Returns
/// PEM-encoded public key
pub async fn fetch_public_key(
    key_id: &str,
    http_client: &reqwest::Client,
) -> Result<String, AppError> {
    // Validate actor URL/domain and extract actor document URL.
    let actor_domain = extract_actor_domain(key_id)?;
    let actor_url = key_id.split('#').next().unwrap_or(key_id);
    let parsed_actor_url = url::Url::parse(actor_url)
        .map_err(|e| AppError::Validation(format!("Invalid actor URL: {}", e)))?;
    let actor_port = parsed_actor_url
        .port_or_known_default()
        .ok_or_else(|| AppError::Validation("Missing port in actor URL".to_string()))?;

    // Resolve DNS before fetching and reject local/private destinations.
    // This reduces SSRF risk for hosts that look public but resolve internally.
    validate_resolved_host_ips(&actor_domain, actor_port).await?;

    // Fetch actor document
    let response = http_client
        .get(actor_url)
        .header("Accept", "application/activity+json")
        .send()
        .await
        .map_err(|e| AppError::Federation(format!("Failed to fetch actor: {}", e)))?;

    if !response.status().is_success() {
        return Err(AppError::Federation(format!(
            "Failed to fetch actor: HTTP {}",
            response.status()
        )));
    }

    let actor: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Federation(format!("Failed to parse actor: {}", e)))?;

    let public_key = actor
        .get("publicKey")
        .ok_or_else(|| AppError::Federation("Missing publicKey in actor".to_string()))?;

    // If a key fragment is provided, ensure actor advertises exactly that key id.
    if key_id.contains('#') {
        let advertised_key_id = public_key
            .get("id")
            .and_then(|id| id.as_str())
            .ok_or_else(|| AppError::Federation("Missing publicKey.id in actor".to_string()))?;

        if advertised_key_id != key_id {
            return Err(AppError::Validation(
                "Signature keyId does not match actor public key id".to_string(),
            ));
        }
    }

    // Extract public key
    let public_key_pem = public_key
        .get("publicKeyPem")
        .and_then(|pem| pem.as_str())
        .ok_or_else(|| AppError::Federation("Missing publicKeyPem in actor".to_string()))?;

    Ok(public_key_pem.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderMap, HeaderValue};
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::{RsaPrivateKey, RsaPublicKey};

    fn generate_test_keypair() -> (String, String) {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 1024).expect("key generation should work");
        let public_key = RsaPublicKey::from(&private_key);

        let private_key_pem = private_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("private key pem")
            .to_string();
        let public_key_pem = public_key
            .to_public_key_pem(LineEnding::LF)
            .expect("public key pem");

        (private_key_pem, public_key_pem)
    }

    fn build_signed_header_map(
        method: &str,
        url: &str,
        body: Option<&[u8]>,
        private_key_pem: &str,
    ) -> (HeaderMap, String) {
        let key_id = "https://remote.example/users/alice#main-key";
        let signed = sign_request(method, url, body, private_key_pem, key_id).expect("signed");
        let parsed_url = url::Url::parse(url).expect("valid test url");
        let host = parsed_url.host_str().expect("host");
        let path = parsed_url.path();
        let path_and_query = if let Some(query) = parsed_url.query() {
            format!("{}?{}", path, query)
        } else {
            path.to_string()
        };

        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_str(host).expect("host header"));
        headers.insert(
            "date",
            HeaderValue::from_str(&signed.date).expect("date header"),
        );
        if let Some(digest) = signed.digest {
            headers.insert(
                "digest",
                HeaderValue::from_str(&digest).expect("digest header"),
            );
        }
        headers.insert(
            "signature",
            HeaderValue::from_str(&signed.signature).expect("signature header"),
        );

        (headers, path_and_query)
    }

    #[test]
    fn verify_signature_accepts_valid_signed_request() {
        let (private_key_pem, public_key_pem) = generate_test_keypair();
        let body = br#"{"type":"Follow"}"#;
        let (headers, path) = build_signed_header_map(
            "POST",
            "https://remote.example/inbox?foo=bar",
            Some(body),
            &private_key_pem,
        );

        let result = verify_signature("POST", &path, &headers, Some(body), &public_key_pem);
        assert!(result.is_ok(), "valid signature should verify: {result:?}");
    }

    #[test]
    fn verify_signature_rejects_missing_date_header() {
        let (private_key_pem, public_key_pem) = generate_test_keypair();
        let body = br#"{"type":"Follow"}"#;
        let (mut headers, path) = build_signed_header_map(
            "POST",
            "https://remote.example/inbox",
            Some(body),
            &private_key_pem,
        );
        headers.remove("date");

        match verify_signature("POST", &path, &headers, Some(body), &public_key_pem) {
            Err(AppError::Validation(msg)) => assert!(msg.contains("Missing Date header")),
            other => panic!("expected missing Date header error, got: {other:?}"),
        }
    }

    #[test]
    fn verify_signature_rejects_missing_digest_header_for_body() {
        let (private_key_pem, public_key_pem) = generate_test_keypair();
        let body = br#"{"type":"Follow"}"#;
        let (mut headers, path) = build_signed_header_map(
            "POST",
            "https://remote.example/inbox",
            Some(body),
            &private_key_pem,
        );
        headers.remove("digest");

        match verify_signature("POST", &path, &headers, Some(body), &public_key_pem) {
            Err(AppError::Validation(msg)) => assert!(msg.contains("Missing Digest header")),
            other => panic!("expected missing Digest header error, got: {other:?}"),
        }
    }

    #[test]
    fn verify_signature_rejects_when_date_not_in_signed_headers() {
        let (private_key_pem, public_key_pem) = generate_test_keypair();
        let body = br#"{"type":"Follow"}"#;
        let (mut headers, path) = build_signed_header_map(
            "POST",
            "https://remote.example/inbox",
            Some(body),
            &private_key_pem,
        );

        let signature_header = headers
            .get("signature")
            .expect("signature")
            .to_str()
            .expect("signature str");
        let parsed = parse_signature_header(signature_header).expect("parsed signature");
        let tampered = format!(
            "keyId=\"{}\",algorithm=\"{}\",headers=\"(request-target) host digest\",signature=\"{}\"",
            parsed.key_id, parsed.algorithm, parsed.signature
        );
        headers.insert(
            "signature",
            HeaderValue::from_str(&tampered).expect("tampered signature"),
        );

        match verify_signature("POST", &path, &headers, Some(body), &public_key_pem) {
            Err(AppError::Validation(msg)) => {
                assert!(msg.contains("Signed headers must include: date"))
            }
            other => panic!("expected missing signed date error, got: {other:?}"),
        }
    }

    #[test]
    fn extract_actor_domain_rejects_localhost() {
        match extract_actor_domain("https://localhost/users/alice#main-key") {
            Err(AppError::Forbidden) => {}
            other => panic!("expected forbidden for localhost, got: {other:?}"),
        }
    }

    #[test]
    fn extract_actor_domain_rejects_private_ip() {
        match extract_actor_domain("http://192.168.1.10/users/alice#main-key") {
            Err(AppError::Forbidden) => {}
            other => panic!("expected forbidden for private ip, got: {other:?}"),
        }
    }

    #[test]
    fn extract_actor_domain_accepts_public_host() {
        let domain = extract_actor_domain("https://example.com/users/alice#main-key")
            .expect("public host should be accepted");
        assert_eq!(domain, "example.com");
    }

    #[tokio::test]
    async fn validate_resolved_host_ips_rejects_localhost() {
        match validate_resolved_host_ips("localhost", 80).await {
            Err(AppError::Forbidden) => {}
            other => panic!("expected forbidden for localhost resolution, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn validate_resolved_host_ips_rejects_private_ip() {
        match validate_resolved_host_ips("127.0.0.1", 80).await {
            Err(AppError::Forbidden) => {}
            other => panic!("expected forbidden for private ip resolution, got: {other:?}"),
        }
    }

    #[test]
    fn extract_signature_key_id_reads_key_id() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "signature",
            HeaderValue::from_static(
                "keyId=\"https://remote.example/users/alice#main-key\",algorithm=\"rsa-sha256\",headers=\"(request-target) host date\",signature=\"ZmFrZQ==\"",
            ),
        );

        let key_id = extract_signature_key_id(&headers).expect("keyId should be parsed");
        assert_eq!(key_id, "https://remote.example/users/alice#main-key");
    }

    #[test]
    fn key_id_matches_actor_accepts_same_actor() {
        assert!(key_id_matches_actor(
            "https://remote.example/users/alice#main-key",
            "https://remote.example/users/alice",
        ));
    }

    #[test]
    fn key_id_matches_actor_rejects_different_actor() {
        assert!(!key_id_matches_actor(
            "https://remote.example/users/bob#main-key",
            "https://remote.example/users/alice",
        ));
    }
}
