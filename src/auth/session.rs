//! Session management
//!
//! Uses HMAC-signed tokens stored in cookies.
//! No server-side session storage needed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// User session data
///
/// Stored in a signed cookie. Contains minimal user info
/// from GitHub OAuth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// GitHub username
    pub github_username: String,
    /// GitHub user ID
    pub github_id: u64,
    /// Avatar URL from GitHub
    pub avatar_url: String,
    /// Display name from GitHub
    pub name: Option<String>,
    /// When session was created
    pub created_at: DateTime<Utc>,
    /// When session expires
    pub expires_at: DateTime<Utc>,
}

impl Session {
    /// Check if session is expired
    pub fn is_expired(&self) -> bool {
        self.expires_at < Utc::now()
    }
}

/// Create a signed session token
///
/// Token format: base64(payload).base64(hmac_sha256(payload))
///
/// # Arguments
/// * `session` - Session data to encode
/// * `secret` - HMAC secret key
///
/// # Returns
/// Signed token string
pub fn create_session_token(
    session: &Session,
    secret: &str,
) -> Result<String, crate::error::AppError> {
    use base64::{Engine as _, engine::general_purpose};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    // 1. Serialize session to JSON
    let payload =
        serde_json::to_string(session).map_err(|e| crate::error::AppError::Internal(e.into()))?;

    // 2. Base64 encode the payload
    let payload_b64 = general_purpose::URL_SAFE_NO_PAD.encode(payload.as_bytes());

    // 3. Create HMAC-SHA256 signature
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| crate::error::AppError::Encryption(e.to_string()))?;
    mac.update(payload_b64.as_bytes());
    let signature = mac.finalize().into_bytes();
    let signature_b64 = general_purpose::URL_SAFE_NO_PAD.encode(&signature);

    // 4. Return "{payload}.{signature}"
    Ok(format!("{}.{}", payload_b64, signature_b64))
}

/// Verify and decode a session token
///
/// # Arguments
/// * `token` - Token string to verify
/// * `secret` - HMAC secret key
///
/// # Returns
/// Decoded session if valid
///
/// # Errors
/// Returns error if signature is invalid or token is malformed
pub fn verify_session_token(token: &str, secret: &str) -> Result<Session, crate::error::AppError> {
    use base64::{Engine as _, engine::general_purpose};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    // 1. Split token into payload and signature
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 2 {
        return Err(crate::error::AppError::Unauthorized);
    }

    let payload_b64 = parts[0];
    let signature_b64 = parts[1];

    // 2. Verify HMAC signature
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| crate::error::AppError::Encryption(e.to_string()))?;
    mac.update(payload_b64.as_bytes());

    let expected_signature = general_purpose::URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|_| crate::error::AppError::Unauthorized)?;

    mac.verify_slice(&expected_signature)
        .map_err(|_| crate::error::AppError::InvalidSignature)?;

    // 3. Decode and deserialize payload
    let payload_bytes = general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| crate::error::AppError::Unauthorized)?;

    let payload_str =
        String::from_utf8(payload_bytes).map_err(|_| crate::error::AppError::Unauthorized)?;

    let session: Session =
        serde_json::from_str(&payload_str).map_err(|_| crate::error::AppError::Unauthorized)?;

    // 4. Check if session is expired
    if session.is_expired() {
        return Err(crate::error::AppError::Unauthorized);
    }

    Ok(session)
}
