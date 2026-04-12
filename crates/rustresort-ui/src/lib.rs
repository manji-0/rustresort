use std::{cell::RefCell, rc::Rc};

use gloo_net::http::Request;
use html_escape::encode_text;
use serde::Deserialize;
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Document, Element, Event, HtmlInputElement, HtmlTextAreaElement, Window};

const APP_TITLE: &str = "RustResort";

#[derive(Clone, Copy, PartialEq, Eq)]
enum FeedMode {
    Home,
    Public,
}

impl FeedMode {
    fn as_path(self) -> &'static str {
        match self {
            Self::Home => "/api/v1/timelines/home?limit=20",
            Self::Public => "/api/v1/timelines/public?local=true&limit=20",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Public => "Local",
        }
    }
}

#[derive(Clone, Default)]
struct Model {
    loading: bool,
    feed_mode: FeedMode,
    session: Option<Session>,
    account: Option<Account>,
    statuses: Vec<Status>,
    notifications: Vec<Notification>,
    backups: Vec<BackupInfo>,
    domain_blocks: Vec<String>,
    flash: Option<String>,
}

impl Default for FeedMode {
    fn default() -> Self {
        Self::Home
    }
}

struct App {
    window: Window,
    document: Document,
    root: Element,
    model: RefCell<Model>,
}

#[derive(Clone, Deserialize)]
struct Session {
    username: String,
    auth_method: String,
}

#[derive(Clone, Deserialize)]
struct Source {
    note: String,
}

#[derive(Clone, Deserialize)]
struct Account {
    username: String,
    acct: String,
    display_name: String,
    avatar: String,
    source: Option<Source>,
    followers_count: i64,
    following_count: i64,
    statuses_count: i64,
}

#[derive(Clone, Deserialize)]
struct StatusAccount {
    display_name: String,
    acct: String,
    avatar: String,
}

#[derive(Clone, Deserialize)]
struct Status {
    created_at: String,
    content: String,
    replies_count: i64,
    reblogs_count: i64,
    favourites_count: i64,
    visibility: String,
    url: Option<String>,
    account: StatusAccount,
}

#[derive(Clone, Deserialize)]
struct Notification {
    #[serde(rename = "type")]
    notification_type: String,
    created_at: String,
    account: StatusAccount,
    status: Option<Status>,
}

#[derive(Clone, Deserialize)]
struct BackupInfo {
    key: String,
    size: u64,
    created_at: String,
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window not available"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("document not available"))?;
    let root = document
        .get_element_by_id("app")
        .ok_or_else(|| JsValue::from_str("#app not found"))?;

    let app = Rc::new(App {
        window,
        document,
        root,
        model: RefCell::new(Model::default()),
    });

    app.render();
    spawn_local({
        let app = app.clone();
        async move {
            app.load_dashboard().await;
        }
    });

    Ok(())
}

impl App {
    fn render(self: &Rc<Self>) {
        let model = self.model.borrow().clone();
        self.root.set_inner_html(&render_app(&model));
        self.attach_handlers();
    }

    fn attach_handlers(self: &Rc<Self>) {
        self.attach_click("nav-home", {
            let app = self.clone();
            move || {
                app.set_feed_mode(FeedMode::Home);
            }
        });
        self.attach_click("nav-public", {
            let app = self.clone();
            move || {
                app.set_feed_mode(FeedMode::Public);
            }
        });
        self.attach_click("refresh-feed", {
            let app = self.clone();
            move || {
                spawn_local({
                    let app = app.clone();
                    async move { app.refresh_feed().await }
                });
            }
        });
        self.attach_click("composer-submit", {
            let app = self.clone();
            move || {
                let Some(input) = app.textarea_value("composer-input") else {
                    return;
                };
                spawn_local({
                    let app = app.clone();
                    async move { app.create_status(input).await }
                });
            }
        });
        self.attach_click("logout-action", {
            let app = self.clone();
            move || {
                spawn_local({
                    let app = app.clone();
                    async move { app.logout().await }
                });
            }
        });
        self.attach_click("backup-action", {
            let app = self.clone();
            move || {
                spawn_local({
                    let app = app.clone();
                    async move { app.trigger_backup().await }
                });
            }
        });
        self.attach_click("domain-block-action", {
            let app = self.clone();
            move || {
                let Some(domain) = app.input_value("domain-block-input") else {
                    return;
                };
                spawn_local({
                    let app = app.clone();
                    async move { app.block_domain(domain).await }
                });
            }
        });

        if let Ok(node_list) = self.document.query_selector_all("[data-domain-remove]") {
            let len = node_list.length();
            for index in 0..len {
                let Some(node) = node_list.get(index) else {
                    continue;
                };
                let Ok(element) = node.dyn_into::<Element>() else {
                    continue;
                };
                let Some(domain) = element.get_attribute("data-domain-remove") else {
                    continue;
                };
                let closure = Closure::<dyn FnMut(_)>::wrap(Box::new({
                    let app = self.clone();
                    move |_event: Event| {
                        let domain = domain.clone();
                        spawn_local({
                            let app = app.clone();
                            async move { app.unblock_domain(domain).await }
                        });
                    }
                }));
                let _ = element
                    .add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
                closure.forget();
            }
        }
    }

    fn attach_click<F>(self: &Rc<Self>, id: &str, callback: F)
    where
        F: Fn() + 'static,
    {
        let Some(element) = self.document.get_element_by_id(id) else {
            return;
        };
        let closure = Closure::<dyn FnMut(_)>::wrap(Box::new(move |_event: Event| {
            callback();
        }));
        let _ = element.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
        closure.forget();
    }

    fn input_value(&self, id: &str) -> Option<String> {
        let element = self.document.get_element_by_id(id)?;
        let input: HtmlInputElement = element.dyn_into().ok()?;
        Some(input.value())
    }

    fn textarea_value(&self, id: &str) -> Option<String> {
        let element = self.document.get_element_by_id(id)?;
        let input: HtmlTextAreaElement = element.dyn_into().ok()?;
        Some(input.value())
    }

    fn set_feed_mode(self: &Rc<Self>, feed_mode: FeedMode) {
        {
            let mut model = self.model.borrow_mut();
            model.feed_mode = feed_mode;
            model.loading = true;
        }
        self.render();
        spawn_local({
            let app = self.clone();
            async move { app.refresh_feed().await }
        });
    }

    async fn load_dashboard(self: Rc<Self>) {
        {
            let mut model = self.model.borrow_mut();
            model.loading = true;
            model.flash = None;
        }
        self.render();

        let session = match fetch_json::<Session>("/auth/session").await {
            Ok(session) => session,
            Err(_) => {
                let _ = self.window.location().set_href("/login");
                return;
            }
        };

        let feed_mode = self.model.borrow().feed_mode;
        let (account, statuses, notifications, backups, domain_blocks) = futures_join5(
            fetch_json::<Account>("/api/v1/accounts/verify_credentials"),
            fetch_json::<Vec<Status>>(feed_mode.as_path()),
            fetch_json::<Vec<Notification>>("/api/v1/notifications?limit=8"),
            fetch_json::<Vec<BackupInfo>>("/admin/backups"),
            fetch_json::<Vec<String>>("/admin/domain_blocks"),
        )
        .await;

        let mut model = self.model.borrow_mut();
        model.session = Some(session);
        model.account = account.ok();
        model.statuses = statuses.unwrap_or_default();
        model.notifications = notifications.unwrap_or_default();
        model.backups = backups.unwrap_or_default();
        model.domain_blocks = domain_blocks.unwrap_or_default();
        model.loading = false;
        if model.account.is_none() {
            model.flash = Some("Failed to load account details.".to_string());
        }
        drop(model);
        self.render();
    }

    async fn refresh_feed(self: Rc<Self>) {
        let feed_mode = self.model.borrow().feed_mode;
        match fetch_json::<Vec<Status>>(feed_mode.as_path()).await {
            Ok(statuses) => {
                let mut model = self.model.borrow_mut();
                model.statuses = statuses;
                model.loading = false;
            }
            Err(error) => {
                let mut model = self.model.borrow_mut();
                model.loading = false;
                model.flash = Some(error);
            }
        }
        self.render();
    }

    async fn create_status(self: Rc<Self>, raw_input: String) {
        let status = raw_input.trim().to_string();
        if status.is_empty() {
            self.set_flash("Post text is required.");
            return;
        }

        match Request::post("/api/v1/statuses")
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({
                    "status": status,
                    "visibility": "public"
                })
                .to_string(),
            ) {
            Ok(request) => match request.send().await {
                Ok(response) if response.ok() => {
                    self.set_flash("Post published.");
                    self.clear_textarea("composer-input");
                    self.load_dashboard().await;
                }
                Ok(response) => {
                    self.set_flash_from_response(response, "Failed to publish post.")
                        .await;
                }
                Err(error) => self.set_flash(&format!("Failed to publish post: {error}")),
            },
            Err(error) => self.set_flash(&format!("Failed to publish post: {error:?}")),
        }
    }

    async fn trigger_backup(self: Rc<Self>) {
        match Request::post("/admin/backup").send().await {
            Ok(response) if response.ok() => {
                self.set_flash("Backup triggered.");
                self.refresh_admin().await;
            }
            Ok(response) => {
                self.set_flash_from_response(response, "Failed to trigger backup.")
                    .await;
            }
            Err(error) => self.set_flash(&format!("Failed to trigger backup: {error}")),
        }
    }

    async fn block_domain(self: Rc<Self>, raw_domain: String) {
        let domain = raw_domain.trim().to_string();
        if domain.is_empty() {
            self.set_flash("Domain is required.");
            return;
        }

        match Request::post("/admin/domain_blocks")
            .header("Content-Type", "application/json")
            .body(serde_json::json!({ "domain": domain }).to_string())
        {
            Ok(request) => match request.send().await {
                Ok(response) if response.ok() => {
                    self.set_flash("Domain block added.");
                    self.clear_input("domain-block-input");
                    self.refresh_admin().await;
                }
                Ok(response) => {
                    self.set_flash_from_response(response, "Failed to add domain block.")
                        .await;
                }
                Err(error) => self.set_flash(&format!("Failed to add domain block: {error}")),
            },
            Err(error) => self.set_flash(&format!("Failed to add domain block: {error:?}")),
        }
    }

    async fn unblock_domain(self: Rc<Self>, domain: String) {
        match Request::delete(&format!("/admin/domain_blocks/{domain}"))
            .send()
            .await
        {
            Ok(response) if response.ok() => {
                self.set_flash("Domain block removed.");
                self.refresh_admin().await;
            }
            Ok(response) => {
                self.set_flash_from_response(response, "Failed to remove domain block.")
                    .await;
            }
            Err(error) => self.set_flash(&format!("Failed to remove domain block: {error}")),
        }
    }

    async fn refresh_admin(self: Rc<Self>) {
        let (backups, domain_blocks, notifications) = futures_join3(
            fetch_json::<Vec<BackupInfo>>("/admin/backups"),
            fetch_json::<Vec<String>>("/admin/domain_blocks"),
            fetch_json::<Vec<Notification>>("/api/v1/notifications?limit=8"),
        )
        .await;

        let mut model = self.model.borrow_mut();
        model.backups = backups.unwrap_or_default();
        model.domain_blocks = domain_blocks.unwrap_or_default();
        model.notifications = notifications.unwrap_or_default();
        drop(model);
        self.render();
    }

    async fn logout(self: Rc<Self>) {
        match Request::post("/logout").send().await {
            Ok(_) => {
                let _ = self.window.location().set_href("/login");
            }
            Err(error) => self.set_flash(&format!("Failed to log out: {error}")),
        }
    }

    async fn set_flash_from_response(
        self: &Rc<Self>,
        response: gloo_net::http::Response,
        fallback: &str,
    ) {
        match response.text().await {
            Ok(text) if !text.trim().is_empty() => self.set_flash(&text),
            _ => self.set_flash(fallback),
        }
    }

    fn set_flash(self: &Rc<Self>, message: &str) {
        self.model.borrow_mut().flash = Some(message.to_string());
        self.render();
    }

    fn clear_input(&self, id: &str) {
        let Some(element) = self.document.get_element_by_id(id) else {
            return;
        };
        let Ok(input) = element.dyn_into::<HtmlInputElement>() else {
            return;
        };
        input.set_value("");
    }

    fn clear_textarea(&self, id: &str) {
        let Some(element) = self.document.get_element_by_id(id) else {
            return;
        };
        let Ok(input) = element.dyn_into::<HtmlTextAreaElement>() else {
            return;
        };
        input.set_value("");
    }
}

async fn fetch_json<T>(url: &str) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let response = Request::get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        let body = response.text().await.unwrap_or_default();
        return Err(if body.trim().is_empty() {
            format!("HTTP {}", response.status())
        } else {
            body
        });
    }
    response
        .json::<T>()
        .await
        .map_err(|error| error.to_string())
}

async fn futures_join5<A, B, C, D, E>(
    a: impl std::future::Future<Output = A>,
    b: impl std::future::Future<Output = B>,
    c: impl std::future::Future<Output = C>,
    d: impl std::future::Future<Output = D>,
    e: impl std::future::Future<Output = E>,
) -> (A, B, C, D, E) {
    futures::join!(a, b, c, d, e)
}

async fn futures_join3<A, B, C>(
    a: impl std::future::Future<Output = A>,
    b: impl std::future::Future<Output = B>,
    c: impl std::future::Future<Output = C>,
) -> (A, B, C) {
    futures::join!(a, b, c)
}

fn render_app(model: &Model) -> String {
    let brand = model
        .account
        .as_ref()
        .map(|account| encode_text(&display_name(account)).into_owned())
        .unwrap_or_else(|| APP_TITLE.to_string());

    format!(
        r#"
<div class="x-shell">
  <aside class="left-rail">
    <div class="brand-mark">rr</div>
    <div class="brand-block">
      <p class="brand-kicker">Unified UI</p>
      <h1>{brand}</h1>
      <p class="brand-copy">Rust/WASM dashboard with an X-like three-column frame.</p>
    </div>
    <nav class="nav-stack">
      <button id="nav-home" class="nav-pill {home_active}">Home timeline</button>
      <button id="nav-public" class="nav-pill {public_active}">Local timeline</button>
      <a class="nav-link" href="/settings">Legacy settings</a>
      <a class="nav-link" href="/api/v1/accounts/verify_credentials">Raw account JSON</a>
      <button id="logout-action" class="nav-link destructive">Log out</button>
    </nav>
    {account_card}
  </aside>
  <main class="center-column">
    <header class="timeline-head">
      <div>
        <p class="eyebrow">Feed</p>
        <h2>{feed_label}</h2>
      </div>
      <button id="refresh-feed" class="ghost-button">Refresh</button>
    </header>
    <section class="composer-card">
      <div class="composer-avatar">{composer_avatar}</div>
      <div class="composer-body">
        <textarea id="composer-input" placeholder="What's happening in RustResort?"></textarea>
        <div class="composer-actions">
          <span class="muted-copy">Posts are published through the existing Mastodon-compatible API.</span>
          <button id="composer-submit" class="primary-button">Post</button>
        </div>
      </div>
    </section>
    {flash}
    <section class="timeline-list">
      {timeline}
    </section>
  </main>
  <aside class="right-rail">
    <section class="rail-card">
      <div class="rail-card-head">
        <div>
          <p class="eyebrow">Notifications</p>
          <h3>Recent signals</h3>
        </div>
      </div>
      <div class="rail-list">
        {notifications}
      </div>
    </section>
    <section class="rail-card">
      <div class="rail-card-head">
        <div>
          <p class="eyebrow">Admin</p>
          <h3>Operations</h3>
        </div>
        <button id="backup-action" class="ghost-button">Run backup</button>
      </div>
      <label class="field">
        <span>Block domain</span>
        <div class="inline-form">
          <input id="domain-block-input" type="text" placeholder="bad.example" />
          <button id="domain-block-action" class="primary-button">Block</button>
        </div>
      </label>
      <div class="admin-split">
        <div>
          <h4>Backups</h4>
          <div class="rail-list">{backups}</div>
        </div>
        <div>
          <h4>Domain blocks</h4>
          <div class="rail-list">{domain_blocks}</div>
        </div>
      </div>
    </section>
  </aside>
</div>
"#,
        home_active = if model.feed_mode == FeedMode::Home {
            "active"
        } else {
            ""
        },
        public_active = if model.feed_mode == FeedMode::Public {
            "active"
        } else {
            ""
        },
        account_card = render_account_card(model),
        feed_label = model.feed_mode.label(),
        composer_avatar = render_avatar_monogram(model),
        flash = render_flash(model),
        timeline = render_timeline(model),
        notifications = render_notifications(model),
        backups = render_backups(model),
        domain_blocks = render_domain_blocks(model),
    )
}

fn render_account_card(model: &Model) -> String {
    let Some(account) = model.account.as_ref() else {
        return r#"<section class="profile-card"><p class="muted-copy">Loading account…</p></section>"#
            .to_string();
    };

    let note = account
        .source
        .as_ref()
        .map(|source| source.note.trim())
        .filter(|note| !note.is_empty())
        .map(|note| encode_text(note).into_owned())
        .unwrap_or_else(|| "Single-user ActivityPub control surface.".to_string());

    format!(
        r#"<section class="profile-card">
  <img class="profile-avatar" src="{avatar}" alt="{name}" />
  <div>
    <h3>{name}</h3>
    <p class="profile-handle">@{acct}</p>
  </div>
  <p class="muted-copy">{note}</p>
  <div class="stats-row">
    <span><strong>{statuses}</strong><small>posts</small></span>
    <span><strong>{followers}</strong><small>followers</small></span>
    <span><strong>{following}</strong><small>following</small></span>
  </div>
  <p class="session-chip">{session}</p>
</section>"#,
        avatar = encode_attribute(&account.avatar),
        name = encode_text(&display_name(account)),
        acct = encode_text(&account.acct),
        note = note,
        statuses = account.statuses_count,
        followers = account.followers_count,
        following = account.following_count,
        session = model
            .session
            .as_ref()
            .map(|session| format!(
                "{} via {}",
                encode_text(&session.username),
                encode_text(&session.auth_method)
            ))
            .unwrap_or_else(|| "Authenticating…".to_string()),
    )
}

fn render_timeline(model: &Model) -> String {
    if model.loading && model.statuses.is_empty() {
        return r#"<article class="timeline-card"><p class="muted-copy">Loading timeline…</p></article>"#
            .to_string();
    }

    if model.statuses.is_empty() {
        return r#"<article class="timeline-card"><p class="muted-copy">No posts available yet.</p></article>"#
            .to_string();
    }

    model
        .statuses
        .iter()
        .map(|status| {
            let content = if status.content.trim().is_empty() {
                "<p class=\"muted-copy\">No text content</p>".to_string()
            } else {
                status.content.clone()
            };
            let status_url = status
                .url
                .as_deref()
                .map(encode_attribute)
                .unwrap_or_else(|| "#".to_string());
            format!(
                r#"<article class="timeline-card">
  <div class="status-head">
    <img class="status-avatar" src="{avatar}" alt="{display_name}" />
    <div>
      <div class="status-meta">
        <strong>{display_name}</strong>
        <span>@{acct}</span>
        <span>{created_at}</span>
      </div>
      <div class="status-visibility">{visibility}</div>
    </div>
  </div>
  <div class="status-content">{content}</div>
  <div class="status-actions">
    <span>{replies} replies</span>
    <span>{reblogs} boosts</span>
    <span>{favourites} likes</span>
    <a href="{status_url}" target="_blank" rel="noreferrer">Open</a>
  </div>
</article>"#,
                avatar = encode_attribute(&status.account.avatar),
                display_name = encode_text(&status.account.display_name),
                acct = encode_text(&status.account.acct),
                created_at = encode_text(&short_timestamp(&status.created_at)),
                visibility = encode_text(&status.visibility),
                content = content,
                replies = status.replies_count,
                reblogs = status.reblogs_count,
                favourites = status.favourites_count,
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn render_notifications(model: &Model) -> String {
    if model.notifications.is_empty() {
        return r#"<div class="notification-card"><p class="muted-copy">No notifications.</p></div>"#
            .to_string();
    }

    model
        .notifications
        .iter()
        .map(|notification| {
            let preview = notification
                .status
                .as_ref()
                .map(|status| summarize_html(&status.content))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "Administrative or social event".to_string());

            format!(
                r#"<div class="notification-card">
  <div class="notification-meta">
    <strong>{kind}</strong>
    <span>{created_at}</span>
  </div>
  <div class="notification-user">
    <img class="status-avatar tiny" src="{avatar}" alt="{display_name}" />
    <div>
      <strong>{display_name}</strong>
      <span>@{acct}</span>
    </div>
  </div>
  <p>{preview}</p>
</div>"#,
                kind = encode_text(&notification.notification_type),
                created_at = encode_text(&short_timestamp(&notification.created_at)),
                avatar = encode_attribute(&notification.account.avatar),
                display_name = encode_text(&notification.account.display_name),
                acct = encode_text(&notification.account.acct),
                preview = encode_text(&preview),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn render_backups(model: &Model) -> String {
    if model.backups.is_empty() {
        return r#"<div class="mini-row"><span class="muted-copy">No backups listed.</span></div>"#
            .to_string();
    }

    model
        .backups
        .iter()
        .take(5)
        .map(|backup| {
            format!(
                r#"<div class="mini-row">
  <span>{key}</span>
  <small>{size} bytes · {created_at}</small>
</div>"#,
                key = encode_text(&backup.key),
                size = backup.size,
                created_at = encode_text(&short_timestamp(&backup.created_at)),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn render_domain_blocks(model: &Model) -> String {
    if model.domain_blocks.is_empty() {
        return r#"<div class="mini-row"><span class="muted-copy">No blocked domains.</span></div>"#
            .to_string();
    }

    model
        .domain_blocks
        .iter()
        .map(|domain| {
            format!(
                r#"<div class="mini-row removable">
  <span>{domain}</span>
  <button class="ghost-button small" data-domain-remove="{encoded_domain}">Remove</button>
</div>"#,
                domain = encode_text(domain),
                encoded_domain = encode_attribute(domain),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn render_flash(model: &Model) -> String {
    model
        .flash
        .as_ref()
        .map(|flash| format!(r#"<div class="flash-banner">{}</div>"#, encode_text(flash)))
        .unwrap_or_default()
}

fn render_avatar_monogram(model: &Model) -> String {
    model
        .account
        .as_ref()
        .map(|account| {
            display_name(account)
                .chars()
                .next()
                .unwrap_or('R')
                .to_ascii_uppercase()
                .to_string()
        })
        .unwrap_or_else(|| "R".to_string())
}

fn display_name(account: &Account) -> String {
    let trimmed = account.display_name.trim();
    if trimmed.is_empty() {
        account.username.clone()
    } else {
        trimmed.to_string()
    }
}

fn short_timestamp(value: &str) -> String {
    value.replace('T', " ").replace("+00:00", " UTC")
}

fn summarize_html(value: &str) -> String {
    let mut plain = String::with_capacity(value.len());
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => plain.push(character),
            _ => {}
        }
    }
    let decoded = html_escape::decode_html_entities(plain.trim()).into_owned();
    let normalized = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated = normalized.chars().take(140).collect::<String>();
    if normalized.chars().count() > 140 {
        format!("{truncated}…")
    } else {
        normalized
    }
}

fn encode_attribute(value: &str) -> String {
    encode_text(value).replace('"', "&quot;")
}
