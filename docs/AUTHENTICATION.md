# RustResort Authentication Design

## Overview

RustResort is a single-user instance and supports **GitHub OAuth only** for authentication.
Only the GitHub user specified in the configuration can log in to the instance.

## Authentication Flow

### GitHub OAuth 2.0

```
┌──────────┐     ┌──────────────┐     ┌──────────────┐
│  User    │     │  RustResort  │     │   GitHub     │
│ (Admin)  │     │              │     │              │
└────┬─────┘     └──────┬───────┘     └──────┬───────┘
     │                  │                    │
     │  1. /login       │                    │
     │─────────────────▶│                    │
     │                  │                    │
     │  2. Redirect to GitHub               │
     │◀─────────────────│                    │
     │                  │                    │
     │  3. GitHub Login Page                │
     │─────────────────────────────────────▶│
     │                  │                    │
     │  4. User authorizes                  │
     │◀─────────────────────────────────────│
     │                  │                    │
     │  5. Callback with code               │
     │─────────────────▶│                    │
     │                  │                    │
     │                  │  6. Exchange code  │
     │                  │   for access token │
     │                  │───────────────────▶│
     │                  │                    │
     │                  │  7. Access token   │
     │                  │◀───────────────────│
     │                  │                    │
     │                  │  8. Get user info  │
     │                  │───────────────────▶│
     │                  │                    │
     │                  │  9. User info      │
     │                  │◀───────────────────│
     │                  │                    │
     │                  │  10. Verify GitHub │
     │                  │      username      │
     │                  │                    │
     │  11. Session cookie                  │
     │◀─────────────────│                    │
     │                  │                    │
```

### Single-User Authentication

RustResort allows only **one GitHub username** specified in the configuration to log in:

```toml
[auth]
# GitHub username allowed to login (this becomes the instance admin)
github_username = "your-github-username"
```

Login attempts from other GitHub users are rejected.

## Configuration

### 1. Create GitHub OAuth App

1. GitHub → Settings → Developer settings → OAuth Apps → New OAuth App
2. Enter the following:
   - **Application name**: `RustResort`
   - **Homepage URL**: `https://social.example.com`
   - **Authorization callback URL**: `https://social.example.com/auth/github/callback`
3. Note the Client ID and Client Secret

### 2. Configuration File

```toml
[auth]
# GitHub username allowed to login
github_username = "your-github-username"

# Session settings
session_secret = "${SESSION_SECRET}"  # Random string of 32+ bytes
session_max_age = 604800              # 7 days (seconds)

[auth.github]
client_id = "${GITHUB_CLIENT_ID}"
client_secret = "${GITHUB_CLIENT_SECRET}"
```

### 3. Environment Variables

```bash
# GitHub OAuth
export GITHUB_CLIENT_ID="Iv1.xxxxxxxxxxxx"
export GITHUB_CLIENT_SECRET="xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"

# Session secret (generate with: openssl rand -base64 32)
export SESSION_SECRET="$(openssl rand -base64 32)"
```

## Implementation

### Authentication Router

```rust
use axum::{
    routing::{get, post},
    Router,
};

pub fn auth_router() -> Router<AppState> {
    Router::new()
        .route("/login", get(login_page))
        .route("/auth/github", get(github_redirect))
        .route("/auth/github/callback", get(github_callback))
        .route("/logout", post(logout))
}
```

### Login Page

```rust
/// GET /login
/// Display simple login page
async fn login_page() -> impl IntoResponse {
    Html(r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>Login - RustResort</title>
            <style>
                body {
                    font-family: system-ui, sans-serif;
                    display: flex;
                    justify-content: center;
                    align-items: center;
                    height: 100vh;
                    margin: 0;
                    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
                }
                .login-box {
                    background: white;
                    padding: 2rem;
                    border-radius: 8px;
                    box-shadow: 0 4px 6px rgba(0, 0, 0, 0.1);
                    text-align: center;
                }
                .github-btn {
                    display: inline-flex;
                    align-items: center;
                    gap: 0.5rem;
                    background: #24292e;
                    color: white;
                    padding: 0.75rem 1.5rem;
                    border-radius: 6px;
                    text-decoration: none;
                    font-weight: 500;
                }
                .github-btn:hover {
                    background: #1b1f23;
                }
            </style>
        </head>
        <body>
            <div class="login-box">
                <h1>🏝️ RustResort</h1>
                <p>Sign in to manage your instance</p>
                <a href="/auth/github" class="github-btn">
                    <svg height="20" width="20" viewBox="0 0 16 16" fill="currentColor">
                        <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/>
                    </svg>
                    Sign in with GitHub
                </a>
            </div>
        </body>
        </html>
    "#)
}
```

### GitHub OAuth Redirect

```rust
/// GET /auth/github
/// Redirect to GitHub authorization page
async fn github_redirect(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let csrf_state = generate_csrf_state();
    
    // Store CSRF token in session
    // ...
    
    let auth_url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&scope=read:user&state={}",
        state.config.auth.github.client_id,
        urlencoding::encode(&format!("{}/auth/github/callback", state.config.server.base_url())),
        csrf_state,
    );
    
    Redirect::temporary(&auth_url)
}
```

### GitHub Callback

```rust
#[derive(Deserialize)]
struct GitHubCallbackQuery {
    code: String,
    state: String,
}

#[derive(Deserialize)]
struct GitHubTokenResponse {
    access_token: String,
    token_type: String,
}

#[derive(Deserialize)]
struct GitHubUser {
    login: String,
    id: u64,
    avatar_url: String,
    name: Option<String>,
}

/// GET /auth/github/callback
/// Handle GitHub callback
async fn github_callback(
    State(state): State<AppState>,
    Query(query): Query<GitHubCallbackQuery>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    // 1. Verify CSRF token
    verify_csrf_state(&query.state, &jar)?;
    
    // 2. Get access token
    let token_response: GitHubTokenResponse = state.http_client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", &state.config.auth.github.client_id),
            ("client_secret", &state.config.auth.github.client_secret),
            ("code", &query.code),
        ])
        .send()
        .await?
        .json()
        .await?;
    
    // 3. Get user info
    let github_user: GitHubUser = state.http_client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", token_response.access_token))
        .header("User-Agent", "RustResort")
        .send()
        .await?
        .json()
        .await?;
    
    // 4. Verify authorized user
    if github_user.login != state.config.auth.github_username {
        tracing::warn!(
            attempted_user = %github_user.login,
            allowed_user = %state.config.auth.github_username,
            "Unauthorized login attempt"
        );
        return Err(AppError::Unauthorized);
    }
    
    tracing::info!(user = %github_user.login, "Admin logged in");
    
    // 5. Create session
    let session = Session {
        github_username: github_user.login,
        github_id: github_user.id,
        avatar_url: github_user.avatar_url,
        name: github_user.name,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::seconds(state.config.auth.session_max_age),
    };
    
    let session_token = create_session_token(&session, &state.config.auth.session_secret)?;
    
    // 6. Set session cookie and redirect
    let cookie = Cookie::build(("session", session_token))
        .path("/")
        .http_only(true)
        .secure(state.config.server.protocol == "https")
        .same_site(SameSite::Lax)
        .max_age(time::Duration::seconds(state.config.auth.session_max_age))
        .build();
    
    Ok((jar.add(cookie), Redirect::to("/")))
}
```

### Logout

```rust
/// POST /logout
async fn logout(jar: CookieJar) -> impl IntoResponse {
    let cookie = Cookie::build(("session", ""))
        .path("/")
        .max_age(time::Duration::ZERO)
        .build();
    
    (jar.remove(cookie), Redirect::to("/login"))
}
```

### Authentication Middleware

```rust
use axum::middleware::Next;

/// Middleware to protect routes requiring authentication
pub async fn require_auth(
    State(state): State<AppState>,
    jar: CookieJar,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let session_token = jar
        .get("session")
        .map(|c| c.value().to_string())
        .ok_or(AppError::Unauthorized)?;
    
    let session = verify_session_token(&session_token, &state.config.auth.session_secret)?;
    
    // Check session expiration
    if session.expires_at < Utc::now() {
        return Err(AppError::Unauthorized);
    }
    
    // Add session info to request extensions
    let mut request = request;
    request.extensions_mut().insert(session);
    
    Ok(next.run(request).await)
}

/// Get current session info
pub struct CurrentUser(pub Session);

#[async_trait]
impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Session>()
            .cloned()
            .map(CurrentUser)
            .ok_or(AppError::Unauthorized)
    }
}
```

### Session Token

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub github_username: String,
    pub github_id: u64,
    pub avatar_url: String,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Create session token (signed JSON payload)
fn create_session_token(session: &Session, secret: &str) -> Result<String, Error> {
    let payload = serde_json::to_string(session)?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
    mac.update(payload_b64.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    
    Ok(format!("{}.{}", payload_b64, signature))
}

/// Verify and decode session token
fn verify_session_token(token: &str, secret: &str) -> Result<Session, Error> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 2 {
        return Err(Error::InvalidToken);
    }
    
    let (payload_b64, signature) = (parts[0], parts[1]);
    
    // Verify signature
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
    mac.update(payload_b64.as_bytes());
    let expected = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    
    if signature != expected {
        return Err(Error::InvalidSignature);
    }
    
    // Decode payload
    let payload = URL_SAFE_NO_PAD.decode(payload_b64)?;
    let session: Session = serde_json::from_slice(&payload)?;
    
    Ok(session)
}
```

## Router Configuration

```rust
use axum::middleware;

pub fn app_router(state: AppState) -> Router {
    Router::new()
        // Routes not requiring authentication
        .merge(auth_router())
        .merge(wellknown_router())
        .merge(activitypub_router())
        
        // Routes requiring authentication
        .nest("/api/v1", 
            mastodon_api_router()
                .layer(middleware::from_fn_with_state(state.clone(), require_auth))
        )
        .nest("/api/admin",
            admin_router()
                .layer(middleware::from_fn_with_state(state.clone(), require_auth))
        )
        
        .with_state(state)
}
```

## Mastodon API Authentication

For Mastodon client apps, OAuth 2.0 token authentication is also supported:

```rust
/// Token authentication middleware for Mastodon API
pub async fn require_api_token(
    State(state): State<AppState>,
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = state.db.get_token(auth.token()).await?
        .ok_or(AppError::Unauthorized)?;
    
    // Check token expiration
    if let Some(expires_at) = token.expires_at {
        if expires_at < Utc::now() {
            return Err(AppError::Unauthorized);
        }
    }
    
    Ok(next.run(request).await)
}
```

### Mastodon OAuth Flow

```
POST /api/v1/apps       → App registration
GET  /oauth/authorize   → Authorization page (redirects to GitHub OAuth)
POST /oauth/token       → Token issuance
```

## Security Considerations

### CSRF Token

```rust
fn generate_csrf_state() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::thread_rng().gen();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
```

### Session Security

| Setting | Value | Reason |
|---------|-------|--------|
| `HttpOnly` | true | Protection against XSS attacks |
| `Secure` | true (HTTPS) | Encrypted transport only |
| `SameSite` | Lax | CSRF protection |
| `Path` | / | Valid for all paths |

### Rate Limiting

```rust
use tower_governor::{GovernorLayer, GovernorConfigBuilder};

let governor_config = GovernorConfigBuilder::default()
    .per_second(1)
    .burst_size(5)
    .finish()
    .unwrap();

let auth_router = auth_router()
    .layer(GovernorLayer {
        config: &governor_config,
    });
```

## Dependencies

```toml
[dependencies]
# Authentication
hmac = "0.12"
sha2 = "0.10"
base64 = "0.21"
rand = "0.8"
urlencoding = "2"

# Cookie/Session
tower-cookies = "0.10"
axum-extra = { version = "0.9", features = ["typed-header", "cookie"] }

# Rate limiting
tower-governor = "0.3"
```

## Next Steps

- [API.md](./API.md) - Mastodon API specification
- [DEVELOPMENT.md](./DEVELOPMENT.md) - Development guide
