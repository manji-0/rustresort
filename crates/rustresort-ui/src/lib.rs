use std::{cell::RefCell, rc::Rc};

use gloo_net::http::{Request, RequestBuilder, Response};
use html_escape::encode_text;
use serde::Deserialize;
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    Document, Element, Event, HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement, Window,
};

const APP_TITLE: &str = "RustResort";
const DEFAULT_FEED_LIMIT: usize = 20;
const DEFAULT_MAX_CHARACTERS: usize = 500;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FeedMode {
    Home,
    Public,
    Profile,
}

impl Default for FeedMode {
    fn default() -> Self {
        Self::Home
    }
}

impl FeedMode {
    fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Public => "Local",
            Self::Profile => "Posts",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NotificationFilter {
    All,
    Mentions,
    Activity,
}

impl Default for NotificationFilter {
    fn default() -> Self {
        Self::All
    }
}

impl NotificationFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Mentions => "Mentions",
            Self::Activity => "Activity",
        }
    }

    fn query(self) -> &'static str {
        match self {
            Self::All => "/api/v1/notifications?limit=8",
            Self::Mentions => "/api/v1/notifications?limit=8&types[]=mention",
            Self::Activity => {
                "/api/v1/notifications?limit=8&types[]=favourite&types[]=reblog&types[]=follow&types[]=status"
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FlashTone {
    Success,
    Error,
}

#[derive(Clone)]
struct FlashMessage {
    tone: FlashTone,
    text: String,
}

#[derive(Clone)]
struct ComposerDraft {
    status: String,
    spoiler_text: String,
    visibility: String,
    language: String,
    in_reply_to_id: Option<String>,
    in_reply_to_label: Option<String>,
}

impl Default for ComposerDraft {
    fn default() -> Self {
        Self {
            status: String::new(),
            spoiler_text: String::new(),
            visibility: "public".to_string(),
            language: String::new(),
            in_reply_to_id: None,
            in_reply_to_label: None,
        }
    }
}

#[derive(Clone, Default)]
struct ThreadView {
    status_id: Option<String>,
    loading: bool,
    focus: Option<Status>,
    ancestors: Vec<Status>,
    descendants: Vec<Status>,
}

#[derive(Clone, Default)]
struct Model {
    dashboard_loading: bool,
    feed_loading: bool,
    feed_mode: FeedMode,
    notification_filter: NotificationFilter,
    session: Option<Session>,
    instance: Option<Instance>,
    account: Option<Account>,
    statuses: Vec<Status>,
    notifications: Vec<Notification>,
    notifications_unread: usize,
    thread: ThreadView,
    composer: ComposerDraft,
    backups: Vec<BackupInfo>,
    domain_blocks: Vec<String>,
    flash: Option<FlashMessage>,
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

#[derive(Clone, Default, Deserialize)]
struct AccountSource {
    #[serde(default)]
    note: String,
    #[serde(default)]
    privacy: String,
    language: Option<String>,
    follow_requests_count: Option<usize>,
}

#[derive(Clone, Default, Deserialize)]
struct Account {
    id: String,
    username: String,
    acct: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    avatar: String,
    #[serde(default)]
    header: String,
    followers_count: i64,
    following_count: i64,
    statuses_count: i64,
    source: Option<AccountSource>,
}

#[derive(Clone, Default, Deserialize)]
struct StatusAccount {
    #[serde(default)]
    display_name: String,
    acct: String,
    #[serde(default)]
    avatar: String,
}

#[derive(Clone, Default, Deserialize)]
struct MediaAttachment {
    #[serde(rename = "type", default)]
    media_type: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    preview_url: String,
    description: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
struct Status {
    id: String,
    created_at: String,
    in_reply_to_id: Option<String>,
    #[serde(default)]
    spoiler_text: String,
    #[serde(default)]
    visibility: String,
    language: Option<String>,
    #[serde(default)]
    uri: String,
    url: Option<String>,
    replies_count: i64,
    reblogs_count: i64,
    favourites_count: i64,
    #[serde(default)]
    content: String,
    #[serde(default)]
    reblog: Option<Box<Status>>,
    account: StatusAccount,
    #[serde(default)]
    media_attachments: Vec<MediaAttachment>,
    #[serde(default)]
    favourited: bool,
    #[serde(default)]
    reblogged: bool,
    #[serde(default)]
    bookmarked: bool,
    #[serde(default)]
    pinned: bool,
    edited_at: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
struct StatusContext {
    #[serde(default)]
    ancestors: Vec<Status>,
    #[serde(default)]
    descendants: Vec<Status>,
}

#[derive(Clone, Default, Deserialize)]
struct Notification {
    id: String,
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

#[derive(Clone, Default, Deserialize)]
struct UnreadCount {
    count: usize,
}

#[derive(Clone, Default, Deserialize)]
struct Instance {
    #[serde(default)]
    title: String,
    configuration: Option<InstanceConfiguration>,
}

#[derive(Clone, Default, Deserialize)]
struct InstanceConfiguration {
    statuses: Option<StatusesConfiguration>,
}

#[derive(Clone, Default, Deserialize)]
struct StatusesConfiguration {
    max_characters: i32,
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
            move || app.set_feed_mode(FeedMode::Home)
        });
        self.attach_click("nav-public", {
            let app = self.clone();
            move || app.set_feed_mode(FeedMode::Public)
        });
        self.attach_click("nav-profile", {
            let app = self.clone();
            move || app.set_feed_mode(FeedMode::Profile)
        });
        self.attach_click("refresh-feed", {
            let app = self.clone();
            move || {
                spawn_local({
                    let app = app.clone();
                    async move {
                        app.refresh_social(None).await;
                    }
                });
            }
        });
        self.attach_click("composer-submit", {
            let app = self.clone();
            move || {
                spawn_local({
                    let app = app.clone();
                    async move {
                        app.create_status().await;
                    }
                });
            }
        });
        self.attach_click("composer-cancel-reply", {
            let app = self.clone();
            move || {
                app.clear_reply_target();
            }
        });
        self.attach_click("logout-action", {
            let app = self.clone();
            move || {
                spawn_local({
                    let app = app.clone();
                    async move {
                        app.logout().await;
                    }
                });
            }
        });
        self.attach_click("backup-action", {
            let app = self.clone();
            move || {
                spawn_local({
                    let app = app.clone();
                    async move {
                        app.trigger_backup().await;
                    }
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
                    async move {
                        app.block_domain(domain).await;
                    }
                });
            }
        });
        self.attach_click("notifications-clear", {
            let app = self.clone();
            move || {
                spawn_local({
                    let app = app.clone();
                    async move {
                        app.clear_notifications().await;
                    }
                });
            }
        });
        self.attach_click("thread-close", {
            let app = self.clone();
            move || {
                app.clear_thread();
            }
        });

        self.attach_textarea_input("composer-input", {
            let app = self.clone();
            move |value| {
                app.model.borrow_mut().composer.status = value;
            }
        });
        self.attach_input_change("composer-spoiler", {
            let app = self.clone();
            move |value| {
                app.model.borrow_mut().composer.spoiler_text = value;
            }
        });
        self.attach_input_change("composer-language", {
            let app = self.clone();
            move |value| {
                app.model.borrow_mut().composer.language = value;
            }
        });
        self.attach_select_change("composer-visibility", {
            let app = self.clone();
            move |value| {
                app.model.borrow_mut().composer.visibility = value;
            }
        });

        self.attach_dynamic_action("[data-domain-remove]", move |app, element| {
            let Some(domain) = element.get_attribute("data-domain-remove") else {
                return;
            };
            spawn_local(async move {
                app.unblock_domain(domain).await;
            });
        });
        self.attach_dynamic_action("[data-select-status]", move |app, element| {
            let Some(status_id) = element.get_attribute("data-select-status") else {
                return;
            };
            spawn_local(async move {
                app.load_thread(status_id).await;
            });
        });
        self.attach_dynamic_action("[data-reply-status]", move |app, element| {
            let Some(status_id) = element.get_attribute("data-reply-status") else {
                return;
            };
            let label = element
                .get_attribute("data-reply-label")
                .unwrap_or_else(|| "selected post".to_string());
            app.set_reply_target(status_id, label);
        });
        self.attach_dynamic_action("[data-status-action]", move |app, element| {
            let Some(action) = element.get_attribute("data-status-action") else {
                return;
            };
            let Some(status_id) = element.get_attribute("data-status-id") else {
                return;
            };
            spawn_local(async move {
                app.execute_status_action(action, status_id).await;
            });
        });
        self.attach_dynamic_action("[data-dismiss-notification]", move |app, element| {
            let Some(notification_id) = element.get_attribute("data-dismiss-notification") else {
                return;
            };
            spawn_local(async move {
                app.dismiss_notification(notification_id).await;
            });
        });
        self.attach_dynamic_action("[data-notification-filter]", move |app, element| {
            let Some(filter) = element.get_attribute("data-notification-filter") else {
                return;
            };
            let filter = match filter.as_str() {
                "mentions" => NotificationFilter::Mentions,
                "activity" => NotificationFilter::Activity,
                _ => NotificationFilter::All,
            };
            app.set_notification_filter(filter);
        });
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

    fn attach_input_change<F>(self: &Rc<Self>, id: &str, callback: F)
    where
        F: Fn(String) + 'static,
    {
        let Some(element) = self.document.get_element_by_id(id) else {
            return;
        };
        let Ok(input) = element.dyn_into::<HtmlInputElement>() else {
            return;
        };
        let listener_input = input.clone();
        let closure = Closure::<dyn FnMut(_)>::wrap(Box::new(move |_event: Event| {
            callback(listener_input.value());
        }));
        let _ = input.add_event_listener_with_callback("input", closure.as_ref().unchecked_ref());
        closure.forget();
    }

    fn attach_textarea_input<F>(self: &Rc<Self>, id: &str, callback: F)
    where
        F: Fn(String) + 'static,
    {
        let Some(element) = self.document.get_element_by_id(id) else {
            return;
        };
        let Ok(input) = element.dyn_into::<HtmlTextAreaElement>() else {
            return;
        };
        let listener_input = input.clone();
        let closure = Closure::<dyn FnMut(_)>::wrap(Box::new(move |_event: Event| {
            callback(listener_input.value());
        }));
        let _ = input.add_event_listener_with_callback("input", closure.as_ref().unchecked_ref());
        closure.forget();
    }

    fn attach_select_change<F>(self: &Rc<Self>, id: &str, callback: F)
    where
        F: Fn(String) + 'static,
    {
        let Some(element) = self.document.get_element_by_id(id) else {
            return;
        };
        let Ok(select) = element.dyn_into::<HtmlSelectElement>() else {
            return;
        };
        let listener_select = select.clone();
        let closure = Closure::<dyn FnMut(_)>::wrap(Box::new(move |_event: Event| {
            callback(listener_select.value());
        }));
        let _ = select.add_event_listener_with_callback("change", closure.as_ref().unchecked_ref());
        closure.forget();
    }

    fn attach_dynamic_action<F>(self: &Rc<Self>, selector: &str, callback: F)
    where
        F: Fn(Rc<Self>, Element) + Clone + 'static,
    {
        let Ok(node_list) = self.document.query_selector_all(selector) else {
            return;
        };
        for index in 0..node_list.length() {
            let Some(node) = node_list.get(index) else {
                continue;
            };
            let Ok(element) = node.dyn_into::<Element>() else {
                continue;
            };
            let closure = Closure::<dyn FnMut(_)>::wrap(Box::new({
                let app = self.clone();
                let element = element.clone();
                let callback = callback.clone();
                move |_event: Event| callback(app.clone(), element.clone())
            }));
            let _ =
                element.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
            closure.forget();
        }
    }

    fn input_value(&self, id: &str) -> Option<String> {
        let element = self.document.get_element_by_id(id)?;
        let input: HtmlInputElement = element.dyn_into().ok()?;
        Some(input.value())
    }

    fn set_feed_mode(self: &Rc<Self>, feed_mode: FeedMode) {
        {
            let mut model = self.model.borrow_mut();
            model.feed_mode = feed_mode;
            model.feed_loading = true;
        }
        self.render();
        spawn_local({
            let app = self.clone();
            async move {
                app.refresh_social(None).await;
            }
        });
    }

    fn set_notification_filter(self: &Rc<Self>, filter: NotificationFilter) {
        {
            let mut model = self.model.borrow_mut();
            model.notification_filter = filter;
        }
        self.render();
        spawn_local({
            let app = self.clone();
            async move {
                app.refresh_notifications().await;
            }
        });
    }

    fn set_reply_target(self: &Rc<Self>, status_id: String, label: String) {
        {
            let mut model = self.model.borrow_mut();
            model.composer.in_reply_to_id = Some(status_id);
            model.composer.in_reply_to_label = Some(label);
        }
        self.render();
    }

    fn clear_reply_target(self: &Rc<Self>) {
        {
            let mut model = self.model.borrow_mut();
            model.composer.in_reply_to_id = None;
            model.composer.in_reply_to_label = None;
        }
        self.render();
    }

    fn clear_thread(self: &Rc<Self>) {
        {
            let mut model = self.model.borrow_mut();
            model.thread = ThreadView::default();
        }
        self.render();
    }

    async fn load_dashboard(self: Rc<Self>) {
        {
            let mut model = self.model.borrow_mut();
            model.dashboard_loading = true;
            model.feed_loading = true;
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

        let feed_url = self.feed_url();
        let notifications_url = self.notifications_url();
        let (instance, account, statuses, notifications, unread, backups, domain_blocks) = futures::join!(
            fetch_json::<Instance>("/api/v1/instance"),
            fetch_json::<Account>("/api/v1/accounts/verify_credentials"),
            fetch_json::<Vec<Status>>(&feed_url),
            fetch_json::<Vec<Notification>>(&notifications_url),
            fetch_json::<UnreadCount>("/api/v1/notifications/unread_count"),
            fetch_json::<Vec<BackupInfo>>("/admin/backups"),
            fetch_json::<Vec<String>>("/admin/domain_blocks"),
        );

        let mut model = self.model.borrow_mut();
        model.session = Some(session);
        model.instance = instance.ok();
        if let Ok(account) = account {
            apply_account_defaults(&mut model.composer, &account);
            model.account = Some(account);
        } else {
            model.flash = Some(FlashMessage {
                tone: FlashTone::Error,
                text: "Failed to load account details.".to_string(),
            });
        }
        if let Ok(statuses) = statuses {
            model.statuses = statuses;
        }
        if let Ok(notifications) = notifications {
            model.notifications = notifications;
        }
        if let Ok(unread) = unread {
            model.notifications_unread = unread.count;
        }
        model.backups = backups.unwrap_or_default();
        model.domain_blocks = domain_blocks.unwrap_or_default();
        model.dashboard_loading = false;
        model.feed_loading = false;
        drop(model);
        self.render();
    }

    async fn refresh_social(self: Rc<Self>, focus_status: Option<String>) {
        {
            let mut model = self.model.borrow_mut();
            model.feed_loading = true;
        }
        self.render();

        let feed_url = self.feed_url();
        let notifications_url = self.notifications_url();
        let selected_thread = {
            let model = self.model.borrow();
            focus_status.or_else(|| model.thread.status_id.clone())
        };

        let (account, statuses, notifications, unread) = futures::join!(
            fetch_json::<Account>("/api/v1/accounts/verify_credentials"),
            fetch_json::<Vec<Status>>(&feed_url),
            fetch_json::<Vec<Notification>>(&notifications_url),
            fetch_json::<UnreadCount>("/api/v1/notifications/unread_count"),
        );

        let mut flash_error = None;
        {
            let mut model = self.model.borrow_mut();
            model.feed_loading = false;
            match account {
                Ok(account) => {
                    apply_account_defaults(&mut model.composer, &account);
                    model.account = Some(account);
                }
                Err(error) => flash_error = Some(error),
            }
            match statuses {
                Ok(statuses) => model.statuses = statuses,
                Err(error) => flash_error = Some(error),
            }
            if let Ok(notifications) = notifications {
                model.notifications = notifications;
            }
            if let Ok(unread) = unread {
                model.notifications_unread = unread.count;
            }
            if let Some(error) = flash_error {
                model.flash = Some(FlashMessage {
                    tone: FlashTone::Error,
                    text: error,
                });
            }
        }
        self.render();

        if let Some(status_id) = selected_thread {
            self.load_thread(status_id).await;
        }
    }

    async fn refresh_notifications(self: Rc<Self>) {
        let notifications_url = self.notifications_url();
        let (notifications, unread) = futures::join!(
            fetch_json::<Vec<Notification>>(&notifications_url),
            fetch_json::<UnreadCount>("/api/v1/notifications/unread_count"),
        );

        let mut model = self.model.borrow_mut();
        match notifications {
            Ok(notifications) => model.notifications = notifications,
            Err(error) => {
                model.flash = Some(FlashMessage {
                    tone: FlashTone::Error,
                    text: error,
                })
            }
        }
        if let Ok(unread) = unread {
            model.notifications_unread = unread.count;
        }
        drop(model);
        self.render();
    }

    async fn refresh_admin(self: Rc<Self>) {
        let (backups, domain_blocks) = futures::join!(
            fetch_json::<Vec<BackupInfo>>("/admin/backups"),
            fetch_json::<Vec<String>>("/admin/domain_blocks"),
        );

        let mut model = self.model.borrow_mut();
        model.backups = backups.unwrap_or_default();
        model.domain_blocks = domain_blocks.unwrap_or_default();
        drop(model);
        self.render();
    }

    async fn load_thread(self: Rc<Self>, status_id: String) {
        let initial_focus = self
            .find_status(&status_id)
            .or_else(|| self.find_notification_status(&status_id));
        {
            let mut model = self.model.borrow_mut();
            model.thread.status_id = Some(status_id.clone());
            model.thread.loading = true;
            if model.thread.focus.is_none() {
                model.thread.focus = initial_focus;
            }
        }
        self.render();

        let detail_url = status_endpoint(&status_id, None);
        let context_url = status_endpoint(&status_id, Some("context"));
        let (status, context) = futures::join!(
            fetch_json::<Status>(&detail_url),
            fetch_json::<StatusContext>(&context_url),
        );

        let mut model = self.model.borrow_mut();
        model.thread.loading = false;
        match status {
            Ok(status) => {
                model.thread.focus = Some(status);
            }
            Err(error) => {
                model.flash = Some(FlashMessage {
                    tone: FlashTone::Error,
                    text: error,
                });
            }
        }
        match context {
            Ok(context) => {
                model.thread.ancestors = context.ancestors;
                model.thread.descendants = context.descendants;
            }
            Err(error) => {
                model.flash = Some(FlashMessage {
                    tone: FlashTone::Error,
                    text: error,
                });
            }
        }
        drop(model);
        self.render();
    }

    async fn create_status(self: Rc<Self>) {
        let composer = self.model.borrow().composer.clone();
        let status_text = composer.status.trim().to_string();
        if status_text.is_empty() {
            self.set_flash(FlashTone::Error, "Post text is required.");
            return;
        }

        let mut payload = serde_json::Map::new();
        payload.insert("status".to_string(), serde_json::Value::String(status_text));
        payload.insert(
            "visibility".to_string(),
            serde_json::Value::String(composer.visibility.clone()),
        );
        if !composer.spoiler_text.trim().is_empty() {
            payload.insert(
                "spoiler_text".to_string(),
                serde_json::Value::String(composer.spoiler_text.trim().to_string()),
            );
        }
        if !composer.language.trim().is_empty() {
            payload.insert(
                "language".to_string(),
                serde_json::Value::String(composer.language.trim().to_string()),
            );
        }
        if let Some(reply_to) = composer.in_reply_to_id.as_ref() {
            payload.insert(
                "in_reply_to_id".to_string(),
                serde_json::Value::String(reply_to.clone()),
            );
        }

        match send_json::<Status>(
            "POST",
            "/api/v1/statuses",
            Some(serde_json::Value::Object(payload).to_string()),
        )
        .await
        {
            Ok(status) => {
                {
                    let mut model = self.model.borrow_mut();
                    model.composer = ComposerDraft::default();
                    let account = model.account.clone();
                    if let Some(account) = account.as_ref() {
                        apply_account_defaults(&mut model.composer, account);
                    }
                    model.flash = Some(FlashMessage {
                        tone: FlashTone::Success,
                        text: "Post published through the Mastodon API.".to_string(),
                    });
                }
                self.render();
                self.refresh_social(Some(status.id)).await;
            }
            Err(error) => self.set_flash(FlashTone::Error, &error),
        }
    }

    async fn execute_status_action(self: Rc<Self>, action: String, status_id: String) {
        let (method, url, success_message) = match action.as_str() {
            "favourite" => (
                "POST",
                status_endpoint(&status_id, Some("favourite")),
                "Post liked.",
            ),
            "unfavourite" => (
                "POST",
                status_endpoint(&status_id, Some("unfavourite")),
                "Like removed.",
            ),
            "reblog" => (
                "POST",
                status_endpoint(&status_id, Some("reblog")),
                "Boost published.",
            ),
            "unreblog" => (
                "POST",
                status_endpoint(&status_id, Some("unreblog")),
                "Boost removed.",
            ),
            "bookmark" => (
                "POST",
                status_endpoint(&status_id, Some("bookmark")),
                "Post bookmarked.",
            ),
            "unbookmark" => (
                "POST",
                status_endpoint(&status_id, Some("unbookmark")),
                "Bookmark removed.",
            ),
            "pin" => (
                "POST",
                status_endpoint(&status_id, Some("pin")),
                "Post pinned.",
            ),
            "unpin" => (
                "POST",
                status_endpoint(&status_id, Some("unpin")),
                "Pinned post removed.",
            ),
            "delete" => ("DELETE", status_endpoint(&status_id, None), "Post deleted."),
            _ => return,
        };

        match send_request(method, &url, None).await {
            Ok(_) => {
                self.set_flash(FlashTone::Success, success_message);
                if action == "delete" {
                    {
                        let mut model = self.model.borrow_mut();
                        if model.thread.status_id.as_deref() == Some(status_id.as_str()) {
                            model.thread = ThreadView::default();
                        }
                    }
                    self.refresh_social(None).await;
                } else {
                    self.refresh_social(Some(status_id)).await;
                }
            }
            Err(error) => self.set_flash(FlashTone::Error, &error),
        }
    }

    async fn dismiss_notification(self: Rc<Self>, notification_id: String) {
        let url = format!(
            "/api/v1/notifications/{}/dismiss",
            encode_path_segment(&notification_id)
        );
        match send_request("POST", &url, None).await {
            Ok(_) => {
                self.set_flash(FlashTone::Success, "Notification dismissed.");
                self.refresh_notifications().await;
            }
            Err(error) => self.set_flash(FlashTone::Error, &error),
        }
    }

    async fn clear_notifications(self: Rc<Self>) {
        match send_request("POST", "/api/v1/notifications/clear", None).await {
            Ok(_) => {
                self.set_flash(FlashTone::Success, "Notifications cleared.");
                self.refresh_notifications().await;
            }
            Err(error) => self.set_flash(FlashTone::Error, &error),
        }
    }

    async fn trigger_backup(self: Rc<Self>) {
        match send_request("POST", "/admin/backup", None).await {
            Ok(_) => {
                self.set_flash(FlashTone::Success, "Backup triggered.");
                self.refresh_admin().await;
            }
            Err(error) => self.set_flash(FlashTone::Error, &error),
        }
    }

    async fn block_domain(self: Rc<Self>, raw_domain: String) {
        let domain = raw_domain.trim().to_string();
        if domain.is_empty() {
            self.set_flash(FlashTone::Error, "Domain is required.");
            return;
        }

        let payload = serde_json::json!({ "domain": domain }).to_string();
        match send_request("POST", "/admin/domain_blocks", Some(payload)).await {
            Ok(_) => {
                self.set_flash(FlashTone::Success, "Domain block added.");
                self.clear_input("domain-block-input");
                self.refresh_admin().await;
            }
            Err(error) => self.set_flash(FlashTone::Error, &error),
        }
    }

    async fn unblock_domain(self: Rc<Self>, domain: String) {
        let url = format!("/admin/domain_blocks/{}", encode_path_segment(&domain));
        match send_request("DELETE", &url, None).await {
            Ok(_) => {
                self.set_flash(FlashTone::Success, "Domain block removed.");
                self.refresh_admin().await;
            }
            Err(error) => self.set_flash(FlashTone::Error, &error),
        }
    }

    async fn logout(self: Rc<Self>) {
        match send_request("POST", "/logout", None).await {
            Ok(_) => {
                let _ = self.window.location().set_href("/login");
            }
            Err(error) => self.set_flash(FlashTone::Error, &error),
        }
    }

    fn set_flash(self: &Rc<Self>, tone: FlashTone, text: &str) {
        self.model.borrow_mut().flash = Some(FlashMessage {
            tone,
            text: text.to_string(),
        });
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

    fn feed_url(&self) -> String {
        let model = self.model.borrow();
        match model.feed_mode {
            FeedMode::Home => format!("/api/v1/timelines/home?limit={DEFAULT_FEED_LIMIT}"),
            FeedMode::Public => {
                format!("/api/v1/timelines/public?local=true&limit={DEFAULT_FEED_LIMIT}")
            }
            FeedMode::Profile => model
                .account
                .as_ref()
                .map(|account| {
                    format!(
                        "/api/v1/accounts/{}/statuses?limit={DEFAULT_FEED_LIMIT}",
                        encode_path_segment(&account.id)
                    )
                })
                .unwrap_or_else(|| format!("/api/v1/timelines/home?limit={DEFAULT_FEED_LIMIT}")),
        }
    }

    fn notifications_url(&self) -> String {
        let model = self.model.borrow();
        model.notification_filter.query().to_string()
    }

    fn find_status(&self, id: &str) -> Option<Status> {
        self.model
            .borrow()
            .statuses
            .iter()
            .find_map(|status| status_with_id(status, id))
    }

    fn find_notification_status(&self, id: &str) -> Option<Status> {
        self.model
            .borrow()
            .notifications
            .iter()
            .filter_map(|notification| notification.status.as_ref())
            .find_map(|status| status_with_id(status, id))
    }
}

fn request_builder(method: &str, url: &str) -> RequestBuilder {
    match method {
        "GET" => Request::get(url),
        "POST" => Request::post(url),
        "PUT" => Request::put(url),
        "DELETE" => Request::delete(url),
        _ => Request::get(url),
    }
}

async fn send_request(method: &str, url: &str, body: Option<String>) -> Result<Response, String> {
    let builder = request_builder(method, url).header("Accept", "application/json");
    let builder = if body.is_some() {
        builder.header("Content-Type", "application/json")
    } else {
        builder
    };

    let response = match body {
        Some(body) => builder
            .body(body)
            .map_err(|error| format!("{error:?}"))?
            .send()
            .await
            .map_err(|error| error.to_string())?,
        None => builder.send().await.map_err(|error| error.to_string())?,
    };

    if response.ok() {
        Ok(response)
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(if body.trim().is_empty() {
            format!("HTTP {status}")
        } else {
            body
        })
    }
}

async fn fetch_json<T>(url: &str) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let response = send_request("GET", url, None).await?;
    response
        .json::<T>()
        .await
        .map_err(|error| error.to_string())
}

async fn send_json<T>(method: &str, url: &str, body: Option<String>) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let response = send_request(method, url, body).await?;
    response
        .json::<T>()
        .await
        .map_err(|error| error.to_string())
}

fn render_app(model: &Model) -> String {
    let brand_title = model
        .instance
        .as_ref()
        .map(|instance| instance.title.trim())
        .filter(|title| !title.is_empty())
        .unwrap_or(APP_TITLE);

    format!(
        r#"
<div class="app-shell">
  <aside class="sidebar">
    <div class="brand-lockup">
      <div class="brand-orb">rr</div>
      <div class="brand-copy">
        <p class="micro-label">Rust/WASM</p>
        <h1>{brand_title}</h1>
        <p class="lede">Integrated client tuned to Mastodon-compatible APIs first, with RustResort admin controls layered beside it.</p>
      </div>
    </div>
    <nav class="sidebar-nav">
      <button id="nav-home" class="sidebar-link {home_active}">Home timeline</button>
      <button id="nav-public" class="sidebar-link {public_active}">Local timeline</button>
      <button id="nav-profile" class="sidebar-link {profile_active}">My posts</button>
      <a class="sidebar-link" href="/settings">Legacy settings</a>
      <a class="sidebar-link" href="/api/v1/accounts/verify_credentials">Raw Mastodon JSON</a>
      <button id="logout-action" class="sidebar-link danger">Log out</button>
    </nav>
    {profile_panel}
    <section class="sidebar-note">
      <p class="micro-label">Contract</p>
      <p>Social UI reads and writes only through Mastodon-style endpoints under <code>/api/v1</code>. RustResort-specific admin routes stay isolated in the operations rail.</p>
    </section>
  </aside>

  <main class="timeline-column">
    <header class="timeline-header">
      <div>
        <p class="micro-label">Timeline</p>
        <h2>{feed_label}</h2>
        <p class="subtle-line">{feed_subtitle}</p>
      </div>
      <button id="refresh-feed" class="ghost-button">Refresh</button>
    </header>

    <section class="composer-panel">
      <div class="composer-avatar">{composer_avatar}</div>
      <div class="composer-stack">
        {reply_banner}
        <textarea id="composer-input" placeholder="What do you want to post?">{composer_text}</textarea>
        <div class="composer-grid">
          <label class="field">
            <span>Visibility</span>
            <select id="composer-visibility">
              {visibility_options}
            </select>
          </label>
          <label class="field">
            <span>Content warning</span>
            <input id="composer-spoiler" type="text" value="{spoiler_text}" placeholder="Optional spoiler or summary" />
          </label>
          <label class="field">
            <span>Language</span>
            <input id="composer-language" type="text" value="{language}" placeholder="en, ja, ..." />
          </label>
        </div>
        <div class="composer-footer">
          <span class="muted-copy">Character budget: {composer_count}/{max_characters}</span>
          <button id="composer-submit" class="primary-button">Post</button>
        </div>
      </div>
    </section>

    {flash_banner}
    {thread_panel}

    <section class="timeline-list">
      {timeline_cards}
    </section>
  </main>

  <aside class="activity-column">
    <section class="rail-card">
      <div class="rail-card-head">
        <div>
          <p class="micro-label">Notifications</p>
          <h3>Signals</h3>
        </div>
        <div class="notification-count">{notifications_unread} unread</div>
      </div>
      <div class="filter-row">
        {notification_filters}
      </div>
      <div class="rail-list">
        {notifications}
      </div>
      <div class="rail-actions">
        <button id="notifications-clear" class="ghost-button">Clear all</button>
      </div>
    </section>

    <section class="rail-card">
      <div class="rail-card-head">
        <div>
          <p class="micro-label">Operations</p>
          <h3>Admin</h3>
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
      <div class="admin-grid">
        <div>
          <h4>Recent backups</h4>
          <div class="rail-list">{backups}</div>
        </div>
        <div>
          <h4>Blocked domains</h4>
          <div class="rail-list">{domain_blocks}</div>
        </div>
      </div>
      <div class="rail-links">
        <a href="/settings">Open legacy admin/settings</a>
      </div>
    </section>
  </aside>
</div>
"#,
        brand_title = encode_text(brand_title),
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
        profile_active = if model.feed_mode == FeedMode::Profile {
            "active"
        } else {
            ""
        },
        profile_panel = render_profile_panel(model),
        feed_label = encode_text(model.feed_mode.label()),
        feed_subtitle = encode_text(&feed_subtitle(model)),
        composer_avatar = render_avatar_monogram(model),
        reply_banner = render_reply_banner(model),
        composer_text = encode_text(&model.composer.status),
        visibility_options = render_visibility_options(&model.composer.visibility),
        spoiler_text = encode_attribute(&model.composer.spoiler_text),
        language = encode_attribute(&model.composer.language),
        composer_count = model.composer.status.chars().count(),
        max_characters = composer_limit(model),
        flash_banner = render_flash(model),
        thread_panel = render_thread_panel(model),
        timeline_cards = render_timeline(model),
        notifications_unread = model.notifications_unread,
        notification_filters = render_notification_filters(model),
        notifications = render_notifications(model),
        backups = render_backups(model),
        domain_blocks = render_domain_blocks(model),
    )
}

fn render_profile_panel(model: &Model) -> String {
    let Some(account) = model.account.as_ref() else {
        return r#"<section class="profile-panel"><p class="muted-copy">Loading account…</p></section>"#
            .to_string();
    };

    let note = account
        .source
        .as_ref()
        .map(|source| source.note.trim())
        .filter(|note| !note.is_empty())
        .unwrap_or("Single-user ActivityPub node.");

    let follow_requests = account
        .source
        .as_ref()
        .and_then(|source| source.follow_requests_count)
        .unwrap_or(0);

    let session_label = model
        .session
        .as_ref()
        .map(|session| format!("{} via {}", session.username, session.auth_method))
        .unwrap_or_else(|| "session loading".to_string());

    format!(
        r#"<section class="profile-panel">
  <div class="profile-hero">
    {header}
  </div>
  <div class="profile-body">
    {avatar}
    <div class="profile-text">
      <h3>{name}</h3>
      <p class="handle">@{acct}</p>
    </div>
    <p class="bio">{note}</p>
    <div class="stat-grid">
      <div><strong>{statuses}</strong><span>posts</span></div>
      <div><strong>{followers}</strong><span>followers</span></div>
      <div><strong>{following}</strong><span>following</span></div>
    </div>
    <div class="chip-row">
      <span class="chip">{session}</span>
      <span class="chip">{follow_requests} follow requests</span>
    </div>
  </div>
</section>"#,
        header = if account.header.trim().is_empty() {
            "<div class=\"profile-hero-fallback\"></div>".to_string()
        } else {
            format!(
                "<img src=\"{}\" alt=\"{} header\" />",
                encode_attribute(&account.header),
                encode_attribute(&display_name(account))
            )
        },
        avatar = avatar_markup("profile-avatar", &account.avatar, &display_name(account)),
        name = encode_text(&display_name(account)),
        acct = encode_text(&account.acct),
        note = encode_text(note),
        statuses = account.statuses_count,
        followers = account.followers_count,
        following = account.following_count,
        session = encode_text(&session_label),
    )
}

fn render_reply_banner(model: &Model) -> String {
    let Some(label) = model.composer.in_reply_to_label.as_ref() else {
        return String::new();
    };

    format!(
        r#"<div class="reply-banner">
  <span>Replying to {label}</span>
  <button id="composer-cancel-reply" class="ghost-button small">Cancel</button>
</div>"#,
        label = encode_text(label),
    )
}

fn render_visibility_options(selected: &str) -> String {
    ["public", "unlisted", "private", "direct"]
        .into_iter()
        .map(|value| {
            format!(
                r#"<option value="{value}" {selected_attr}>{label}</option>"#,
                value = value,
                selected_attr = if selected == value { "selected" } else { "" },
                label = encode_text(value),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn render_flash(model: &Model) -> String {
    let Some(flash) = model.flash.as_ref() else {
        return String::new();
    };

    let tone = match flash.tone {
        FlashTone::Success => "success",
        FlashTone::Error => "error",
    };

    format!(
        r#"<div class="flash-banner {tone}">{text}</div>"#,
        tone = tone,
        text = encode_text(&flash.text),
    )
}

fn render_thread_panel(model: &Model) -> String {
    if model.thread.status_id.is_none() && !model.thread.loading {
        return String::new();
    }

    let content = if model.thread.loading && model.thread.focus.is_none() {
        r#"<div class="thread-empty">Loading conversation…</div>"#.to_string()
    } else if let Some(focus) = model.thread.focus.as_ref() {
        let ancestors = if model.thread.ancestors.is_empty() {
            String::new()
        } else {
            format!(
                r#"<div class="thread-stack">{}</div>"#,
                model
                    .thread
                    .ancestors
                    .iter()
                    .map(|status| render_status_card(status, model, true))
                    .collect::<Vec<_>>()
                    .join("")
            )
        };
        let descendants = if model.thread.descendants.is_empty() {
            String::new()
        } else {
            format!(
                r#"<div class="thread-stack">{}</div>"#,
                model
                    .thread
                    .descendants
                    .iter()
                    .map(|status| render_status_card(status, model, true))
                    .collect::<Vec<_>>()
                    .join("")
            )
        };

        format!(
            r#"{ancestors}
<div class="thread-focus">
  {focus}
</div>
{descendants}"#,
            ancestors = ancestors,
            focus = render_status_card(focus, model, false),
            descendants = descendants,
        )
    } else {
        r#"<div class="thread-empty">Conversation unavailable.</div>"#.to_string()
    };

    format!(
        r#"<section class="thread-panel">
  <div class="thread-head">
    <div>
      <p class="micro-label">Conversation</p>
      <h3>Selected thread</h3>
    </div>
    <button id="thread-close" class="ghost-button small">Close</button>
  </div>
  {content}
</section>"#,
        content = content,
    )
}

fn render_timeline(model: &Model) -> String {
    if model.dashboard_loading && model.statuses.is_empty() {
        return r#"<article class="status-card empty"><p>Loading timeline…</p></article>"#
            .to_string();
    }

    if model.statuses.is_empty() {
        return r#"<article class="status-card empty"><p>No posts available in this feed yet.</p></article>"#
            .to_string();
    }

    model
        .statuses
        .iter()
        .map(|status| render_status_card(status, model, false))
        .collect::<Vec<_>>()
        .join("")
}

fn render_status_card(status: &Status, model: &Model, compact: bool) -> String {
    let primary = display_status(status);
    let boost_banner = status.reblog.as_ref().map(|_| {
        format!(
            r#"<div class="boost-banner">Boosted by @{acct}</div>"#,
            acct = encode_text(&status.account.acct),
        )
    });

    let own_status = model
        .account
        .as_ref()
        .map(|account| primary.account.acct == account.acct)
        .unwrap_or(false)
        && status.reblog.is_none();
    let reply_label = format!(
        "@{} · {}",
        primary.account.acct,
        summarize_html(&primary.content)
    );

    let media = render_media_attachments(&primary.media_attachments);
    let spoiler = if primary.spoiler_text.trim().is_empty() {
        String::new()
    } else {
        format!(
            r#"<div class="spoiler-box">{}</div>"#,
            encode_text(primary.spoiler_text.trim())
        )
    };
    let edited = primary
        .edited_at
        .as_deref()
        .map(|value| {
            format!(
                r#"<span class="edited-chip">edited {}</span>"#,
                encode_text(&short_timestamp(value))
            )
        })
        .unwrap_or_default();
    let language_chip = primary
        .language
        .as_deref()
        .filter(|language| !language.trim().is_empty())
        .map(|language| {
            format!(
                r#"<span class="status-chip subtle">{}</span>"#,
                encode_text(language)
            )
        })
        .unwrap_or_default();

    let open_url = permalink(primary);
    let action_target = encode_attribute(&primary.id);
    let select_target = encode_attribute(&primary.id);

    format!(
        r#"<article class="status-card {compact}">
  {boost_banner}
  <div class="status-head">
    {avatar}
    <div class="status-meta">
      <div class="status-line">
        <strong>{display_name}</strong>
        <span>@{acct}</span>
        <span>{created_at}</span>
        {edited}
      </div>
      <div class="status-badges">
        <span class="status-chip">{visibility}</span>
        {replying}
        {language_chip}
      </div>
    </div>
  </div>
  <button class="status-thread-link" data-select-status="{select_target}">Open thread</button>
  {spoiler}
  <div class="status-content">{content}</div>
  {media}
  <div class="status-actions">
    <button class="action-pill" data-reply-status="{action_target}" data-reply-label="{reply_label}">Reply {replies}</button>
    <button class="action-pill {favourite_active}" data-status-action="{favourite_action}" data-status-id="{action_target}">Like {favourites}</button>
    <button class="action-pill {reblog_active}" data-status-action="{reblog_action}" data-status-id="{action_target}">Boost {reblogs}</button>
    <button class="action-pill {bookmark_active}" data-status-action="{bookmark_action}" data-status-id="{action_target}">Save</button>
    {pin_button}
    {delete_button}
    <a class="action-link" href="{open_url}" target="_blank" rel="noreferrer">Open</a>
  </div>
</article>"#,
        compact = if compact { "compact" } else { "" },
        boost_banner = boost_banner.unwrap_or_default(),
        avatar = avatar_markup(
            "status-avatar",
            &primary.account.avatar,
            &display_name_from_status(&primary.account)
        ),
        display_name = encode_text(&display_name_from_status(&primary.account)),
        acct = encode_text(&primary.account.acct),
        created_at = encode_text(&short_timestamp(&primary.created_at)),
        edited = edited,
        visibility = encode_text(&primary.visibility),
        replying = primary
            .in_reply_to_id
            .as_ref()
            .map(|_| r#"<span class="status-chip subtle">reply</span>"#.to_string())
            .unwrap_or_default(),
        language_chip = language_chip,
        select_target = select_target,
        spoiler = spoiler,
        content = if primary.content.trim().is_empty() {
            "<p class=\"muted-copy\">No text content</p>".to_string()
        } else {
            primary.content.clone()
        },
        media = media,
        action_target = action_target,
        reply_label = encode_attribute(&reply_label),
        replies = primary.replies_count,
        favourite_active = if primary.favourited { "active" } else { "" },
        favourite_action = if primary.favourited {
            "unfavourite"
        } else {
            "favourite"
        },
        favourites = primary.favourites_count,
        reblog_active = if primary.reblogged { "active" } else { "" },
        reblog_action = if primary.reblogged {
            "unreblog"
        } else {
            "reblog"
        },
        reblogs = primary.reblogs_count,
        bookmark_active = if primary.bookmarked { "active" } else { "" },
        bookmark_action = if primary.bookmarked {
            "unbookmark"
        } else {
            "bookmark"
        },
        pin_button = render_pin_button(primary, own_status),
        delete_button = render_delete_button(primary, own_status),
        open_url = encode_attribute(&open_url),
    )
}

fn render_pin_button(status: &Status, own_status: bool) -> String {
    if !own_status {
        return String::new();
    }

    format!(
        r#"<button class="action-pill {active}" data-status-action="{action}" data-status-id="{status_id}">{label}</button>"#,
        active = if status.pinned { "active" } else { "" },
        action = if status.pinned { "unpin" } else { "pin" },
        status_id = encode_attribute(&status.id),
        label = if status.pinned { "Unpin" } else { "Pin" },
    )
}

fn render_delete_button(status: &Status, own_status: bool) -> String {
    if !own_status {
        return String::new();
    }

    format!(
        r#"<button class="action-pill danger" data-status-action="delete" data-status-id="{status_id}">Delete</button>"#,
        status_id = encode_attribute(&status.id),
    )
}

fn render_media_attachments(media: &[MediaAttachment]) -> String {
    if media.is_empty() {
        return String::new();
    }

    let items = media
        .iter()
        .filter_map(|attachment| {
            let preview = if attachment.preview_url.trim().is_empty() {
                &attachment.url
            } else {
                &attachment.preview_url
            };
            if preview.trim().is_empty() {
                return None;
            }
            Some(format!(
                r#"<figure class="media-tile">
  <img src="{preview}" alt="{alt}" />
  <figcaption>{kind}</figcaption>
</figure>"#,
                preview = encode_attribute(preview),
                alt = encode_attribute(
                    attachment
                        .description
                        .as_deref()
                        .unwrap_or("attached media preview"),
                ),
                kind = encode_text(if attachment.media_type.trim().is_empty() {
                    "media"
                } else {
                    &attachment.media_type
                }),
            ))
        })
        .collect::<Vec<_>>()
        .join("");

    if items.is_empty() {
        return String::new();
    }

    format!(r#"<div class="media-grid">{items}</div>"#)
}

fn render_notification_filters(model: &Model) -> String {
    [
        (NotificationFilter::All, "all"),
        (NotificationFilter::Mentions, "mentions"),
        (NotificationFilter::Activity, "activity"),
    ]
    .into_iter()
    .map(|(filter, value)| {
        format!(
            r#"<button class="filter-pill {active}" data-notification-filter="{value}">{label}</button>"#,
            active = if model.notification_filter == filter {
                "active"
            } else {
                ""
            },
            value = value,
            label = filter.label(),
        )
    })
    .collect::<Vec<_>>()
    .join("")
}

fn render_notifications(model: &Model) -> String {
    if model.notifications.is_empty() {
        return r#"<div class="notification-card empty"><p>No notifications for this filter.</p></div>"#
            .to_string();
    }

    model
        .notifications
        .iter()
        .map(|notification| {
            let preview = notification
                .status
                .as_ref()
                .map(|status| summarize_html(&display_status(status).content))
                .filter(|summary| !summary.is_empty())
                .unwrap_or_else(|| "Account-level event".to_string());
            let thread_button = notification
                .status
                .as_ref()
                .map(|status| {
                    format!(
                        r#"<button class="ghost-button small" data-select-status="{status_id}">Open thread</button>"#,
                        status_id = encode_attribute(&display_status(status).id),
                    )
                })
                .unwrap_or_default();

            format!(
                r#"<div class="notification-card">
  <div class="notification-head">
    <strong>{kind}</strong>
    <span>{created_at}</span>
  </div>
  <div class="notification-user">
    {avatar}
    <div>
      <strong>{display_name}</strong>
      <span>@{acct}</span>
    </div>
  </div>
  <p>{preview}</p>
  <div class="notification-actions">
    {thread_button}
    <button class="ghost-button small" data-dismiss-notification="{notification_id}">Dismiss</button>
  </div>
</div>"#,
                kind = encode_text(&notification.notification_type),
                created_at = encode_text(&short_timestamp(&notification.created_at)),
                avatar = avatar_markup(
                    "status-avatar small",
                    &notification.account.avatar,
                    &display_name_from_status(&notification.account)
                ),
                display_name = encode_text(&display_name_from_status(&notification.account)),
                acct = encode_text(&notification.account.acct),
                preview = encode_text(&preview),
                thread_button = thread_button,
                notification_id = encode_attribute(&notification.id),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn avatar_markup(class_name: &str, url: &str, label: &str) -> String {
    if !url.trim().is_empty() {
        return format!(
            r#"<img class="{class_name}" src="{src}" alt="{label}" />"#,
            class_name = class_name,
            src = encode_attribute(url),
            label = encode_attribute(label),
        );
    }

    let initial = label
        .trim()
        .chars()
        .next()
        .map(|ch| ch.to_ascii_uppercase())
        .filter(|ch| ch.is_ascii_alphanumeric())
        .unwrap_or('R');
    format!(
        r#"<div class="{class_name} avatar-fallback" aria-label="{label}">{initial}</div>"#,
        class_name = class_name,
        label = encode_attribute(label),
        initial = initial,
    )
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

fn feed_subtitle(model: &Model) -> String {
    if model.feed_loading {
        return "Refreshing feed and notifications…".to_string();
    }

    match model.feed_mode {
        FeedMode::Home => {
            "Followed accounts and recent local activity from the Mastodon home feed.".to_string()
        }
        FeedMode::Public => {
            "Local public timeline resolved through the Mastodon public timeline contract."
                .to_string()
        }
        FeedMode::Profile => {
            "Your account statuses from the account timeline endpoint.".to_string()
        }
    }
}

fn composer_limit(model: &Model) -> usize {
    model
        .instance
        .as_ref()
        .and_then(|instance| instance.configuration.as_ref())
        .and_then(|configuration| configuration.statuses.as_ref())
        .map(|statuses| statuses.max_characters.max(0) as usize)
        .filter(|limit| *limit > 0)
        .unwrap_or(DEFAULT_MAX_CHARACTERS)
}

fn apply_account_defaults(composer: &mut ComposerDraft, account: &Account) {
    let Some(source) = account.source.as_ref() else {
        return;
    };

    if composer.visibility == "public" && !source.privacy.trim().is_empty() {
        composer.visibility = source.privacy.trim().to_string();
    }
    if composer.language.trim().is_empty() {
        if let Some(language) = source.language.as_deref() {
            composer.language = language.trim().to_string();
        }
    }
}

fn status_with_id(status: &Status, id: &str) -> Option<Status> {
    if status.id == id {
        return Some(status.clone());
    }
    status
        .reblog
        .as_deref()
        .and_then(|reblog| status_with_id(reblog, id))
}

fn display_status(status: &Status) -> &Status {
    status.reblog.as_deref().unwrap_or(status)
}

fn display_name(account: &Account) -> String {
    let trimmed = account.display_name.trim();
    if trimmed.is_empty() {
        account.username.clone()
    } else {
        trimmed.to_string()
    }
}

fn display_name_from_status(account: &StatusAccount) -> String {
    let trimmed = account.display_name.trim();
    if trimmed.is_empty() {
        account.acct.clone()
    } else {
        trimmed.to_string()
    }
}

fn short_timestamp(value: &str) -> String {
    let normalized = value
        .replace('T', " ")
        .replace(".000Z", " UTC")
        .replace('Z', " UTC")
        .replace("+00:00", " UTC");
    normalized.chars().take(22).collect()
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
    let truncated = normalized.chars().take(120).collect::<String>();
    if normalized.chars().count() > 120 {
        format!("{truncated}…")
    } else {
        normalized
    }
}

fn permalink(status: &Status) -> String {
    status
        .url
        .clone()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| status.uri.clone())
}

fn status_endpoint(id: &str, suffix: Option<&str>) -> String {
    let encoded = encode_path_segment(id);
    match suffix {
        Some(suffix) => format!("/api/v1/statuses/{encoded}/{suffix}"),
        None => format!("/api/v1/statuses/{encoded}"),
    }
}

fn encode_path_segment(value: &str) -> String {
    js_sys::encode_uri_component(value)
        .as_string()
        .unwrap_or_else(|| value.to_string())
}

fn encode_attribute(value: &str) -> String {
    encode_text(value).replace('"', "&quot;")
}
