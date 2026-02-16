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
        if let Some(digest_header) = headers.get("digest") {
            let digest_str = digest_header
                .to_str()
                .map_err(|_| AppError::Validation("Invalid Digest header".to_string()))?;

            let expected_digest = generate_digest(body_data);
            if digest_str != expected_digest {
                return Err(AppError::Validation("Digest mismatch".to_string()));
            }
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
                    headers = Some(value.split_whitespace().map(|s| s.to_string()).collect())
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
    // Extract actor URL (remove fragment if present)
    let actor_url = key_id.split('#').next().unwrap_or(key_id);

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

    // Extract public key
    let public_key_pem = actor
        .get("publicKey")
        .and_then(|pk| pk.get("publicKeyPem"))
        .and_then(|pem| pem.as_str())
        .ok_or_else(|| AppError::Federation("Missing publicKeyPem in actor".to_string()))?;

    Ok(public_key_pem.to_string())
}
