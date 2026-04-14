use std::{collections::HashMap, sync::Arc};

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::{ConnectInfo, FromRef, Path, State},
    http::HeaderMap,
    middleware,
    response::{Html, IntoResponse, Redirect},
    routing::{get, patch, post},
};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use time::Duration as CookieDuration;
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::*;

use crate::LocalAuthState;
use crate::auth::session::{Session, create_session_token, verify_session_token};
use crate::auth::{CurrentUser, check_auth_rate_limit, require_session_auth};
use crate::data::{Database, EntityId, PasskeyCredential};
use crate::error::AppError;

const AUTH_PASSWORD_HASH_SETTING_KEY: &str = "auth_password_hash";
const AUTH_WEBAUTHN_USER_ID_SETTING_KEY: &str = "auth_webauthn_user_id";
const SESSION_COOKIE: &str = "session";

#[derive(Default)]
struct PendingPasskeyState {
    registrations: HashMap<String, PasskeyRegistration>,
    authentications: HashMap<String, PasskeyAuthentication>,
}

#[derive(Clone)]
pub struct LocalAuthService {
    webauthn: Webauthn,
    pending: Arc<Mutex<PendingPasskeyState>>,
}

impl LocalAuthService {
    pub fn new(config: &crate::config::AppConfig) -> Result<Self, AppError> {
        let origin = Url::parse(&config.server.base_url())
            .map_err(|error| AppError::internal(format!("invalid local auth origin: {error}")))?;
        let rp_id = origin
            .host_str()
            .ok_or_else(|| AppError::internal("local auth origin missing host"))?;
        let builder = WebauthnBuilder::new(rp_id, &origin).map_err(|error| {
            AppError::internal(format!("failed to build webauthn config: {error}"))
        })?;
        let webauthn = builder
            .rp_name(&config.instance.title)
            .build()
            .map_err(|error| {
                AppError::internal(format!("failed to initialize webauthn: {error}"))
            })?;

        Ok(Self {
            webauthn,
            pending: Arc::new(Mutex::new(PendingPasskeyState::default())),
        })
    }

    pub async fn start_passkey_registration(
        &self,
        user_id: Uuid,
        username: &str,
        display_name: &str,
    ) -> Result<(String, CreationChallengeResponse), AppError> {
        let (challenge, state) = self
            .webauthn
            .start_passkey_registration(user_id, username, display_name, None)
            .map_err(|error| {
                AppError::Validation(format!("failed to start passkey registration: {error}"))
            })?;
        let request_id = EntityId::new_string();
        self.pending
            .lock()
            .await
            .registrations
            .insert(request_id.clone(), state);
        Ok((request_id, challenge))
    }

    pub async fn finish_passkey_registration(
        &self,
        request_id: &str,
        credential: RegisterPublicKeyCredential,
    ) -> Result<Passkey, AppError> {
        let state = self
            .pending
            .lock()
            .await
            .registrations
            .remove(request_id)
            .ok_or_else(|| {
                AppError::Validation("unknown passkey registration request".to_string())
            })?;
        self.webauthn
            .finish_passkey_registration(&credential, &state)
            .map_err(|error| {
                AppError::Validation(format!("failed to finish passkey registration: {error}"))
            })
    }

    pub async fn start_passkey_authentication(
        &self,
        passkeys: &[Passkey],
    ) -> Result<(String, RequestChallengeResponse), AppError> {
        let (challenge, state) = self
            .webauthn
            .start_passkey_authentication(passkeys)
            .map_err(|error| {
                AppError::Validation(format!("failed to start passkey authentication: {error}"))
            })?;
        let request_id = EntityId::new_string();
        self.pending
            .lock()
            .await
            .authentications
            .insert(request_id.clone(), state);
        Ok((request_id, challenge))
    }

    pub async fn finish_passkey_authentication(
        &self,
        request_id: &str,
        credential: PublicKeyCredential,
    ) -> Result<AuthenticationResult, AppError> {
        let state = self
            .pending
            .lock()
            .await
            .authentications
            .remove(request_id)
            .ok_or_else(|| {
                AppError::Validation("unknown passkey authentication request".to_string())
            })?;
        self.webauthn
            .finish_passkey_authentication(&credential, &state)
            .map_err(|error| {
                AppError::Validation(format!("failed to finish passkey authentication: {error}"))
            })
    }
}

pub async fn ensure_local_auth_config(
    db: &Database,
    config: &crate::config::AppConfig,
) -> Result<(), AppError> {
    if db
        .get_setting(AUTH_PASSWORD_HASH_SETTING_KEY)
        .await?
        .is_none()
    {
        let password = config
            .auth
            .password
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::Config(
                    "auth.password is required to initialize local authentication".to_string(),
                )
            })?;
        let password_hash = hash_password(password)?;
        db.set_setting(AUTH_PASSWORD_HASH_SETTING_KEY, &password_hash)
            .await?;
    }

    if db
        .get_setting(AUTH_WEBAUTHN_USER_ID_SETTING_KEY)
        .await?
        .is_none()
    {
        db.set_setting(
            AUTH_WEBAUTHN_USER_ID_SETTING_KEY,
            &Uuid::new_v4().to_string(),
        )
        .await?;
    }

    Ok(())
}

fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| AppError::Encryption(error.to_string()))
}

async fn verify_local_password(db: &Database, password: &str) -> Result<bool, AppError> {
    let Some(password_hash) = db.get_setting(AUTH_PASSWORD_HASH_SETTING_KEY).await? else {
        return Ok(false);
    };
    let parsed_hash = PasswordHash::new(&password_hash)
        .map_err(|error| AppError::Encryption(error.to_string()))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

async fn set_local_password(db: &Database, password: &str) -> Result<(), AppError> {
    let trimmed = password.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation(
            "new password must not be blank".to_string(),
        ));
    }

    let password_hash = hash_password(trimmed)?;
    db.set_setting(AUTH_PASSWORD_HASH_SETTING_KEY, &password_hash)
        .await
}

async fn local_webauthn_user_id(db: &Database) -> Result<Uuid, AppError> {
    let Some(value) = db.get_setting(AUTH_WEBAUTHN_USER_ID_SETTING_KEY).await? else {
        return Err(AppError::Internal(
            "local auth webauthn user id is not initialized".to_string(),
        ));
    };
    Uuid::parse_str(&value)
        .map_err(|error| AppError::internal(format!("invalid stored webauthn user id: {error}")))
}

fn build_session(
    account: &crate::data::Account,
    auth_method: &str,
    session_max_age: i64,
) -> Session {
    let now = Utc::now();
    Session {
        username: account.username.clone(),
        display_name: account.display_name.clone(),
        auth_method: auth_method.to_string(),
        created_at: now,
        expires_at: now + Duration::seconds(session_max_age),
    }
}

fn build_session_cookie(session_token: &str, secure: bool) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, session_token.to_string()))
        .path("/")
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .max_age(CookieDuration::days(30))
        .build()
}

fn clear_cookie(name: &str, secure: bool) -> Cookie<'static> {
    Cookie::build((name.to_string(), String::new()))
        .path("/")
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .max_age(CookieDuration::seconds(0))
        .build()
}

fn credential_id_string(passkey: &Passkey) -> String {
    URL_SAFE_NO_PAD.encode(passkey.cred_id().as_ref())
}

fn deserialize_passkey(record: &PasskeyCredential) -> Result<Passkey, AppError> {
    serde_json::from_str(&record.passkey_json)
        .map_err(|error| AppError::internal(format!("invalid stored passkey json: {error}")))
}

async fn load_passkey_records(
    db: &Database,
) -> Result<Vec<(PasskeyCredential, Passkey)>, AppError> {
    db.list_passkeys()
        .await?
        .into_iter()
        .map(|record| deserialize_passkey(&record).map(|passkey| (record, passkey)))
        .collect()
}

#[derive(Debug, Deserialize)]
struct PasswordLoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct LoginResponse {
    access_token: String,
    token_type: &'static str,
    username: String,
    auth_method: String,
}

#[derive(Debug, Serialize)]
struct SessionResponse {
    username: String,
    display_name: Option<String>,
    auth_method: String,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct PasskeyRegistrationFinishRequest {
    request_id: String,
    credential: RegisterPublicKeyCredential,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PasskeyAuthenticationStartRequest {
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PasskeyAuthenticationFinishRequest {
    request_id: String,
    credential: PublicKeyCredential,
}

#[derive(Debug, Deserialize)]
struct UpdatePasswordRequest {
    current_password: String,
    new_password: String,
}

#[derive(Debug, Deserialize)]
struct UpdatePasskeyRequest {
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct PasskeyChallengeResponse<T> {
    request_id: String,
    #[serde(flatten)]
    options: T,
}

#[derive(Debug, Serialize)]
struct PasskeyListItem {
    id: String,
    credential_id: String,
    name: Option<String>,
    created_at: chrono::DateTime<Utc>,
    last_used_at: Option<chrono::DateTime<Utc>>,
}

pub fn auth_router<S>(config: Arc<crate::config::AppConfig>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    LocalAuthState: FromRef<S>,
{
    let protected_routes = Router::new()
        .route("/auth/session", get(get_session))
        .route("/auth/password", post(update_password))
        .route("/auth/passkeys", get(list_passkeys))
        .route(
            "/auth/passkeys/:id",
            patch(update_passkey).delete(delete_passkey),
        )
        .route(
            "/auth/passkeys/register/start",
            post(start_passkey_registration),
        )
        .route(
            "/auth/passkeys/register/finish",
            post(finish_passkey_registration),
        )
        .route_layer(middleware::from_fn_with_state(config, require_session_auth));

    Router::new()
        .route("/login", get(login_page))
        .route("/settings", get(settings_page))
        .route("/auth/login", post(login_password))
        .route(
            "/auth/passkeys/auth/start",
            post(start_passkey_authentication),
        )
        .route(
            "/auth/passkeys/auth/finish",
            post(finish_passkey_authentication),
        )
        .route("/logout", post(logout))
        .merge(protected_routes)
}

async fn login_page() -> impl IntoResponse {
    Html(
        r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Login - RustResort</title>
  <style>
    body { font-family: sans-serif; max-width: 36rem; margin: 3rem auto; padding: 0 1rem; }
    form, .actions { display: grid; gap: 0.75rem; }
    input, button { font: inherit; padding: 0.7rem 0.9rem; }
    .muted { color: #555; font-size: 0.95rem; }
    .hidden { display: none; }
    .row { display: flex; gap: 0.75rem; align-items: center; flex-wrap: wrap; }
    ul { padding-left: 1.25rem; }
  </style>
</head>
<body>
  <h1>RustResort</h1>
  <p class="muted">Sign in with the built-in local account.</p>
  <section id="login-section">
  <form id="password-form">
    <input id="username" name="username" autocomplete="username webauthn" placeholder="Username" required />
    <input id="password" name="password" type="password" autocomplete="current-password" placeholder="Password" required />
    <button type="submit">Sign in with password</button>
  </form>
  <div class="actions">
    <button id="passkey-login" type="button">Sign in with passkey</button>
  </div>
  </section>
  <section id="session-section" class="hidden">
    <h2>Authenticated</h2>
    <p class="muted" id="session-summary"></p>
    <div class="row">
      <input id="passkey-name" placeholder="Passkey name (optional)" />
      <button id="passkey-register" type="button">Register current device passkey</button>
      <button id="logout-button" type="button">Log out</button>
    </div>
    <h3>Registered passkeys</h3>
    <ul id="passkey-list"></ul>
  </section>
  <pre id="message"></pre>
  <script>
    const loginSection = document.getElementById('login-section');
    const sessionSection = document.getElementById('session-section');
    const sessionSummary = document.getElementById('session-summary');
    const passkeyList = document.getElementById('passkey-list');
    const message = document.getElementById('message');
    const loginNext = (() => {
      const candidate = new URLSearchParams(window.location.search).get('next');
      if (!candidate || !candidate.startsWith('/') || candidate.startsWith('//')) {
        return '/settings';
      }
      return candidate;
    })();
    const show = (value) => { message.textContent = value; };
    const b64urlToBytes = (input) => {
      const padding = '='.repeat((4 - input.length % 4) % 4);
      const base64 = (input + padding).replace(/-/g, '+').replace(/_/g, '/');
      const binary = atob(base64);
      return Uint8Array.from(binary, (char) => char.charCodeAt(0));
    };
    const bytesToB64url = (bytes) => {
      const binary = String.fromCharCode(...new Uint8Array(bytes));
      return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
    };
    const encodeCredential = (credential) => JSON.stringify({
      id: credential.id,
      rawId: bytesToB64url(credential.rawId),
      type: credential.type,
      response: {
        clientDataJSON: bytesToB64url(credential.response.clientDataJSON),
        attestationObject: credential.response.attestationObject ? bytesToB64url(credential.response.attestationObject) : undefined,
        authenticatorData: credential.response.authenticatorData ? bytesToB64url(credential.response.authenticatorData) : undefined,
        signature: credential.response.signature ? bytesToB64url(credential.response.signature) : undefined,
        userHandle: credential.response.userHandle ? bytesToB64url(credential.response.userHandle) : null
      }
    });
    async function refreshSession() {
      const response = await fetch('/auth/session', { credentials: 'same-origin' });
      if (!response.ok) {
        loginSection.classList.remove('hidden');
        sessionSection.classList.add('hidden');
        return null;
      }
      const session = await response.json();
      loginSection.classList.add('hidden');
      sessionSection.classList.remove('hidden');
      sessionSummary.textContent = `${session.username} via ${session.auth_method}`;
      return session;
    }
    async function loadPasskeys() {
      const response = await fetch('/auth/passkeys', { credentials: 'same-origin' });
      if (!response.ok) {
        passkeyList.innerHTML = '<li>Failed to load passkeys</li>';
        return;
      }
      const passkeys = await response.json();
      if (!passkeys.length) {
        passkeyList.innerHTML = '<li>No passkeys registered</li>';
        return;
      }
      passkeyList.innerHTML = '';
      for (const passkey of passkeys) {
        const item = document.createElement('li');
        const label = passkey.name || passkey.credential_id;
        const text = document.createElement('span');
        text.textContent = `${label}`;
        const remove = document.createElement('button');
        remove.type = 'button';
        remove.textContent = 'Delete';
        remove.addEventListener('click', async () => {
          const response = await fetch(`/auth/passkeys/${passkey.id}`, {
            method: 'DELETE',
            credentials: 'same-origin'
          });
          if (!response.ok) {
            show('Failed to delete passkey');
            return;
          }
          await loadPasskeys();
        });
        item.appendChild(text);
        item.appendChild(document.createTextNode(' '));
        item.appendChild(remove);
        passkeyList.appendChild(item);
      }
    }
    async function loginWithPassword(event) {
      event.preventDefault();
      const response = await fetch('/auth/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          username: document.getElementById('username').value,
          password: document.getElementById('password').value
        })
      });
      if (!response.ok) {
        show('Password login failed');
        return;
      }
      location.href = loginNext;
    }
    async function loginWithPasskey() {
      if (!window.PublicKeyCredential) {
        show('This browser does not support passkeys');
        return;
      }
      const start = await fetch('/auth/passkeys/auth/start', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ username: document.getElementById('username').value || null })
      });
      if (!start.ok) {
        show('Passkey login could not be started');
        return;
      }
      const options = await start.json();
      const publicKey = options.publicKey;
      publicKey.challenge = b64urlToBytes(publicKey.challenge);
      publicKey.allowCredentials = (publicKey.allowCredentials || []).map((credential) => ({
        ...credential,
        id: b64urlToBytes(credential.id)
      }));
      const credential = await navigator.credentials.get({ publicKey });
      const finish = await fetch('/auth/passkeys/auth/finish', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          request_id: options.request_id,
          credential: JSON.parse(encodeCredential(credential))
        })
      });
      if (!finish.ok) {
        show('Passkey login failed');
        return;
      }
      location.href = loginNext;
    }
    async function registerPasskey() {
      if (!window.PublicKeyCredential) {
        show('This browser does not support passkeys');
        return;
      }
      const start = await fetch('/auth/passkeys/register/start', {
        method: 'POST',
        credentials: 'same-origin'
      });
      if (!start.ok) {
        show('Passkey registration could not be started');
        return;
      }
      const options = await start.json();
      const publicKey = options.publicKey;
      publicKey.challenge = b64urlToBytes(publicKey.challenge);
      publicKey.user.id = b64urlToBytes(publicKey.user.id);
      publicKey.excludeCredentials = (publicKey.excludeCredentials || []).map((credential) => ({
        ...credential,
        id: b64urlToBytes(credential.id)
      }));
      const credential = await navigator.credentials.create({ publicKey });
      const finish = await fetch('/auth/passkeys/register/finish', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'same-origin',
        body: JSON.stringify({
          request_id: options.request_id,
          name: document.getElementById('passkey-name').value || null,
          credential: JSON.parse(encodeCredential(credential))
        })
      });
      if (!finish.ok) {
        show('Passkey registration failed');
        return;
      }
      await loadPasskeys();
      show('Passkey registered');
    }
    async function logoutCurrentSession() {
      await fetch('/logout', {
        method: 'POST',
        credentials: 'same-origin'
      });
      await refreshSession();
      show('Logged out');
    }
    document.getElementById('password-form').addEventListener('submit', loginWithPassword);
    document.getElementById('passkey-login').addEventListener('click', loginWithPasskey);
    document.getElementById('passkey-register').addEventListener('click', registerPasskey);
    document.getElementById('logout-button').addEventListener('click', logoutCurrentSession);
    refreshSession().then((session) => {
      if (session) {
        document.getElementById('username').value = session.username;
        loadPasskeys();
      }
    });
  </script>
</body>
</html>"#,
    )
}

async fn settings_page(
    State(state): State<LocalAuthState>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    let Some(token) = jar
        .get(SESSION_COOKIE)
        .map(|cookie| cookie.value().to_string())
    else {
        return Ok(Redirect::to("/login").into_response());
    };
    if verify_session_token(&token, &state.config.auth.session_secret).is_err() {
        return Ok(Redirect::to("/login").into_response());
    }

    Ok(Html(
        r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>User Settings - RustResort</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f5efe6;
      --panel: #fffaf3;
      --line: #d8c8b5;
      --ink: #2e241d;
      --muted: #6d5d4f;
      --accent: #0c6b58;
      --accent-2: #b85c38;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      font-family: Georgia, "Times New Roman", serif;
      color: var(--ink);
      background:
        radial-gradient(circle at top left, #fef7ed, transparent 30%),
        linear-gradient(180deg, #efe3d2, var(--bg));
    }
    main {
      max-width: 72rem;
      margin: 0 auto;
      padding: 2rem 1rem 4rem;
      display: grid;
      gap: 1.25rem;
    }
    .hero {
      background: linear-gradient(135deg, #fffaf3, #f7ecdf);
      border: 1px solid var(--line);
      padding: 1.5rem;
    }
    .hero h1 { margin: 0 0 0.5rem; font-size: 2rem; }
    .muted { color: var(--muted); }
    .grid {
      display: grid;
      gap: 1rem;
      grid-template-columns: repeat(auto-fit, minmax(20rem, 1fr));
    }
    .card {
      background: var(--panel);
      border: 1px solid var(--line);
      padding: 1rem;
      display: grid;
      gap: 0.85rem;
    }
    label { display: grid; gap: 0.35rem; font-weight: 600; }
    input, textarea, button { font: inherit; }
    input, textarea {
      width: 100%;
      padding: 0.75rem;
      border: 1px solid var(--line);
      background: #fff;
    }
    textarea { min-height: 8rem; resize: vertical; }
    button {
      border: 0;
      padding: 0.75rem 1rem;
      cursor: pointer;
      background: var(--accent);
      color: #fff;
    }
    button.secondary { background: #8c775f; }
    button.danger { background: var(--accent-2); }
    button.ghost {
      background: transparent;
      border: 1px solid var(--line);
      color: var(--ink);
    }
    .row {
      display: flex;
      gap: 0.75rem;
      flex-wrap: wrap;
      align-items: center;
    }
    .media-preview {
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 0.75rem;
    }
    .media-preview img {
      width: 100%;
      aspect-ratio: 16 / 9;
      object-fit: cover;
      border: 1px solid var(--line);
      background: #e9dccb;
    }
    .passkey-item {
      border: 1px solid var(--line);
      padding: 0.75rem;
      display: grid;
      gap: 0.5rem;
      background: #fff;
    }
    pre {
      margin: 0;
      white-space: pre-wrap;
      word-break: break-word;
      background: #201915;
      color: #fef7ed;
      padding: 0.85rem;
    }
  </style>
</head>
<body>
  <main>
    <section class="hero">
      <div class="row" style="justify-content: space-between;">
        <div>
          <h1>User Settings</h1>
          <p class="muted">Manage your local profile, password, and passkeys.</p>
        </div>
        <div class="row">
          <a href="/api/v1/accounts/verify_credentials"><button class="ghost" type="button">Raw account JSON</button></a>
          <button id="logout-button" class="secondary" type="button">Log out</button>
        </div>
      </div>
    </section>
    <div class="grid">
      <section class="card">
        <h2>Profile</h2>
        <label>Username
          <input id="username" disabled />
        </label>
        <label>Display name
          <input id="display-name" />
        </label>
        <label>Bio
          <textarea id="note"></textarea>
        </label>
        <label>Avatar image
          <input id="avatar-file" type="file" accept="image/*" />
        </label>
        <label>Header image
          <input id="header-file" type="file" accept="image/*" />
        </label>
        <div class="media-preview">
          <img id="avatar-preview" alt="Avatar preview" />
          <img id="header-preview" alt="Header preview" />
        </div>
        <div class="row">
          <button id="save-profile" type="button">Save profile</button>
        </div>
      </section>
      <section class="card">
        <h2>Password</h2>
        <label>Current password
          <input id="current-password" type="password" autocomplete="current-password" />
        </label>
        <label>New password
          <input id="new-password" type="password" autocomplete="new-password" />
        </label>
        <label>Repeat new password
          <input id="new-password-confirm" type="password" autocomplete="new-password" />
        </label>
        <div class="row">
          <button id="change-password" type="button">Change password</button>
        </div>
      </section>
    </div>
    <section class="card">
      <h2>Passkeys</h2>
      <p class="muted">Register this device or rename and remove stored passkeys.</p>
      <div class="row">
        <input id="new-passkey-name" placeholder="New passkey name (optional)" />
        <button id="register-passkey" type="button">Add passkey</button>
      </div>
      <div id="passkey-list" style="display:grid; gap:0.75rem;"></div>
    </section>
    <pre id="message"></pre>
  </main>
  <script>
    const message = document.getElementById('message');
    const show = (value) => { message.textContent = value; };
    const b64urlToBytes = (input) => {
      const padding = '='.repeat((4 - input.length % 4) % 4);
      const base64 = (input + padding).replace(/-/g, '+').replace(/_/g, '/');
      const binary = atob(base64);
      return Uint8Array.from(binary, (char) => char.charCodeAt(0));
    };
    const bytesToB64url = (bytes) => {
      const binary = String.fromCharCode(...new Uint8Array(bytes));
      return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
    };
    const encodeCredential = (credential) => JSON.stringify({
      id: credential.id,
      rawId: bytesToB64url(credential.rawId),
      type: credential.type,
      response: {
        clientDataJSON: bytesToB64url(credential.response.clientDataJSON),
        attestationObject: credential.response.attestationObject ? bytesToB64url(credential.response.attestationObject) : undefined,
        authenticatorData: credential.response.authenticatorData ? bytesToB64url(credential.response.authenticatorData) : undefined,
        signature: credential.response.signature ? bytesToB64url(credential.response.signature) : undefined,
        userHandle: credential.response.userHandle ? bytesToB64url(credential.response.userHandle) : null
      }
    });
    async function responseError(response) {
      const text = await response.text();
      return text || `HTTP ${response.status}`;
    }
    function fileToDataUrl(input) {
      const file = input.files && input.files[0];
      if (!file) return Promise.resolve(null);
      return new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(reader.result);
        reader.onerror = () => reject(new Error('failed to read file'));
        reader.readAsDataURL(file);
      });
    }
    async function loadProfile() {
      const response = await fetch('/api/v1/accounts/verify_credentials', {
        credentials: 'same-origin'
      });
      if (!response.ok) {
        show(`Failed to load profile: ${await responseError(response)}`);
        return;
      }
      const account = await response.json();
      document.getElementById('username').value = account.username || '';
      document.getElementById('display-name').value = account.display_name || '';
      document.getElementById('note').value = account.note || '';
      document.getElementById('avatar-preview').src = account.avatar || '';
      document.getElementById('header-preview').src = account.header || '';
    }
    async function saveProfile() {
      const avatar = await fileToDataUrl(document.getElementById('avatar-file'));
      const header = await fileToDataUrl(document.getElementById('header-file'));
      const response = await fetch('/api/v1/accounts/update_credentials', {
        method: 'PATCH',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          display_name: document.getElementById('display-name').value,
          note: document.getElementById('note').value,
          avatar,
          header
        })
      });
      if (!response.ok) {
        show(`Failed to save profile: ${await responseError(response)}`);
        return;
      }
      await loadProfile();
      document.getElementById('avatar-file').value = '';
      document.getElementById('header-file').value = '';
      show('Profile updated');
    }
    async function changePassword() {
      const currentPassword = document.getElementById('current-password').value;
      const newPassword = document.getElementById('new-password').value;
      const newPasswordConfirm = document.getElementById('new-password-confirm').value;
      if (newPassword !== newPasswordConfirm) {
        show('New passwords do not match');
        return;
      }
      const response = await fetch('/auth/password', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          current_password: currentPassword,
          new_password: newPassword
        })
      });
      if (!response.ok) {
        show(`Failed to change password: ${await responseError(response)}`);
        return;
      }
      document.getElementById('current-password').value = '';
      document.getElementById('new-password').value = '';
      document.getElementById('new-password-confirm').value = '';
      show('Password updated');
    }
    async function loadPasskeys() {
      const response = await fetch('/auth/passkeys', { credentials: 'same-origin' });
      if (!response.ok) {
        show(`Failed to load passkeys: ${await responseError(response)}`);
        return;
      }
      const passkeys = await response.json();
      const root = document.getElementById('passkey-list');
      root.innerHTML = '';
      if (!passkeys.length) {
        root.innerHTML = '<div class="passkey-item">No passkeys registered</div>';
        return;
      }
      for (const passkey of passkeys) {
        const item = document.createElement('div');
        item.className = 'passkey-item';
        const name = document.createElement('input');
        name.value = passkey.name || '';
        name.placeholder = passkey.credential_id;
        const meta = document.createElement('div');
        meta.className = 'muted';
        meta.textContent = `Credential: ${passkey.credential_id}`;
        const actions = document.createElement('div');
        actions.className = 'row';
        const save = document.createElement('button');
        save.type = 'button';
        save.textContent = 'Rename';
        save.addEventListener('click', async () => {
          const response = await fetch(`/auth/passkeys/${passkey.id}`, {
            method: 'PATCH',
            credentials: 'same-origin',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name: name.value || null })
          });
          if (!response.ok) {
            show(`Failed to update passkey: ${await responseError(response)}`);
            return;
          }
          await loadPasskeys();
          show('Passkey updated');
        });
        const remove = document.createElement('button');
        remove.type = 'button';
        remove.className = 'danger';
        remove.textContent = 'Delete';
        remove.addEventListener('click', async () => {
          const response = await fetch(`/auth/passkeys/${passkey.id}`, {
            method: 'DELETE',
            credentials: 'same-origin'
          });
          if (!response.ok) {
            show(`Failed to delete passkey: ${await responseError(response)}`);
            return;
          }
          await loadPasskeys();
          show('Passkey deleted');
        });
        actions.appendChild(save);
        actions.appendChild(remove);
        item.appendChild(name);
        item.appendChild(meta);
        item.appendChild(actions);
        root.appendChild(item);
      }
    }
    async function registerPasskey() {
      if (!window.PublicKeyCredential) {
        show('This browser does not support passkeys');
        return;
      }
      const start = await fetch('/auth/passkeys/register/start', {
        method: 'POST',
        credentials: 'same-origin'
      });
      if (!start.ok) {
        show(`Failed to start passkey registration: ${await responseError(start)}`);
        return;
      }
      const options = await start.json();
      const publicKey = options.publicKey;
      publicKey.challenge = b64urlToBytes(publicKey.challenge);
      publicKey.user.id = b64urlToBytes(publicKey.user.id);
      publicKey.excludeCredentials = (publicKey.excludeCredentials || []).map((credential) => ({
        ...credential,
        id: b64urlToBytes(credential.id)
      }));
      const credential = await navigator.credentials.create({ publicKey });
      const finish = await fetch('/auth/passkeys/register/finish', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          request_id: options.request_id,
          name: document.getElementById('new-passkey-name').value || null,
          credential: JSON.parse(encodeCredential(credential))
        })
      });
      if (!finish.ok) {
        show(`Failed to finish passkey registration: ${await responseError(finish)}`);
        return;
      }
      document.getElementById('new-passkey-name').value = '';
      await loadPasskeys();
      show('Passkey registered');
    }
    async function logout() {
      await fetch('/logout', { method: 'POST', credentials: 'same-origin' });
      location.href = '/login';
    }
    document.getElementById('save-profile').addEventListener('click', saveProfile);
    document.getElementById('change-password').addEventListener('click', changePassword);
    document.getElementById('register-passkey').addEventListener('click', registerPasskey);
    document.getElementById('logout-button').addEventListener('click', logout);
    Promise.all([loadProfile(), loadPasskeys()]).catch((error) => show(String(error)));
  </script>
</body>
</html>"#,
    )
    .into_response())
}

async fn login_password(
    State(state): State<LocalAuthState>,
    connect_info: Option<ConnectInfo<std::net::SocketAddr>>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(request): Json<PasswordLoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    let peer_addr = connect_info.as_ref().map(|ConnectInfo(addr)| *addr);
    check_auth_rate_limit(
        state.auth_rate_limiter.as_ref(),
        peer_addr,
        &headers,
        &state.config.server.trusted_proxy_ips,
        "password_login",
    )
    .await?;

    let account = state
        .db
        .get_account()
        .await?
        .ok_or(AppError::Unauthorized)?;
    if !account
        .username
        .eq_ignore_ascii_case(request.username.trim())
    {
        return Err(AppError::Unauthorized);
    }
    if !verify_local_password(state.db.as_ref(), &request.password).await? {
        return Err(AppError::Unauthorized);
    }

    let session = build_session(&account, "password", state.config.auth.session_max_age);
    let token = create_session_token(&session, &state.config.auth.session_secret)?;
    let cookie = build_session_cookie(&token, state.config.should_use_secure_cookies());

    Ok((
        jar.add(cookie),
        Json(LoginResponse {
            access_token: token,
            token_type: "Session",
            username: account.username,
            auth_method: "password".to_string(),
        }),
    ))
}

async fn logout(State(state): State<LocalAuthState>, jar: CookieJar) -> impl IntoResponse {
    (
        jar.remove(clear_cookie(
            SESSION_COOKIE,
            state.config.should_use_secure_cookies(),
        )),
        axum::http::StatusCode::NO_CONTENT,
    )
}

async fn get_session(CurrentUser(session): CurrentUser) -> Json<SessionResponse> {
    Json(SessionResponse {
        username: session.username,
        display_name: session.display_name,
        auth_method: session.auth_method,
        expires_at: session.expires_at,
    })
}

async fn update_password(
    State(state): State<LocalAuthState>,
    Json(request): Json<UpdatePasswordRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    if !verify_local_password(state.db.as_ref(), &request.current_password).await? {
        return Err(AppError::Unauthorized);
    }

    set_local_password(state.db.as_ref(), &request.new_password).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn list_passkeys(
    State(state): State<LocalAuthState>,
) -> Result<Json<Vec<PasskeyListItem>>, AppError> {
    let passkeys = state
        .db
        .list_passkeys()
        .await?
        .into_iter()
        .map(|passkey| PasskeyListItem {
            id: passkey.id,
            credential_id: passkey.credential_id,
            name: passkey.name,
            created_at: passkey.created_at,
            last_used_at: passkey.last_used_at,
        })
        .collect();

    Ok(Json(passkeys))
}

async fn update_passkey(
    State(state): State<LocalAuthState>,
    Path(id): Path<String>,
    Json(request): Json<UpdatePasskeyRequest>,
) -> Result<Json<PasskeyListItem>, AppError> {
    let mut passkey = state
        .db
        .get_passkey_by_id(&id)
        .await?
        .ok_or(AppError::NotFound)?;
    passkey.name = request
        .name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    passkey.updated_at = Utc::now();
    state.db.update_passkey(&passkey).await?;

    Ok(Json(PasskeyListItem {
        id: passkey.id,
        credential_id: passkey.credential_id,
        name: passkey.name,
        created_at: passkey.created_at,
        last_used_at: passkey.last_used_at,
    }))
}

async fn delete_passkey(
    State(state): State<LocalAuthState>,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, AppError> {
    state.db.delete_passkey(&id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn start_passkey_registration(
    State(state): State<LocalAuthState>,
) -> Result<Json<PasskeyChallengeResponse<CreationChallengeResponse>>, AppError> {
    let account = state
        .db
        .get_account()
        .await?
        .ok_or(AppError::Unauthorized)?;
    let user_id = local_webauthn_user_id(state.db.as_ref()).await?;
    let display_name = account
        .display_name
        .as_deref()
        .unwrap_or(account.username.as_str());
    let (request_id, public_key) = state
        .local_auth
        .start_passkey_registration(user_id, &account.username, display_name)
        .await?;

    Ok(Json(PasskeyChallengeResponse {
        request_id,
        options: public_key,
    }))
}

async fn finish_passkey_registration(
    State(state): State<LocalAuthState>,
    Json(request): Json<PasskeyRegistrationFinishRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    let passkey = state
        .local_auth
        .finish_passkey_registration(&request.request_id, request.credential)
        .await?;
    let now = Utc::now();
    let record = PasskeyCredential {
        id: EntityId::new_string(),
        credential_id: credential_id_string(&passkey),
        name: request
            .name
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        passkey_json: serde_json::to_string(&passkey)
            .map_err(|error| AppError::serialization("passkey encoding", error))?,
        created_at: now,
        updated_at: now,
        last_used_at: None,
    };
    state.db.insert_passkey(&record).await?;
    Ok(axum::http::StatusCode::CREATED)
}

async fn start_passkey_authentication(
    State(state): State<LocalAuthState>,
    connect_info: Option<ConnectInfo<std::net::SocketAddr>>,
    headers: HeaderMap,
    Json(request): Json<PasskeyAuthenticationStartRequest>,
) -> Result<Json<PasskeyChallengeResponse<RequestChallengeResponse>>, AppError> {
    let peer_addr = connect_info.as_ref().map(|ConnectInfo(addr)| *addr);
    check_auth_rate_limit(
        state.auth_rate_limiter.as_ref(),
        peer_addr,
        &headers,
        &state.config.server.trusted_proxy_ips,
        "passkey_auth_start",
    )
    .await?;

    let account = state
        .db
        .get_account()
        .await?
        .ok_or(AppError::Unauthorized)?;
    if let Some(username) = request.username.as_deref()
        && !username.trim().is_empty()
        && !account.username.eq_ignore_ascii_case(username.trim())
    {
        return Err(AppError::Unauthorized);
    }

    let passkeys = load_passkey_records(state.db.as_ref()).await?;
    if passkeys.is_empty() {
        return Err(AppError::Unauthorized);
    }

    let passkey_values: Vec<Passkey> = passkeys.into_iter().map(|(_, passkey)| passkey).collect();
    let (request_id, public_key) = state
        .local_auth
        .start_passkey_authentication(&passkey_values)
        .await?;

    Ok(Json(PasskeyChallengeResponse {
        request_id,
        options: public_key,
    }))
}

async fn finish_passkey_authentication(
    State(state): State<LocalAuthState>,
    connect_info: Option<ConnectInfo<std::net::SocketAddr>>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(request): Json<PasskeyAuthenticationFinishRequest>,
) -> Result<impl IntoResponse, AppError> {
    let peer_addr = connect_info.as_ref().map(|ConnectInfo(addr)| *addr);
    check_auth_rate_limit(
        state.auth_rate_limiter.as_ref(),
        peer_addr,
        &headers,
        &state.config.server.trusted_proxy_ips,
        "passkey_auth_finish",
    )
    .await?;

    let credential_id = request.credential.id.clone();
    let auth_result = state
        .local_auth
        .finish_passkey_authentication(&request.request_id, request.credential)
        .await?;
    let mut stored = state
        .db
        .list_passkeys()
        .await?
        .into_iter()
        .find(|record| record.credential_id == credential_id)
        .ok_or(AppError::Unauthorized)?;
    let mut passkey = deserialize_passkey(&stored)?;
    passkey.update_credential(&auth_result);
    stored.passkey_json = serde_json::to_string(&passkey)
        .map_err(|error| AppError::serialization("passkey encoding", error))?;
    stored.updated_at = Utc::now();
    stored.last_used_at = Some(Utc::now());
    state.db.update_passkey(&stored).await?;

    let account = state
        .db
        .get_account()
        .await?
        .ok_or(AppError::Unauthorized)?;
    let session = build_session(&account, "passkey", state.config.auth.session_max_age);
    let token = create_session_token(&session, &state.config.auth.session_secret)?;
    let cookie = build_session_cookie(&token, state.config.should_use_secure_cookies());

    Ok((
        jar.add(cookie),
        Json(LoginResponse {
            access_token: token,
            token_type: "Session",
            username: account.username,
            auth_method: "passkey".to_string(),
        }),
    ))
}
