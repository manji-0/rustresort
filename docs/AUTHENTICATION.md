# RustResort 認証設計

## 概要

RustResortはシングルユーザーインスタンスであり、認証は**GitHub OAuth**のみをサポートします。
設定されたGitHubユーザーのみがインスタンスにログイン可能です。

## 認証フロー

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

### シングルユーザー認証

RustResortでは、設定ファイルで指定された**1つのGitHubユーザー名**のみがログイン可能です：

```toml
[auth]
# 許可するGitHubユーザー名（これがインスタンスの管理者）
github_username = "your-github-username"
```

他のGitHubユーザーがログインしようとしても拒否されます。

## 設定

### 1. GitHub OAuth Appの作成

1. GitHub → Settings → Developer settings → OAuth Apps → New OAuth App
2. 以下を入力：
   - **Application name**: `RustResort`
   - **Homepage URL**: `https://social.example.com`
   - **Authorization callback URL**: `https://social.example.com/auth/github/callback`
3. Client ID と Client Secret をメモ

### 2. 設定ファイル

```toml
[auth]
# 許可するGitHubユーザー名
github_username = "your-github-username"

# セッション設定
session_secret = "${SESSION_SECRET}"  # 32バイト以上のランダム文字列
session_max_age = 604800              # 7日間（秒）

[auth.github]
client_id = "${GITHUB_CLIENT_ID}"
client_secret = "${GITHUB_CLIENT_SECRET}"
```

### 3. 環境変数

```bash
# GitHub OAuth
export GITHUB_CLIENT_ID="Iv1.xxxxxxxxxxxx"
export GITHUB_CLIENT_SECRET="xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"

# セッション秘密鍵（生成: openssl rand -base64 32）
export SESSION_SECRET="$(openssl rand -base64 32)"
```

## 実装

### 認証ルーター

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

### ログインページ

```rust
/// GET /login
/// シンプルなログインページを表示
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

### GitHub OAuth リダイレクト

```rust
/// GET /auth/github
/// GitHubの認可ページにリダイレクト
async fn github_redirect(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let csrf_state = generate_csrf_state();
    
    // CSRFトークンをセッションに保存
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

### GitHub コールバック

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
/// GitHubからのコールバックを処理
async fn github_callback(
    State(state): State<AppState>,
    Query(query): Query<GitHubCallbackQuery>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    // 1. CSRFトークンを検証
    verify_csrf_state(&query.state, &jar)?;
    
    // 2. アクセストークンを取得
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
    
    // 3. ユーザー情報を取得
    let github_user: GitHubUser = state.http_client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", token_response.access_token))
        .header("User-Agent", "RustResort")
        .send()
        .await?
        .json()
        .await?;
    
    // 4. 許可されたユーザーか確認
    if github_user.login != state.config.auth.github_username {
        tracing::warn!(
            attempted_user = %github_user.login,
            allowed_user = %state.config.auth.github_username,
            "Unauthorized login attempt"
        );
        return Err(AppError::Unauthorized);
    }
    
    tracing::info!(user = %github_user.login, "Admin logged in");
    
    // 5. セッションを作成
    let session = Session {
        github_username: github_user.login,
        github_id: github_user.id,
        avatar_url: github_user.avatar_url,
        name: github_user.name,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::seconds(state.config.auth.session_max_age),
    };
    
    let session_token = create_session_token(&session, &state.config.auth.session_secret)?;
    
    // 6. セッションCookieを設定してリダイレクト
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

### ログアウト

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

### 認証ミドルウェア

```rust
use axum::middleware::Next;

/// 認証が必要なルートを保護するミドルウェア
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
    
    // セッションの有効期限をチェック
    if session.expires_at < Utc::now() {
        return Err(AppError::Unauthorized);
    }
    
    // セッション情報をリクエスト拡張に追加
    let mut request = request;
    request.extensions_mut().insert(session);
    
    Ok(next.run(request).await)
}

/// 現在のセッション情報を取得
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

### セッショントークン

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

/// セッショントークンを作成（署名付きJSONペイロード）
fn create_session_token(session: &Session, secret: &str) -> Result<String, Error> {
    let payload = serde_json::to_string(session)?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
    mac.update(payload_b64.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    
    Ok(format!("{}.{}", payload_b64, signature))
}

/// セッショントークンを検証してデコード
fn verify_session_token(token: &str, secret: &str) -> Result<Session, Error> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 2 {
        return Err(Error::InvalidToken);
    }
    
    let (payload_b64, signature) = (parts[0], parts[1]);
    
    // 署名を検証
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
    mac.update(payload_b64.as_bytes());
    let expected = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    
    if signature != expected {
        return Err(Error::InvalidSignature);
    }
    
    // ペイロードをデコード
    let payload = URL_SAFE_NO_PAD.decode(payload_b64)?;
    let session: Session = serde_json::from_slice(&payload)?;
    
    Ok(session)
}
```

## ルーター構成

```rust
use axum::middleware;

pub fn app_router(state: AppState) -> Router {
    Router::new()
        // 認証不要なルート
        .merge(auth_router())
        .merge(wellknown_router())
        .merge(activitypub_router())
        
        // 認証必要なルート
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

## Mastodon API認証

Mastodonクライアントアプリ向けには、OAuth 2.0トークン認証もサポートします：

```rust
/// Mastodon API用のトークン認証ミドルウェア
pub async fn require_api_token(
    State(state): State<AppState>,
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = state.db.get_token(auth.token()).await?
        .ok_or(AppError::Unauthorized)?;
    
    // トークンの有効期限をチェック
    if let Some(expires_at) = token.expires_at {
        if expires_at < Utc::now() {
            return Err(AppError::Unauthorized);
        }
    }
    
    Ok(next.run(request).await)
}
```

### Mastodon OAuth フロー

```
POST /api/v1/apps       → アプリ登録
GET  /oauth/authorize   → 認可ページ（GitHub OAuthにリダイレクト）
POST /oauth/token       → トークン発行
```

## セキュリティ考慮事項

### CSRFトークン

```rust
fn generate_csrf_state() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::thread_rng().gen();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
```

### セッションセキュリティ

| 設定 | 値 | 理由 |
|------|-----|------|
| `HttpOnly` | true | XSS攻撃からの保護 |
| `Secure` | true (HTTPS) | 暗号化通信のみ |
| `SameSite` | Lax | CSRF保護 |
| `Path` | / | 全パスで有効 |

### レート制限

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

## 依存クレート

```toml
[dependencies]
# 認証
hmac = "0.12"
sha2 = "0.10"
base64 = "0.21"
rand = "0.8"
urlencoding = "2"

# Cookie/Session
tower-cookies = "0.10"
axum-extra = { version = "0.9", features = ["typed-header", "cookie"] }

# レート制限
tower-governor = "0.3"
```

## 次のステップ

- [API.md](./API.md) - Mastodon API仕様
- [DEVELOPMENT.md](./DEVELOPMENT.md) - 開発ガイド
