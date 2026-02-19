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
use std::collections::HashSet;
use std::net::IpAddr;

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

fn is_supported_signature_algorithm(algorithm: &str) -> bool {
    algorithm.eq_ignore_ascii_case("rsa-sha256") || algorithm.eq_ignore_ascii_case("hs2019")
}

fn parse_actor_url(raw: &str) -> Result<url::Url, AppError> {
    let mut parsed = url::Url::parse(raw)
        .map_err(|_| AppError::Validation("Invalid actor URL in keyId".to_string()))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(AppError::Validation(
            "Actor URL in keyId must use http or https".to_string(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::Validation(
            "Actor URL in keyId must not include user info".to_string(),
        ));
    }
    if parsed.host_str().is_none() {
        return Err(AppError::Validation(
            "Actor URL in keyId must include a host".to_string(),
        ));
    }
    parsed.set_fragment(None);
    Ok(parsed)
}

fn is_blocked_ip_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_unspecified()
                || v6.is_multicast()
        }
    }
}

fn validate_remote_actor_url(actor_url: &url::Url) -> Result<(), AppError> {
    let host = actor_url
        .host_str()
        .ok_or_else(|| AppError::Validation("Actor URL in keyId must include a host".to_string()))?
        .trim_end_matches('.')
        .to_ascii_lowercase();

    if host == "localhost" || host.ends_with(".localhost") {
        return Err(AppError::Validation(
            "Actor URL host is not allowed".to_string(),
        ));
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip_address(ip) {
            return Err(AppError::Validation(
                "Actor URL host is not allowed".to_string(),
            ));
        }
    }

    Ok(())
}

/// Returns true when the actor URL derived from keyId matches activity actor.
pub fn key_id_matches_actor(key_id: &str, actor_id: &str) -> Result<bool, AppError> {
    let key_actor = parse_actor_url(key_id)?;
    let actor = parse_actor_url(actor_id)
        .map_err(|_| AppError::Validation("Invalid activity actor URL".to_string()))?;
    Ok(key_actor == actor)
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
    if !is_supported_signature_algorithm(&parsed.algorithm) {
        return Err(AppError::Validation(
            "Unsupported signature algorithm".to_string(),
        ));
    }
    let signed_headers: HashSet<&str> = parsed.headers.iter().map(String::as_str).collect();
    for required_header in ["(request-target)", "host", "date"] {
        if !signed_headers.contains(required_header) {
            return Err(AppError::Validation(format!(
                "Signature must include {} header",
                required_header
            )));
        }
    }

    // 2. Verify Date is recent (within 5 minutes)
    if let Some(date_header) = headers.get("date") {
        let date_str = date_header
            .to_str()
            .map_err(|_| AppError::Validation("Invalid Date header".to_string()))?;

        // Parse RFC 2822 date format
        let date = DateTime::parse_from_rfc2822(date_str)
            .map_err(|_| AppError::Validation("Invalid Date format".to_string()))?;

        let now = Utc::now();
        let diff = (now.timestamp() - date.timestamp()).abs();

        if diff > 300 {
            // 5 minutes
            return Err(AppError::Validation(
                "Date header too old or in future".to_string(),
            ));
        }
    }

    // 3. If body present, verify Digest
    if let Some(body_data) = body {
        if !signed_headers.contains("digest") {
            return Err(AppError::Validation(
                "Signature must include digest header for requests with body".to_string(),
            ));
        }
        let digest_header = headers
            .get("digest")
            .ok_or_else(|| AppError::Validation("Missing digest header".to_string()))?;
        let digest_str = digest_header
            .to_str()
            .map_err(|_| AppError::Validation("Invalid Digest header".to_string()))?;

        let expected_digest = generate_digest(body_data);
        if digest_str != expected_digest {
            return Err(AppError::Validation("Digest mismatch".to_string()));
        }
    }

    // 4. Reconstruct signing string
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

    // 5. Verify RSA signature
    let signature_bytes = BASE64
        .decode(&parsed.signature)
        .map_err(|_| AppError::Validation("Invalid signature encoding".to_string()))?;

    // Parse the public key
    let public_key = RsaPublicKey::from_public_key_pem(public_key_pem)
        .map_err(|e| AppError::Validation(format!("Invalid public key: {}", e)))?;

    // Create verifier (use new_unprefixed for compatibility)
    let verifier = rsa::pkcs1v15::VerifyingKey::<Sha256>::new_unprefixed(public_key);

    // Parse signature
    let signature = Pkcs1v15Signature::try_from(signature_bytes.as_slice())
        .map_err(|e| AppError::Validation(format!("Invalid signature format: {}", e)))?;

    // Verify
    verifier
        .verify(signing_string.as_bytes(), &signature)
        .map_err(|_| AppError::Validation("Signature verification failed".to_string()))?;

    Ok(())
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
    let actor_url = parse_actor_url(key_id)?;
    validate_remote_actor_url(&actor_url)?;

    // Fetch actor document
    let response = http_client
        .get(actor_url.as_str())
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

    // Extract public key
    let public_key_pem = actor
        .get("publicKey")
        .and_then(|pk| pk.get("publicKeyPem"))
        .and_then(|pem| pem.as_str())
        .ok_or_else(|| AppError::Federation("Missing publicKeyPem in actor".to_string()))?;

    Ok(public_key_pem.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::{RsaPrivateKey, RsaPublicKey};

    fn generate_test_keypair() -> (String, String) {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = RsaPublicKey::from(&private_key);
        let private_key_pem = private_key
            .to_pkcs8_pem(LineEnding::LF)
            .unwrap()
            .to_string();
        let public_key_pem = public_key.to_public_key_pem(LineEnding::LF).unwrap();
        (private_key_pem, public_key_pem)
    }

    #[test]
    fn verify_signature_requires_digest_header_for_body() {
        let (private_key_pem, public_key_pem) = generate_test_keypair();
        let body = br#"{"type":"Create"}"#;
        let signed = sign_request(
            "POST",
            "https://remote.example/inbox",
            Some(body),
            &private_key_pem,
            "https://remote.example/users/alice#main-key",
        )
        .unwrap();

        let mut headers = http::HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("remote.example"));
        headers.insert(
            "date",
            HeaderValue::from_str(&signed.date).expect("valid signed date"),
        );
        headers.insert(
            "signature",
            HeaderValue::from_str(&signed.signature).expect("valid signature header"),
        );
        // Intentionally omit Digest header.

        let error = verify_signature("POST", "/inbox", &headers, Some(body), &public_key_pem)
            .expect_err("digest header must be required");
        assert!(matches!(
            error,
            AppError::Validation(message) if message.contains("Missing digest header")
        ));
    }

    #[test]
    fn verify_signature_requires_digest_to_be_signed_for_body() {
        let (private_key_pem, public_key_pem) = generate_test_keypair();
        let body = br#"{"type":"Create"}"#;
        // Sign a request without body so "digest" is not part of signed headers.
        let signed = sign_request(
            "POST",
            "https://remote.example/inbox",
            None,
            &private_key_pem,
            "https://remote.example/users/alice#main-key",
        )
        .unwrap();

        let mut headers = http::HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("remote.example"));
        headers.insert(
            "date",
            HeaderValue::from_str(&signed.date).expect("valid signed date"),
        );
        headers.insert(
            "signature",
            HeaderValue::from_str(&signed.signature).expect("valid signature header"),
        );
        headers.insert(
            "digest",
            HeaderValue::from_str(&generate_digest(body)).expect("valid digest"),
        );

        let error = verify_signature("POST", "/inbox", &headers, Some(body), &public_key_pem)
            .expect_err("digest must be part of signed headers");
        assert!(matches!(
            error,
            AppError::Validation(message)
                if message.contains("Signature must include digest header")
        ));
    }

    #[test]
    fn verify_signature_accepts_hs2019_algorithm_token() {
        let (private_key_pem, public_key_pem) = generate_test_keypair();
        let body = br#"{"type":"Create"}"#;
        let signed = sign_request(
            "POST",
            "https://remote.example/inbox",
            Some(body),
            &private_key_pem,
            "https://remote.example/users/alice#main-key",
        )
        .unwrap();
        let hs2019_signature =
            signed
                .signature
                .replacen("algorithm=\"rsa-sha256\"", "algorithm=\"hs2019\"", 1);

        let mut headers = http::HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("remote.example"));
        headers.insert(
            "date",
            HeaderValue::from_str(&signed.date).expect("valid signed date"),
        );
        headers.insert(
            "digest",
            HeaderValue::from_str(&generate_digest(body)).expect("valid digest"),
        );
        headers.insert(
            "signature",
            HeaderValue::from_str(&hs2019_signature).expect("valid signature header"),
        );

        verify_signature("POST", "/inbox", &headers, Some(body), &public_key_pem)
            .expect("hs2019 token should be accepted for rsa signatures");
    }

    #[test]
    fn key_id_matches_actor_accepts_matching_actor_document_url() {
        let matches = key_id_matches_actor(
            "https://remote.example/users/alice#main-key",
            "https://remote.example/users/alice",
        )
        .expect("valid actor URLs");
        assert!(matches);
    }

    #[test]
    fn key_id_matches_actor_rejects_mismatched_actor_document_url() {
        let matches = key_id_matches_actor(
            "https://remote.example/users/bob#main-key",
            "https://remote.example/users/alice",
        )
        .expect("valid actor URLs");
        assert!(!matches);
    }

    #[tokio::test]
    async fn fetch_public_key_rejects_localhost_targets() {
        let client = reqwest::Client::new();
        let error = fetch_public_key("http://127.0.0.1/users/alice#main-key", &client)
            .await
            .expect_err("localhost/private targets must be rejected");
        assert!(matches!(
            error,
            AppError::Validation(message) if message.contains("not allowed")
        ));
    }
}
