# ActivityPub RSA Key Specification

## Overview

This document describes the RSA key requirements and specifications for ActivityPub HTTP Signatures, based on the implementation in RustResort and ActivityPub community standards.

## RSA Key Requirements

### Key Size

**Minimum**: 2048 bits  
**Recommended**: 2048 bits or larger  
**Current Implementation**: 2048 bits

```rust
let bits = 2048;
let private_key = RsaPrivateKey::new(&mut rng, bits)?;
```

**Rationale:**
- 1024-bit RSA keys are no longer considered secure
- 2048-bit keys provide adequate security for the foreseeable future
- Larger keys (4096 bits) are acceptable but increase computational overhead

### Key Format

**Private Key**: PKCS#8 PEM format  
**Public Key**: PKCS#1 PEM format (SubjectPublicKeyInfo)

**Example Private Key (PKCS#8 PEM)**:
```
-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQC...
-----END PRIVATE KEY-----
```

**Example Public Key (PKCS#1 PEM)**:
```
-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAvN...
-----END PUBLIC KEY-----
```

### Key Generation

```rust
use rsa::{RsaPrivateKey, RsaPublicKey};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};

// Generate keypair
let mut rng = rand::thread_rng();
let bits = 2048;
let private_key = RsaPrivateKey::new(&mut rng, bits)?;
let public_key = RsaPublicKey::from(&private_key);

// Encode to PEM
let private_key_pem = private_key
    .to_pkcs8_pem(LineEnding::LF)?
    .to_string();
let public_key_pem = public_key
    .to_public_key_pem(LineEnding::LF)?;
```

## Public Key in Actor Object

### Actor Document Structure

The public key is embedded in the ActivityPub actor object:

```json
{
  "@context": [
    "https://www.w3.org/ns/activitystreams",
    "https://w3id.org/security/v1"
  ],
  "id": "https://example.com/users/alice",
  "type": "Person",
  "preferredUsername": "alice",
  "inbox": "https://example.com/users/alice/inbox",
  "outbox": "https://example.com/users/alice/outbox",
  "publicKey": {
    "id": "https://example.com/users/alice#main-key",
    "owner": "https://example.com/users/alice",
    "publicKeyPem": "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAvN...\n-----END PUBLIC KEY-----"
  }
}
```

### Key Fields

| Field | Description | Required |
|-------|-------------|----------|
| `id` | Full URI to the key (actor URL + `#main-key`) | Yes |
| `owner` | Actor ID (must match the actor containing this key) | Yes |
| `publicKeyPem` | PEM-encoded RSA public key | Yes |

### Key ID Convention

The key ID typically follows the pattern:
```
{actor_url}#main-key
```

Examples:
- `https://example.com/users/alice#main-key`
- `https://mastodon.social/users/bob#main-key`

## HTTP Signature Algorithm

### Signature Algorithm

**Algorithm**: `rsa-sha256`  
**Alternative**: `hs2019` (maps to `rsa-sha256` in practice)

**Specification**: PKCS#1 v1.5 with SHA-256

```rust
use rsa::pkcs1v15::SigningKey;
use sha2::Sha256;

let signing_key = SigningKey::<Sha256>::new_unprefixed(private_key);
let signature = signing_key.sign_with_rng(&mut rng, message.as_bytes());
```

### Signed Headers

**Minimum Required Headers**:
1. `(request-target)` - Pseudo-header: `{method} {path}`
2. `host` - Target server hostname
3. `date` - Request timestamp (RFC 2822 format)

**Additional Headers (for POST requests)**:
4. `digest` - SHA-256 hash of request body

**Example Signature Header**:
```
Signature: keyId="https://example.com/users/alice#main-key",
           algorithm="rsa-sha256",
           headers="(request-target) host date digest",
           signature="Base64EncodedSignature=="
```

### Signing String Construction

The signing string is constructed by concatenating header values:

```
(request-target): post /users/bob/inbox
host: remote.example.com
date: Tue, 07 Jun 2022 12:34:56 GMT
digest: SHA-256=X48E9qOokqqrvdts8nOJRJN3OWDUoyWxBf7kbu9DBPE=
```

Each line follows the format: `{header_name}: {header_value}`

## Digest Calculation

For POST requests with a body, a digest header is required:

```rust
use sha2::{Digest, Sha256};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

fn generate_digest(body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body);
    let hash = hasher.finalize();
    format!("SHA-256={}", BASE64.encode(hash))
}
```

**Example**:
```
Digest: SHA-256=X48E9qOokqqrvdts8nOJRJN3OWDUoyWxBf7kbu9DBPE=
```

## Signature Verification

### Verification Process

1. **Parse Signature Header**: Extract `keyId`, `algorithm`, `headers`, `signature`
2. **Fetch Public Key**: Retrieve public key from `keyId` URL
3. **Validate Date**: Ensure request is recent (within 5 minutes)
4. **Verify Digest**: If body present, verify SHA-256 digest
5. **Reconstruct Signing String**: Rebuild from specified headers
6. **Verify Signature**: Use RSA public key to verify signature

### Date Validation

**Tolerance**: ±5 minutes (300 seconds)

```rust
let now = Utc::now();
let diff = (now.timestamp() - date.timestamp()).abs();

if diff > 300 {
    return Err("Date header too old or in future");
}
```

**Rationale**: Prevents replay attacks while allowing for clock skew

### Verification Code

```rust
use rsa::pkcs1v15::VerifyingKey;
use rsa::signature::Verifier;

// Parse public key
let public_key = RsaPublicKey::from_public_key_pem(public_key_pem)?;

// Create verifier
let verifier = VerifyingKey::<Sha256>::new_unprefixed(public_key);

// Verify signature
verifier.verify(signing_string.as_bytes(), &signature)?;
```

## Security Considerations

### Key Storage

**Private Key**:
- Store securely in database (encrypted at rest recommended)
- Never expose in API responses
- Rotate periodically (recommended: annually)

**Public Key**:
- Publicly accessible via actor object
- Can be cached by remote servers
- Include in all actor document responses

### Key Rotation

When rotating keys:
1. Generate new keypair
2. Update actor object with new public key
3. Keep old private key for verification of in-flight requests (grace period)
4. Update all outgoing signatures to use new key
5. After grace period (e.g., 24 hours), delete old private key

### Replay Attack Prevention

**Mechanisms**:
1. **Date Header Validation**: Reject requests older than 5 minutes
2. **Digest Verification**: Ensure body hasn't been tampered with
3. **Signature Verification**: Cryptographic proof of authenticity

### Common Pitfalls

1. **Incorrect PEM Format**: Ensure proper line endings and headers
2. **Clock Skew**: Synchronize server time with NTP
3. **Missing Headers**: Always include minimum required headers
4. **Case Sensitivity**: Header names are case-insensitive, but values may not be
5. **URL Encoding**: Ensure proper encoding in `(request-target)`

## Implementation Checklist

- [ ] Generate 2048-bit RSA keypair
- [ ] Store private key securely (PKCS#8 PEM)
- [ ] Expose public key in actor object (PKCS#1 PEM)
- [ ] Include `publicKey` object with `id`, `owner`, `publicKeyPem`
- [ ] Sign outgoing requests with `rsa-sha256`
- [ ] Include minimum headers: `(request-target)`, `host`, `date`
- [ ] Add `digest` header for POST requests
- [ ] Verify incoming signatures
- [ ] Validate date within ±5 minutes
- [ ] Verify digest for POST requests
- [ ] Cache remote public keys (with TTL)
- [ ] Handle key rotation gracefully

## References

- [ActivityPub Specification](https://www.w3.org/TR/activitypub/)
- [HTTP Signatures Draft](https://datatracker.ietf.org/doc/html/draft-cavage-http-signatures-12)
- [Mastodon HTTP Signatures](https://docs.joinmastodon.org/spec/security/)
- [W3C Security Vocabulary](https://w3id.org/security/v1)

## RustResort Implementation

See:
- `src/federation/signature.rs` - Signature generation and verification
- `src/lib.rs` - Admin user keypair generation
- `docs/FEDERATION.md` - Federation specification

---

**Last Updated**: 2026-01-12
