use std::{cell::RefCell, rc::Rc};

use gloo_net::http::{Request, RequestBuilder, Response};
use html_escape::encode_text;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    Document, Element, Event, HtmlElement, HtmlInputElement, HtmlSelectElement,
    HtmlTextAreaElement, KeyboardEvent, Storage, Window,
};

const APP_TITLE: &str = "RustResort";
const DEFAULT_FEED_LIMIT: usize = 20;
const DEFAULT_MAX_CHARACTERS: usize = 500;
const COMPOSER_DRAFT_STORAGE_KEY: &str = "rustresort-ui-composer-draft";

#[derive(Clone, Copy, PartialEq, Eq)]
enum FeedMode {
    Home,
    Public,
    Profile,
    Hashtags,
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
            Self::Hashtags => "Hashtags",
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
enum Pane {
    Timeline,
    Notifications,
}

impl Default for Pane {
    fn default() -> Self {
        Self::Timeline
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActivePane {
    Timeline,
    Notifications,
    DetailModal,
}

impl Default for ActivePane {
    fn default() -> Self {
        Self::Timeline
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

#[derive(Clone, Serialize, Deserialize)]
struct ComposerDraft {
    status: String,
    spoiler_text: String,
    visibility: String,
    language: String,
    in_reply_to_id: Option<String>,
    in_reply_to_label: Option<String>,
    quoted_status_id: Option<String>,
    quoted_status_label: Option<String>,
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
            quoted_status_id: None,
            quoted_status_label: None,
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
    composer_popout: bool,
    shortcut_help_open: bool,
    hashtag_query: String,
    active_pane: ActivePane,
    last_non_modal_pane: Pane,
    selected_status_id: Option<String>,
    selected_notification_key: Option<String>,
    detail_return_status_id: Option<String>,
    thread_history: Vec<String>,
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
    text: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    filtered: Vec<serde_json::Value>,
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
    #[serde(default)]
    group_key: String,
    created_at: String,
    account: StatusAccount,
    status: Option<Status>,
}

#[derive(Clone)]
struct NotificationGroup {
    group_key: String,
    ids: Vec<String>,
    notification_type: String,
    created_at: String,
    accounts: Vec<StatusAccount>,
    status: Option<Status>,
    count: usize,
}

#[derive(Clone, Deserialize)]
#[allow(dead_code)]
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

    let mut model = Model::default();
    if let Some(draft) = restore_composer_draft(&window) {
        model.composer = draft;
        model.flash = Some(FlashMessage {
            tone: FlashTone::Success,
            text: "Recovered unsent draft.".to_string(),
        });
    }

    let app = Rc::new(App {
        window,
        document,
        root,
        model: RefCell::new(model),
    });

    app.render();
    app.attach_keyboard_shortcuts();
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
        self.attach_click("nav-hashtags", {
            let app = self.clone();
            move || app.set_feed_mode(FeedMode::Hashtags)
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
        self.attach_click("composer-cancel-quote", {
            let app = self.clone();
            move || {
                app.clear_quote_target();
            }
        });
        self.attach_click("composer-toggle-popout", {
            let app = self.clone();
            move || {
                app.toggle_composer_popout();
            }
        });
        self.attach_click("composer-close-popout", {
            let app = self.clone();
            move || {
                app.toggle_composer_popout();
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
                app.close_detail_modal();
            }
        });

        self.attach_textarea_input("composer-input", {
            let app = self.clone();
            move |value| {
                app.update_composer(move |composer| composer.status = value);
            }
        });
        self.attach_input_change("composer-spoiler", {
            let app = self.clone();
            move |value| {
                app.update_composer(move |composer| composer.spoiler_text = value);
            }
        });
        self.attach_input_change("composer-language", {
            let app = self.clone();
            move |value| {
                app.update_composer(move |composer| composer.language = value);
            }
        });
        self.attach_input_change("hashtag-query", {
            let app = self.clone();
            move |value| {
                {
                    let mut model = app.model.borrow_mut();
                    model.hashtag_query = value;
                }
                app.render();
            }
        });
        self.attach_dynamic_action("[data-focus-hashtag-query]", move |app, _element| {
            app.focus_hashtag_query();
        });
        self.attach_select_change("composer-visibility", {
            let app = self.clone();
            move |value| {
                app.update_composer(move |composer| composer.visibility = value);
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
                app.open_detail_modal(status_id).await;
            });
        });
        self.attach_dynamic_action("[data-focus-status]", move |app, element| {
            let Some(status_id) = element.get_attribute("data-focus-status") else {
                return;
            };
            app.set_selected_status(status_id);
        });
        self.attach_dynamic_action("[data-focus-notification]", move |app, element| {
            let Some(group_key) = element.get_attribute("data-focus-notification") else {
                return;
            };
            app.set_selected_notification(group_key);
        });
        self.attach_dynamic_action("[data-jump-status]", move |app, element| {
            let Some(status_id) = element.get_attribute("data-jump-status") else {
                return;
            };
            app.jump_to_timeline_status(status_id);
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
            let Some(notification_ids) = element.get_attribute("data-dismiss-notification") else {
                return;
            };
            spawn_local(async move {
                let ids = notification_ids
                    .split('|')
                    .filter(|value| !value.trim().is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                app.dismiss_notifications(ids).await;
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
        self.attach_dynamic_action("[data-open-shortcuts]", move |app, _element| {
            app.open_shortcut_help();
        });
        self.attach_click("shortcut-help-close", {
            let app = self.clone();
            move || {
                app.close_shortcut_help();
            }
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

    fn attach_keyboard_shortcuts(self: &Rc<Self>) {
        let closure = Closure::<dyn FnMut(_)>::wrap(Box::new({
            let app = self.clone();
            move |event: KeyboardEvent| {
                if event.ctrl_key() || event.meta_key() || event.alt_key() {
                    return;
                }
                let raw_key = event.key();
                if raw_key == "Escape" {
                    if app.close_shortcut_help() {
                        event.prevent_default();
                        return;
                    }
                    app.close_detail_modal();
                    event.prevent_default();
                    return;
                }
                if let Some(target) = event.target()
                    && let Ok(element) = target.dyn_into::<Element>()
                {
                    let tag_name = element.tag_name().to_ascii_lowercase();
                    if matches!(tag_name.as_str(), "input" | "textarea" | "select") {
                        return;
                    }
                    if element.get_attribute("contenteditable").as_deref() == Some("true") {
                        return;
                    }
                }

                let key = raw_key.to_ascii_lowercase();
                let shift_tab = event.shift_key() && raw_key == "Tab";
                let handled = match raw_key.as_str() {
                    "N" => {
                        app.activate_mention_shortcut();
                        true
                    }
                    _ => match key.as_str() {
                        "tab" if shift_tab => {
                            app.cycle_pane_focus(-1);
                            true
                        }
                        "tab" => {
                            app.cycle_pane_focus(1);
                            true
                        }
                        "j" => {
                            app.move_selection(-1);
                            true
                        }
                        "k" => {
                            app.move_selection(1);
                            true
                        }
                        "g" => {
                            app.window.scroll_to_with_x_and_y(0.0, 0.0);
                            app.select_first_status();
                            true
                        }
                        "d" => {
                            app.open_selected_detail();
                            true
                        }
                        "n" => {
                            app.open_composer_popout();
                            true
                        }
                        "?" | "/" if event.shift_key() => {
                            app.open_shortcut_help();
                            true
                        }
                        "f" => {
                            if let Some(status_id) = app.shortcut_status_id() {
                                spawn_local({
                                    let app = app.clone();
                                    async move {
                                        app.execute_status_action(
                                            "favourite".to_string(),
                                            status_id,
                                        )
                                        .await;
                                    }
                                });
                            }
                            true
                        }
                        "r" => {
                            if let Some(status_id) = app.shortcut_status_id() {
                                spawn_local({
                                    let app = app.clone();
                                    async move {
                                        app.execute_status_action("reblog".to_string(), status_id)
                                            .await;
                                    }
                                });
                            }
                            true
                        }
                        "q" => {
                            app.activate_quote_shortcut();
                            true
                        }
                        _ => false,
                    },
                };
                if handled {
                    event.prevent_default();
                }
            }
        }));
        let _ = self
            .document
            .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
        closure.forget();
    }

    fn input_value(&self, id: &str) -> Option<String> {
        let input = self.input(id)?;
        Some(input.value())
    }

    fn input(&self, id: &str) -> Option<HtmlInputElement> {
        let element = self.document.get_element_by_id(id)?;
        element.dyn_into::<HtmlInputElement>().ok()
    }

    fn textarea(&self, id: &str) -> Option<HtmlTextAreaElement> {
        let element = self.document.get_element_by_id(id)?;
        element.dyn_into::<HtmlTextAreaElement>().ok()
    }

    fn set_feed_mode(self: &Rc<Self>, feed_mode: FeedMode) {
        {
            let mut model = self.model.borrow_mut();
            model.feed_mode = feed_mode;
            model.feed_loading = true;
            model.active_pane = ActivePane::Timeline;
            model.last_non_modal_pane = Pane::Timeline;
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
            model.active_pane = ActivePane::Notifications;
            model.last_non_modal_pane = Pane::Notifications;
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
            model.composer.quoted_status_id = None;
            model.composer.quoted_status_label = None;
        }
        self.save_composer_draft();
        self.render();
    }

    fn clear_reply_target(self: &Rc<Self>) {
        {
            let mut model = self.model.borrow_mut();
            model.composer.in_reply_to_id = None;
            model.composer.in_reply_to_label = None;
        }
        self.save_composer_draft();
        self.render();
    }

    fn set_quote_target(self: &Rc<Self>, status_id: String, label: String) {
        {
            let mut model = self.model.borrow_mut();
            model.composer.quoted_status_id = Some(status_id);
            model.composer.quoted_status_label = Some(label);
            model.composer.in_reply_to_id = None;
            model.composer.in_reply_to_label = None;
            model.composer_popout = true;
        }
        self.save_composer_draft();
        self.render();
        self.focus_composer_input();
    }

    fn clear_quote_target(self: &Rc<Self>) {
        {
            let mut model = self.model.borrow_mut();
            model.composer.quoted_status_id = None;
            model.composer.quoted_status_label = None;
        }
        self.save_composer_draft();
        self.render();
    }

    fn toggle_composer_popout(self: &Rc<Self>) {
        {
            let mut model = self.model.borrow_mut();
            model.composer_popout = !model.composer_popout;
        }
        self.render();
        self.focus_composer_input();
    }

    fn open_composer_popout(self: &Rc<Self>) {
        {
            let mut model = self.model.borrow_mut();
            model.composer_popout = true;
        }
        self.render();
        self.focus_composer_input();
    }

    fn open_shortcut_help(self: &Rc<Self>) {
        {
            let mut model = self.model.borrow_mut();
            model.shortcut_help_open = true;
        }
        self.render();
    }

    fn close_shortcut_help(self: &Rc<Self>) -> bool {
        let should_render = {
            let mut model = self.model.borrow_mut();
            if model.shortcut_help_open {
                model.shortcut_help_open = false;
                true
            } else {
                false
            }
        };
        if should_render {
            self.render();
        }
        should_render
    }

    fn focus_composer_input(&self) {
        if let Some(textarea) = self.textarea("composer-input") {
            let _ = textarea.focus();
        }
    }

    fn focus_hashtag_query(&self) {
        let Some(input) = self.input("hashtag-query") else {
            return;
        };
        let _ = input.focus();
        input.select();
    }

    fn blur_active_element(&self) {
        let Some(element) = self.document.active_element() else {
            return;
        };
        if let Ok(html_element) = element.dyn_into::<HtmlElement>() {
            let _ = html_element.blur();
        }
    }

    fn set_selected_status(self: &Rc<Self>, status_id: String) {
        self.blur_active_element();
        let should_render = {
            let mut model = self.model.borrow_mut();
            let mut pane_changed = false;
            if model.active_pane != ActivePane::DetailModal {
                pane_changed = model.active_pane != ActivePane::Timeline;
                model.active_pane = ActivePane::Timeline;
                model.last_non_modal_pane = Pane::Timeline;
            }
            let selection_changed =
                if model.selected_status_id.as_deref() == Some(status_id.as_str()) {
                    false
                } else {
                    model.selected_status_id = Some(status_id.clone());
                    true
                };
            pane_changed || selection_changed
        };
        if should_render {
            self.render();
        }
        self.sync_selected_element();
    }

    fn set_selected_notification(self: &Rc<Self>, group_key: String) {
        self.blur_active_element();
        let should_render = {
            let mut model = self.model.borrow_mut();
            let mut pane_changed = false;
            if model.active_pane != ActivePane::DetailModal {
                pane_changed = model.active_pane != ActivePane::Notifications;
                model.active_pane = ActivePane::Notifications;
                model.last_non_modal_pane = Pane::Notifications;
            }
            let selection_changed =
                if model.selected_notification_key.as_deref() == Some(group_key.as_str()) {
                    false
                } else {
                    model.selected_notification_key = Some(group_key.clone());
                    true
                };
            pane_changed || selection_changed
        };
        if should_render {
            self.render();
        }
        self.sync_selected_element();
    }

    fn move_selection(self: &Rc<Self>, delta: isize) {
        let active_pane = self.model.borrow().active_pane;
        match active_pane {
            ActivePane::Timeline => self.move_timeline_selection(delta),
            ActivePane::Notifications => self.move_notification_selection(delta),
            ActivePane::DetailModal => self.move_detail_selection(delta),
        }
    }

    fn move_timeline_selection(self: &Rc<Self>, delta: isize) {
        let order = timeline_selection_order(&self.model.borrow());
        if order.is_empty() {
            return;
        }

        let current_id = self.model.borrow().selected_status_id.clone();
        let current_index = current_id
            .as_deref()
            .and_then(|status_id| order.iter().position(|candidate| candidate == status_id))
            .unwrap_or(0);

        let target_index = next_selection_index(current_index, order.len(), delta);
        self.set_selected_status(order[target_index].clone());
    }

    fn move_notification_selection(self: &Rc<Self>, delta: isize) {
        let order = notification_selection_order(&self.model.borrow());
        if order.is_empty() {
            return;
        }

        let current_id = self.model.borrow().selected_notification_key.clone();
        let current_index = current_id
            .as_deref()
            .and_then(|status_id| order.iter().position(|candidate| candidate == status_id))
            .unwrap_or(0);

        let target_index = next_selection_index(current_index, order.len(), delta);
        self.set_selected_notification(order[target_index].clone());
    }

    fn move_detail_selection(self: &Rc<Self>, delta: isize) {
        let order = detail_selection_order(&self.model.borrow());
        if order.is_empty() {
            return;
        }

        let current_id = self.model.borrow().selected_status_id.clone();
        let current_index = current_id
            .as_deref()
            .and_then(|status_id| order.iter().position(|candidate| candidate == status_id))
            .unwrap_or(0);

        let target_index = next_selection_index(current_index, order.len(), delta);
        self.set_selected_status(order[target_index].clone());
    }

    fn jump_to_timeline_status(self: &Rc<Self>, status_id: String) {
        if self.find_status(&status_id).is_none() {
            return;
        }
        self.set_selected_status(status_id);
    }

    fn select_first_status(self: &Rc<Self>) {
        let active_pane = self.model.borrow().active_pane;
        match active_pane {
            ActivePane::Timeline => {
                let order = timeline_selection_order(&self.model.borrow());
                let Some(status_id) = order.first() else {
                    return;
                };
                self.set_selected_status(status_id.clone());
            }
            ActivePane::Notifications => {
                let order = notification_selection_order(&self.model.borrow());
                let Some(group_key) = order.first() else {
                    return;
                };
                self.set_selected_notification(group_key.clone());
            }
            ActivePane::DetailModal => {
                let order = detail_selection_order(&self.model.borrow());
                let Some(status_id) = order.first() else {
                    return;
                };
                self.set_selected_status(status_id.clone());
            }
        }
    }

    fn open_selected_detail(self: &Rc<Self>) {
        let Some(status_id) = self.detail_target_status_id() else {
            return;
        };
        spawn_local({
            let app = self.clone();
            async move {
                app.open_detail_modal(status_id).await;
            }
        });
    }

    fn cycle_pane_focus(self: &Rc<Self>, direction: isize) {
        if self.model.borrow().active_pane == ActivePane::DetailModal {
            return;
        }
        self.blur_active_element();

        let next_pane = {
            let model = self.model.borrow();
            match (model.last_non_modal_pane, direction.is_negative()) {
                (Pane::Timeline, false) => Pane::Notifications,
                (Pane::Notifications, false) => Pane::Timeline,
                (Pane::Timeline, true) => Pane::Notifications,
                (Pane::Notifications, true) => Pane::Timeline,
            }
        };
        {
            let mut model = self.model.borrow_mut();
            model.last_non_modal_pane = next_pane;
            model.active_pane = match next_pane {
                Pane::Timeline => ActivePane::Timeline,
                Pane::Notifications => ActivePane::Notifications,
            };
        }
        self.render();
        self.sync_selected_element();
    }

    async fn open_detail_modal(self: Rc<Self>, status_id: String) {
        let origin_pane = {
            let mut model = self.model.borrow_mut();
            let origin = model.last_non_modal_pane;
            model.active_pane = ActivePane::DetailModal;
            model.detail_return_status_id = if origin == Pane::Notifications {
                model.selected_status_id.clone()
            } else {
                None
            };
            origin
        };
        self.clone()
            .load_thread_internal(status_id, origin_pane == Pane::Timeline)
            .await;
        {
            let mut model = self.model.borrow_mut();
            model.active_pane = ActivePane::DetailModal;
        }
        self.render();
        self.sync_selected_element();
    }

    fn close_detail_modal(self: &Rc<Self>) {
        let should_render = {
            let mut model = self.model.borrow_mut();
            if model.active_pane != ActivePane::DetailModal {
                if model.composer_popout {
                    model.composer_popout = false;
                    true
                } else {
                    false
                }
            } else {
                if model.last_non_modal_pane == Pane::Notifications
                    && let Some(status_id) = model.detail_return_status_id.take()
                {
                    model.selected_status_id = Some(status_id);
                }
                model.active_pane = match model.last_non_modal_pane {
                    Pane::Timeline => ActivePane::Timeline,
                    Pane::Notifications => ActivePane::Notifications,
                };
                model.thread = ThreadView::default();
                model.thread_history.clear();
                true
            }
        };
        if should_render {
            self.render();
            self.sync_selected_element();
        }
    }

    fn detail_target_status_id(&self) -> Option<String> {
        match self.model.borrow().active_pane {
            ActivePane::Timeline | ActivePane::DetailModal => self.shortcut_status_id(),
            ActivePane::Notifications => self.selected_notification_group().and_then(|group| {
                group
                    .status
                    .as_ref()
                    .map(|status| display_status(status).id.clone())
            }),
        }
    }

    fn activate_mention_shortcut(self: &Rc<Self>) {
        let Some(status) = self.shortcut_status() else {
            return;
        };
        let primary = display_status(&status);
        let mention = format!("@{}", primary.account.acct);
        self.update_composer({
            let mention = mention.clone();
            move |composer| {
                let already_present = composer
                    .status
                    .split_whitespace()
                    .any(|word| word == mention.as_str());
                if already_present {
                    return;
                }

                if composer.status.trim().is_empty() {
                    composer.status = format!("{mention} ");
                } else {
                    composer.status = format!("{}\n{mention} ", composer.status.trim_end());
                }
            }
        });
        {
            let mut model = self.model.borrow_mut();
            model.composer_popout = true;
        }
        self.render();
        self.focus_composer_input();
    }

    fn selected_focus_selector(&self) -> Option<String> {
        match self.model.borrow().active_pane {
            ActivePane::Timeline => self
                .model
                .borrow()
                .selected_status_id
                .clone()
                .map(|status_id| format!(r#".timeline-list [data-focus-status="{}"]"#, status_id)),
            ActivePane::Notifications => {
                self.model
                    .borrow()
                    .selected_notification_key
                    .clone()
                    .map(|group_key| {
                        format!(
                            r#".activity-column [data-focus-notification="{}"]"#,
                            group_key
                        )
                    })
            }
            ActivePane::DetailModal => self
                .model
                .borrow()
                .selected_status_id
                .clone()
                .map(|status_id| format!(r#".detail-modal [data-focus-status="{}"]"#, status_id)),
        }
    }

    fn sync_selected_element(&self) {
        let selector = self.selected_focus_selector();
        let Some(selector) = selector else {
            return;
        };
        let Ok(Some(element)) = self.document.query_selector(&selector) else {
            return;
        };
        element.scroll_into_view();
        if let Ok(html_element) = element.dyn_into::<HtmlElement>() {
            let _ = html_element.focus();
        }
    }

    fn shortcut_status(&self) -> Option<Status> {
        match self.model.borrow().active_pane {
            ActivePane::Timeline => self
                .model
                .borrow()
                .selected_status_id
                .clone()
                .and_then(|status_id| self.find_status(&status_id)),
            ActivePane::Notifications => self
                .selected_notification_group()
                .and_then(|group| group.status.clone()),
            ActivePane::DetailModal => self
                .model
                .borrow()
                .selected_status_id
                .clone()
                .and_then(|status_id| self.find_thread_status(&status_id)),
        }
    }

    fn shortcut_status_id(&self) -> Option<String> {
        self.shortcut_status()
            .map(|status| display_status(&status).id.clone())
    }

    fn activate_quote_shortcut(self: &Rc<Self>) {
        let Some(status) = self.shortcut_status() else {
            return;
        };
        let primary = display_status(&status);
        let label = format!(
            "@{} · {}",
            primary.account.acct,
            summarize_html(&primary.content)
        );
        let quote_target = if primary.uri.trim().is_empty() {
            primary.id.clone()
        } else {
            primary.uri.clone()
        };
        self.set_quote_target(quote_target, label);
    }

    fn update_composer<F>(&self, update: F)
    where
        F: FnOnce(&mut ComposerDraft),
    {
        {
            let mut model = self.model.borrow_mut();
            update(&mut model.composer);
        }
        self.save_composer_draft();
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

        let notifications_url = self.notifications_url();
        let statuses_future = self.fetch_active_feed();
        let (instance, account, statuses, notifications, unread, backups, domain_blocks) = futures::join!(
            fetch_json::<Instance>("/api/v1/instance"),
            fetch_json::<Account>("/api/v1/accounts/verify_credentials"),
            statuses_future,
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
        normalize_selected_status(&mut model);
        drop(model);
        self.render();
    }

    async fn refresh_social(self: Rc<Self>, focus_status: Option<String>) {
        {
            let mut model = self.model.borrow_mut();
            model.feed_loading = true;
        }
        self.render();

        let notifications_url = self.notifications_url();
        let selected_thread = {
            let model = self.model.borrow();
            if model.active_pane == ActivePane::DetailModal {
                focus_status
                    .clone()
                    .or_else(|| model.thread.status_id.clone())
            } else {
                None
            }
        };

        let statuses_future = self.fetch_active_feed();
        let (account, statuses, notifications, unread) = futures::join!(
            fetch_json::<Account>("/api/v1/accounts/verify_credentials"),
            statuses_future,
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
                Ok(statuses) => {
                    model.statuses = statuses;
                    if let Some(status_id) = focus_status.as_ref()
                        && model
                            .statuses
                            .iter()
                            .any(|status| display_status(status).id == *status_id)
                    {
                        model.selected_status_id = Some(status_id.clone());
                    }
                }
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
            normalize_selected_status(&mut model);
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
        normalize_selected_status(&mut model);
        drop(model);
        self.render();
    }

    async fn fetch_active_feed(&self) -> Result<Vec<Status>, String> {
        let (mode, hashtag_query) = {
            let model = self.model.borrow();
            (model.feed_mode, model.hashtag_query.clone())
        };

        match mode {
            FeedMode::Hashtags => self.fetch_hashtag_feed(&hashtag_query).await,
            _ => {
                let feed_url = self.feed_url();
                fetch_json::<Vec<Status>>(&feed_url).await
            }
        }
    }

    async fn fetch_hashtag_feed(&self, raw_query: &str) -> Result<Vec<Status>, String> {
        let hashtags = parse_hashtag_query(raw_query);
        if hashtags.is_empty() {
            return Ok(Vec::new());
        }

        let mut merged = Vec::<Status>::new();
        for hashtag in hashtags {
            let endpoint = format!(
                "/api/v1/timelines/tag/{}?limit={DEFAULT_FEED_LIMIT}",
                encode_path_segment(&hashtag)
            );
            let statuses = fetch_json::<Vec<Status>>(&endpoint).await?;
            for status in statuses {
                let primary_uri = display_status(&status).uri.clone();
                let seen = merged
                    .iter()
                    .any(|existing| display_status(existing).uri == primary_uri);
                if !seen {
                    merged.push(status);
                }
            }
        }
        merged.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        merged.truncate(DEFAULT_FEED_LIMIT);
        Ok(merged)
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
        self.load_thread_internal(status_id, true).await;
    }

    async fn load_thread_internal(self: Rc<Self>, status_id: String, record_history: bool) {
        let initial_focus = self
            .find_status(&status_id)
            .or_else(|| self.find_notification_status(&status_id));
        {
            let mut model = self.model.borrow_mut();
            let is_new_thread = model.thread.status_id.as_deref() != Some(status_id.as_str());
            if record_history
                && let Some(previous_status_id) = model.thread.status_id.clone()
                && previous_status_id != status_id
                && model.thread_history.last() != Some(&previous_status_id)
            {
                model.thread_history.push(previous_status_id);
            }
            model.thread.status_id = Some(status_id.clone());
            model.selected_status_id = Some(status_id.clone());
            model.thread.loading = true;
            if model.thread.focus.is_none() || is_new_thread {
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
        normalize_selected_status(&mut model);
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
        if let Some(quote_target) = composer.quoted_status_id.as_ref() {
            payload.insert(
                "quoted_status_id".to_string(),
                serde_json::Value::String(quote_target.clone()),
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
                self.clear_saved_draft();
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

    async fn dismiss_notifications(self: Rc<Self>, notification_ids: Vec<String>) {
        if notification_ids.is_empty() {
            return;
        }

        for notification_id in &notification_ids {
            let url = format!(
                "/api/v1/notifications/{}/dismiss",
                encode_path_segment(notification_id)
            );
            if let Err(error) = send_request("POST", &url, None).await {
                self.set_flash(FlashTone::Error, &error);
                return;
            }
        }
        let success_message = if notification_ids.len() > 1 {
            "Notification group dismissed."
        } else {
            "Notification dismissed."
        };
        self.set_flash(FlashTone::Success, success_message);
        self.refresh_notifications().await;
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

    fn storage(&self) -> Option<Storage> {
        self.window.local_storage().ok().flatten()
    }

    fn save_composer_draft(&self) {
        let Some(storage) = self.storage() else {
            return;
        };
        let composer = self.model.borrow().composer.clone();
        if !composer_has_saved_state(&composer) {
            let _ = storage.remove_item(COMPOSER_DRAFT_STORAGE_KEY);
            return;
        }
        if let Ok(serialized) = serde_json::to_string(&composer) {
            let _ = storage.set_item(COMPOSER_DRAFT_STORAGE_KEY, &serialized);
        }
    }

    fn clear_saved_draft(&self) {
        if let Some(storage) = self.storage() {
            let _ = storage.remove_item(COMPOSER_DRAFT_STORAGE_KEY);
        }
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
            FeedMode::Hashtags => String::new(),
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

    fn find_thread_status(&self, id: &str) -> Option<Status> {
        let model = self.model.borrow();
        model
            .thread
            .ancestors
            .iter()
            .chain(model.thread.focus.iter())
            .chain(model.thread.descendants.iter())
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

    fn selected_notification_group(&self) -> Option<NotificationGroup> {
        let model = self.model.borrow();
        let selected_key = model.selected_notification_key.as_deref()?;
        group_notifications(&model.notifications)
            .into_iter()
            .find(|group| notification_group_selection_key(group) == selected_key)
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

    let composer_panel = if model.composer_popout {
        String::new()
    } else {
        render_composer(model, false)
    };
    let composer_popout = if model.composer_popout {
        render_composer(model, true)
    } else {
        String::new()
    };
    let detail_modal = render_detail_modal(model);
    let shortcut_help = render_shortcut_help(model);
    let hashtag_query = encode_attribute(&model.hashtag_query);

    format!(
        r#"
<div class="app-shell" data-active-pane="{active_pane}">
  <aside class="sidebar">
    <div class="brand-lockup">
      <div class="brand-orb">rr</div>
      <div class="brand-copy">
        <p class="micro-label">Rust/WASM</p>
        <h1>{brand_title}</h1>
        <p class="lede">Integrated client tuned to Mastodon-compatible APIs first, with RustResort admin controls layered beside it.</p>
      </div>
    </div>
    <nav class="sidebar-nav" aria-label="Primary feeds">
      <button id="nav-home" class="sidebar-link {home_active}" aria-pressed="{home_pressed}">Home</button>
      <button id="nav-public" class="sidebar-link {public_active}" aria-pressed="{public_pressed}">Local</button>
      <button id="nav-profile" class="sidebar-link {profile_active}" aria-pressed="{profile_pressed}">Posts</button>
      <button id="nav-hashtags" class="sidebar-link {hashtags_active}" aria-pressed="{hashtags_pressed}">Hashtags</button>
      <label class="field sidebar-field">
        <span>Hashtags</span>
        <input id="hashtag-query" type="text" value="{hashtag_query}" placeholder="rust, wasm, activitypub" />
      </label>
    </nav>
    {profile_panel}
    <details class="sidebar-more">
      <summary>More</summary>
      <div class="sidebar-more-links">
        <button type="button" class="sidebar-link compact" data-open-shortcuts="true">Keyboard shortcuts</button>
        <a class="sidebar-link compact" href="/settings">Admin / settings</a>
        <a class="sidebar-link compact" href="/api/v1/accounts/verify_credentials">Raw Mastodon JSON</a>
        <button id="logout-action" class="sidebar-link compact danger">Log out</button>
      </div>
      <p class="sidebar-more-note">Social UI stays on Mastodon-style endpoints under <code>/api/v1</code>. Admin flows live on the standalone settings page.</p>
    </details>
  </aside>

  <main class="timeline-column {timeline_active} {feed_mode_class}" aria-label="Timeline pane">
    <header class="timeline-header" aria-current="{timeline_current}">
      <div>
        <p class="micro-label">Timeline</p>
        <div class="timeline-identity">
          <span class="timeline-feed-pill">{feed_label}</span>
          <span class="timeline-feed-context">{feed_context}</span>
          {feed_query_chip}
        </div>
        <h2>{feed_label}</h2>
        <div class="timeline-meta">
          <p class="subtle-line">{feed_subtitle}</p>
          <div class="shortcut-row">
            <button type="button" class="shortcut-inline-link" data-open-shortcuts="true">
              <kbd>?</kbd>
              <span>Keyboard shortcuts</span>
            </button>
          </div>
        </div>
      </div>
      <div class="timeline-actions">
        <button id="composer-toggle-popout" class="ghost-button">Post</button>
        <button id="refresh-feed" class="ghost-button">Refresh</button>
        <button type="button" class="ghost-button shortcut-help-trigger" aria-label="Open keyboard shortcuts" data-open-shortcuts="true">?</button>
      </div>
    </header>

    {composer_panel}

    {flash_banner}

    <section class="timeline-list" aria-label="Timeline posts" role="listbox">
      {timeline_cards}
    </section>
  </main>

  <aside class="activity-column {notifications_active}" aria-label="Notifications pane">
    <section class="rail-card" aria-labelledby="notifications-title">
      <div class="rail-card-head">
        <div>
          <p class="micro-label">Notifications</p>
          <h3 id="notifications-title">Signals</h3>
        </div>
        {notification_count}
      </div>
      <div class="filter-row" aria-label="Notification filters">
        {notification_filters}
      </div>
      <div class="rail-list" aria-label="Notification groups" role="listbox">
        {notifications}
      </div>
      <div class="rail-actions">
        <button id="notifications-clear" class="ghost-button">Clear all</button>
      </div>
    </section>
  </aside>
</div>
{detail_modal}
{composer_popout}
{shortcut_help}
"#,
        active_pane = match model.active_pane {
            ActivePane::Timeline => "timeline",
            ActivePane::Notifications => "notifications",
            ActivePane::DetailModal => "detail",
        },
        brand_title = encode_text(brand_title),
        timeline_active = if matches!(model.active_pane, ActivePane::Timeline) {
            "active-pane"
        } else {
            ""
        },
        timeline_current = if matches!(model.active_pane, ActivePane::Timeline) {
            "true"
        } else {
            "false"
        },
        notifications_active = if matches!(model.active_pane, ActivePane::Notifications) {
            "active-pane"
        } else {
            ""
        },
        home_active = if model.feed_mode == FeedMode::Home {
            "active"
        } else {
            ""
        },
        home_pressed = if model.feed_mode == FeedMode::Home {
            "true"
        } else {
            "false"
        },
        public_active = if model.feed_mode == FeedMode::Public {
            "active"
        } else {
            ""
        },
        public_pressed = if model.feed_mode == FeedMode::Public {
            "true"
        } else {
            "false"
        },
        profile_active = if model.feed_mode == FeedMode::Profile {
            "active"
        } else {
            ""
        },
        profile_pressed = if model.feed_mode == FeedMode::Profile {
            "true"
        } else {
            "false"
        },
        hashtags_active = if model.feed_mode == FeedMode::Hashtags {
            "active"
        } else {
            ""
        },
        hashtags_pressed = if model.feed_mode == FeedMode::Hashtags {
            "true"
        } else {
            "false"
        },
        hashtag_query = hashtag_query,
        profile_panel = render_profile_panel(model),
        feed_label = encode_text(model.feed_mode.label()),
        feed_context = encode_text(feed_context_label(model)),
        feed_query_chip = render_feed_query_chip(model),
        feed_mode_class = feed_mode_class(model),
        feed_subtitle = encode_text(&feed_subtitle(model)),
        composer_panel = composer_panel,
        composer_popout = composer_popout,
        detail_modal = detail_modal,
        shortcut_help = shortcut_help,
        flash_banner = render_flash(model),
        timeline_cards = render_timeline(model),
        notification_count = render_notification_count(model.notifications_unread),
        notification_filters = render_notification_filters(model),
        notifications = render_notifications(model),
    )
}

fn render_composer(model: &Model, popout: bool) -> String {
    let reply_banner = render_reply_banner(model);
    let quote_banner = render_quote_banner(model);
    let composer_count = model.composer.status.chars().count();
    let max_characters = composer_limit(model);
    let submit_disabled = composer_count == 0 || composer_count > max_characters;
    let shell_class = if popout {
        "composer-popout-shell"
    } else {
        "composer-inline-shell"
    };
    let panel_class = if popout {
        "composer-panel composer-panel-popout"
    } else {
        "composer-panel"
    };
    let dismiss = if popout {
        r#"<button id="composer-close-popout" class="ghost-button">Close</button>"#.to_string()
    } else {
        String::new()
    };
    let composer_heading = if popout {
        r#"<div>
          <p class="micro-label">Compose</p>
          <h3>New post</h3>
          <p class="composer-note">Start with the post body. Visibility, spoiler text, and language are optional.</p>
        </div>"#
            .to_string()
    } else {
        r#"<div class="composer-inline-copy">
          <p class="micro-label">Compose</p>
          <h3>Write a post</h3>
        </div>"#
            .to_string()
    };
    let inline_shortcut_note = if popout {
        r#"<span class="composer-shortcut-note"><kbd>n</kbd> compose <kbd>N</kbd> mention</span>"#
            .to_string()
    } else {
        String::new()
    };

    format!(
        r#"<section class="{shell_class}">
  <section class="{panel_class}">
    <div class="composer-avatar">{composer_avatar}</div>
    <div class="composer-stack">
      <div class="composer-head">
        {composer_heading}
        {dismiss}
      </div>
      {reply_banner}
      {quote_banner}
      <div class="composer-main-field">
        <div class="composer-main-head">
          <span class="composer-main-label">Post body</span>
          {inline_shortcut_note}
        </div>
        <textarea id="composer-input" placeholder="What do you want to post?">{composer_text}</textarea>
      </div>
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
        {composer_budget}
        <button id="composer-submit" class="primary-button {submit_state_class}" data-disabled-state="{submit_disabled}">Post</button>
      </div>
    </div>
  </section>
</section>"#,
        shell_class = shell_class,
        panel_class = panel_class,
        composer_avatar = render_avatar_monogram(model),
        composer_heading = composer_heading,
        dismiss = dismiss,
        reply_banner = reply_banner,
        quote_banner = quote_banner,
        inline_shortcut_note = inline_shortcut_note,
        composer_text = encode_text(&model.composer.status),
        visibility_options = render_visibility_options(&model.composer.visibility),
        spoiler_text = encode_attribute(&model.composer.spoiler_text),
        language = encode_attribute(&model.composer.language),
        composer_budget = render_composer_budget(composer_count, max_characters),
        submit_state_class = if submit_disabled {
            "button-disabled"
        } else {
            ""
        },
        submit_disabled = if submit_disabled { "true" } else { "false" },
    )
}

fn render_profile_panel(model: &Model) -> String {
    let Some(account) = model.account.as_ref() else {
        return String::new();
    };

    let note = account
        .source
        .as_ref()
        .map(|source| source.note.trim())
        .filter(|note| !note.is_empty())
        .map(str::to_string);

    let follow_requests = account
        .source
        .as_ref()
        .and_then(|source| source.follow_requests_count)
        .unwrap_or(0);

    let session_label = model
        .session
        .as_ref()
        .map(|session| format!("Signed in via {}", session.auth_method))
        .unwrap_or_else(|| "Session loading".to_string());

    let note_markup = note
        .as_ref()
        .map(|value| format!(r#"<p class="bio">{}</p>"#, encode_text(value)))
        .unwrap_or_default();

    let follow_request_chip = if follow_requests > 0 {
        format!(r#"<span class="chip">{follow_requests} follow requests</span>"#)
    } else {
        String::new()
    };
    let chip_row = if follow_request_chip.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="chip-row">{follow_request_chip}</div>"#)
    };

    format!(
        r#"<section class="profile-panel profile-panel-compact">
  <div class="profile-body compact">
    <div class="profile-summary">
      {avatar}
      <div class="profile-text">
        <h3>{name}</h3>
        <p class="handle">@{acct}</p>
      </div>
    </div>
    <p class="profile-meta-line">{session}</p>
    {note_markup}
    <div class="stat-grid compact">
      <div><strong>{statuses}</strong><span>posts</span></div>
      <div><strong>{followers}</strong><span>followers</span></div>
      <div><strong>{following}</strong><span>following</span></div>
    </div>
    {chip_row}
  </div>
</section>"#,
        avatar = avatar_markup(
            "profile-avatar compact",
            &account.avatar,
            &display_name(account)
        ),
        name = encode_text(&display_name(account)),
        acct = encode_text(&account.acct),
        session = encode_text(&session_label),
        note_markup = note_markup,
        statuses = account.statuses_count,
        followers = account.followers_count,
        following = account.following_count,
        chip_row = chip_row,
    )
}

fn render_reply_banner(model: &Model) -> String {
    let Some(label) = model.composer.in_reply_to_label.as_ref() else {
        return String::new();
    };
    let (author, summary) = split_composer_context_label(label);

    format!(
        r#"<div class="reply-banner">
  <div class="composer-context-copy">
    <span class="composer-context-title">Replying to</span>
    <div class="composer-context-line">
      {author}
      <span class="composer-context-summary">{summary}</span>
    </div>
  </div>
  <button id="composer-cancel-reply" class="ghost-button small">Cancel</button>
</div>"#,
        author = author
            .map(|value| format!(
                r#"<span class="composer-context-author">{}</span>"#,
                encode_text(value)
            ))
            .unwrap_or_default(),
        summary = encode_text(summary),
    )
}

fn render_quote_banner(model: &Model) -> String {
    let Some(label) = model.composer.quoted_status_label.as_ref() else {
        return String::new();
    };
    let (author, summary) = split_composer_context_label(label);

    format!(
        r#"<div class="reply-banner quote-banner">
  <div class="composer-context-copy">
    <span class="composer-context-title">Quoting</span>
    <div class="composer-context-line">
      {author}
      <span class="composer-context-summary">{summary}</span>
    </div>
  </div>
  <button id="composer-cancel-quote" class="ghost-button small">Cancel</button>
</div>"#,
        author = author
            .map(|value| format!(
                r#"<span class="composer-context-author">{}</span>"#,
                encode_text(value)
            ))
            .unwrap_or_default(),
        summary = encode_text(summary),
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
        r#"<div class="flash-banner {tone}" role="status" aria-live="polite">{text}</div>"#,
        tone = tone,
        text = encode_text(&flash.text),
    )
}

fn render_composer_budget(count: usize, max_characters: usize) -> String {
    let remaining = max_characters as isize - count as isize;
    let ratio = if max_characters == 0 {
        0.0
    } else {
        count as f64 / max_characters as f64
    };
    let width = (ratio.clamp(0.0, 1.0) * 100.0).round();
    let tone_class = if remaining < 0 {
        "danger"
    } else if remaining <= 40 || ratio >= 0.9 {
        "warning"
    } else {
        "normal"
    };
    let remaining_label = if remaining < 0 {
        format!("{} over", remaining.unsigned_abs())
    } else {
        format!("{remaining} left")
    };

    format!(
        r#"<div class="composer-budget {tone_class}">
  <div class="composer-budget-copy">
    <strong>{count}/{max_characters}</strong>
    <span>{remaining_label}</span>
  </div>
  <div class="composer-budget-track" aria-hidden="true">
    <span class="composer-budget-fill" style="width: {width}%;"></span>
  </div>
</div>"#,
        tone_class = tone_class,
        count = count,
        max_characters = max_characters,
        remaining_label = encode_text(&remaining_label),
        width = width,
    )
}

fn split_composer_context_label(label: &str) -> (Option<&str>, &str) {
    let trimmed = label.trim();
    if let Some((author, summary)) = trimmed.split_once('·') {
        let author = author.trim();
        let summary = summary.trim();
        if !author.is_empty() && !summary.is_empty() {
            return (Some(author), summary);
        }
    }
    (None, trimmed)
}

fn render_detail_modal(model: &Model) -> String {
    if model.active_pane != ActivePane::DetailModal
        || (model.thread.status_id.is_none() && !model.thread.loading)
    {
        return String::new();
    }

    let ancestor_count = model.thread.ancestors.len();
    let descendant_count = model.thread.descendants.len();
    let thread_summary = if model.thread.focus.is_some() {
        format!("{} earlier · {} replies", ancestor_count, descendant_count)
    } else {
        "Conversation unavailable".to_string()
    };

    let content = if model.thread.loading && model.thread.focus.is_none() {
        r#"<div class="thread-empty">Loading conversation…</div>"#.to_string()
    } else if let Some(focus) = model.thread.focus.as_ref() {
        let ancestors = if model.thread.ancestors.is_empty() {
            String::new()
        } else {
            format!(
                r#"<section class="thread-section thread-section-ancestors">
  <p class="thread-section-label">Earlier in thread</p>
  <div class="thread-stack">{}</div>
</section>"#,
                model
                    .thread
                    .ancestors
                    .iter()
                    .map(|status| render_status_card(status, model, true, false))
                    .collect::<Vec<_>>()
                    .join("")
            )
        };
        let descendants = if model.thread.descendants.is_empty() {
            String::new()
        } else {
            format!(
                r#"<section class="thread-section thread-section-descendants">
  <p class="thread-section-label">Replies</p>
  <div class="thread-stack">{}</div>
</section>"#,
                model
                    .thread
                    .descendants
                    .iter()
                    .map(|status| render_status_card(status, model, true, false))
                    .collect::<Vec<_>>()
                    .join("")
            )
        };

        format!(
            r#"{ancestors}
<div class="thread-focus">
  <p class="thread-section-label">Selected post</p>
  {focus}
</div>
{descendants}"#,
            ancestors = ancestors,
            focus = render_status_card(focus, model, false, true),
            descendants = descendants,
        )
    } else {
        r#"<div class="thread-empty">Conversation unavailable.</div>"#.to_string()
    };

    let thread_badge = if model.thread.focus.is_some() {
        let total = model.thread.ancestors.len() + model.thread.descendants.len() + 1;
        let position = model.thread.ancestors.len() + 1;
        if total > 1 {
            format!(r#"<span class="status-chip subtle">Thread {position}/{total}</span>"#)
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    format!(
        r#"<section class="detail-modal-shell">
  <div class="detail-modal" role="dialog" aria-modal="true" aria-labelledby="detail-modal-title">
    <div class="thread-head">
      <div class="detail-title-group">
        <div class="detail-title-row">
          <p class="micro-label">Conversation</p>
          {thread_badge}
        </div>
        <h3 id="detail-modal-title">Thread</h3>
        <p class="detail-summary">{thread_summary}</p>
        <p class="detail-note">Move with <kbd>j</kbd>/<kbd>k</kbd>. Press <kbd>Esc</kbd> to return.</p>
      </div>
      <div class="detail-actions">
        <button id="thread-close" class="ghost-button small detail-close">Close</button>
      </div>
    </div>
    <div class="thread-panel" role="listbox" aria-label="Conversation posts">
      {content}
    </div>
  </div>
</section>"#,
        thread_badge = thread_badge,
        thread_summary = encode_text(&thread_summary),
        content = content,
    )
}

fn render_shortcut_help(model: &Model) -> String {
    if !model.shortcut_help_open {
        return String::new();
    }

    r#"<section class="shortcut-modal-shell">
  <div class="shortcut-modal" role="dialog" aria-modal="true" aria-labelledby="shortcut-modal-title">
    <div class="shortcut-modal-head">
      <div class="shortcut-modal-copy">
        <p class="micro-label">Keyboard</p>
        <h3 id="shortcut-modal-title">Shortcuts</h3>
        <p class="shortcut-modal-note">Selection follows the active pane. Timeline and thread views share the same movement keys.</p>
      </div>
      <button id="shortcut-help-close" class="ghost-button small detail-close">Close</button>
    </div>
    <div class="shortcut-modal-grid">
      <section class="shortcut-section">
        <h4>Navigation</h4>
        <div class="shortcut-list">
          <div class="shortcut-entry"><span><kbd>j</kbd>/<kbd>k</kbd></span><span>Move selected post</span></div>
          <div class="shortcut-entry"><span><kbd>Tab</kbd></span><span>Switch timeline and notifications</span></div>
          <div class="shortcut-entry"><span><kbd>g</kbd></span><span>Jump to top and select first post</span></div>
          <div class="shortcut-entry"><span><kbd>d</kbd></span><span>Open selected thread</span></div>
          <div class="shortcut-entry"><span><kbd>Esc</kbd></span><span>Close dialog or popout</span></div>
        </div>
      </section>
      <section class="shortcut-section">
        <h4>Compose</h4>
        <div class="shortcut-list">
          <div class="shortcut-entry"><span><kbd>n</kbd></span><span>Open compose window</span></div>
          <div class="shortcut-entry"><span><kbd>N</kbd></span><span>Mention selected post</span></div>
          <div class="shortcut-entry"><span><kbd>q</kbd></span><span>Quote selected post</span></div>
          <div class="shortcut-entry"><span><kbd>?</kbd></span><span>Open this help</span></div>
        </div>
      </section>
      <section class="shortcut-section">
        <h4>Actions</h4>
        <div class="shortcut-list">
          <div class="shortcut-entry"><span><kbd>f</kbd></span><span>Favourite selected post</span></div>
          <div class="shortcut-entry"><span><kbd>r</kbd></span><span>Boost selected post</span></div>
        </div>
      </section>
    </div>
  </div>
</section>"#
        .to_string()
}

fn render_timeline(model: &Model) -> String {
    if model.dashboard_loading && model.statuses.is_empty() {
        return r#"<article class="status-card empty"><p>Loading posts…</p></article>"#.to_string();
    }

    if model.statuses.is_empty() {
        return r#"<article class="status-card empty"><p>No posts in this feed yet.</p></article>"#
            .to_string();
    }

    model
        .statuses
        .iter()
        .map(|status| render_status_card(status, model, false, false))
        .collect::<Vec<_>>()
        .join("")
}

fn render_status_card(status: &Status, model: &Model, compact: bool, expanded: bool) -> String {
    let primary = display_status(status);
    let mut card_classes = vec!["status-card"];
    if compact {
        card_classes.push("compact");
    }
    if model.selected_status_id.as_deref() == Some(primary.id.as_str()) {
        card_classes.push("selected");
    }
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
    let handle = compact_handle_markup(
        &display_name_from_status(&primary.account),
        &primary.account.acct,
    );
    let content_markup = render_status_content(primary, expanded);

    let is_selected = model.selected_status_id.as_deref() == Some(primary.id.as_str());

    format!(
        r#"<article class="{card_classes}" data-focus-status="{select_target}" tabindex="{tabindex}" role="option" aria-selected="{aria_selected}">
  {boost_banner}
  <div class="status-head">
    {avatar}
    <div class="status-meta">
      <div class="status-line">
        <strong>{display_name}</strong>
        {handle}
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
        card_classes = card_classes.join(" "),
        boost_banner = boost_banner.unwrap_or_default(),
        tabindex = if is_selected { "0" } else { "-1" },
        aria_selected = if is_selected { "true" } else { "false" },
        avatar = avatar_markup(
            "status-avatar",
            &primary.account.avatar,
            &display_name_from_status(&primary.account)
        ),
        display_name = encode_text(&display_name_from_status(&primary.account)),
        handle = handle,
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
        content = content_markup,
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
            let kind = if attachment.media_type.trim().is_empty() {
                "media".to_string()
            } else if matches!(attachment.media_type.as_str(), "video" | "gifv") {
                format!("{} · preview only", attachment.media_type)
            } else {
                attachment.media_type.clone()
            };
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
                kind = encode_text(&kind),
            ))
        })
        .collect::<Vec<_>>()
        .join("");

    if items.is_empty() {
        return String::new();
    }

    format!(r#"<div class="media-grid">{items}</div>"#)
}

fn render_status_content(status: &Status, expanded: bool) -> String {
    if !expanded && !status.filtered.is_empty() {
        let labels = filtered_labels(&status.filtered);
        let title = if labels.is_empty() {
            "Filtered post".to_string()
        } else {
            format!("Filtered: {}", labels.join(", "))
        };
        return format!(
            r#"<div class="filtered-post-warning">
  <strong>{}</strong>
  <p>Open thread to reveal this post.</p>
</div>"#,
            encode_text(&title),
        );
    }

    if status.content.trim().is_empty() {
        return "<p class=\"muted-copy\">No text content</p>".to_string();
    }

    if !expanded && should_collapse_hashtag_stuffing(status) {
        let preview = summarize_text_for_collapsed_post(status);
        return format!(
            r#"<div class="collapsed-post">
  <p>{}</p>
  <small>Hashtag-heavy post collapsed. Open thread to expand.</small>
</div>"#,
            encode_text(&preview),
        );
    }

    status.content.clone()
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
            r#"<button class="filter-pill {active}" data-notification-filter="{value}" aria-pressed="{pressed}">{label}</button>"#,
            active = if model.notification_filter == filter {
                "active"
            } else {
                ""
            },
            pressed = if model.notification_filter == filter {
                "true"
            } else {
                "false"
            },
            value = value,
            label = filter.label(),
        )
    })
    .collect::<Vec<_>>()
    .join("")
}

fn render_notifications(model: &Model) -> String {
    let groups = group_notifications(&model.notifications);
    if groups.is_empty() {
        return r#"<div class="notification-card empty"><p>No notifications for this filter.</p></div>"#
            .to_string();
    }

    groups
        .iter()
        .map(|group| {
            let group_key = notification_group_selection_key(group);
            let kind_class = notification_kind_class(&group.notification_type);
            let jump_button = group.status.as_ref().and_then(|status| {
                let status_id = display_status(status).id.clone();
                model
                    .statuses
                    .iter()
                    .any(|candidate| display_status(candidate).id == status_id)
                    .then(|| {
                        format!(
                            r#"<button class="ghost-button small notification-jump-button" data-jump-status="{status_id}">Show in timeline</button>"#,
                            status_id = encode_attribute(&status_id),
                        )
                    })
            }).unwrap_or_default();
            let preview = group
                .status
                .as_ref()
                .map(|status| summarize_html(&display_status(status).content))
                .filter(|summary| !summary.is_empty())
                .unwrap_or_else(|| "Account-level event".to_string());
            let thread_button = group
                .status
                .as_ref()
                .map(|status| {
                    format!(
                        r#"<button class="ghost-button small notification-thread-button" data-select-status="{status_id}">Open thread</button>"#,
                        status_id = encode_attribute(&display_status(status).id),
                    )
                })
                .unwrap_or_default();
            let primary_account = group
                .accounts
                .first()
                .cloned()
                .unwrap_or_else(StatusAccount::default);
            let actor_label = grouped_actor_label(&group.accounts);
            let actor_handle = compact_handle_markup(
                &display_name_from_status(&primary_account),
                &primary_account.acct,
            );
            let count_badge = if group.count > 1 {
                format!(r#"<span class="notification-count-badge">+{}</span>"#, group.count - 1)
            } else {
                String::new()
            };
            let dismiss_ids = encode_attribute(&group.ids.join("|"));

            format!(
                r#"<div class="notification-card {selected} {kind_class}" data-focus-notification="{group_key}" tabindex="{tabindex}" role="option" aria-selected="{aria_selected}">
  <div class="notification-head">
    <strong class="notification-kind-pill {kind_class}">{kind}</strong>
    <span>{created_at}</span>
  </div>
  <div class="notification-user">
    {avatar}
    <div class="notification-actor-meta">
      <strong>{display_name}</strong>
      {handle}
    </div>
    {count_badge}
  </div>
  <p class="notification-preview">{preview}</p>
  <div class="notification-actions">
    {jump_button}
    {thread_button}
    <button class="ghost-button small" data-dismiss-notification="{notification_id}">Dismiss</button>
  </div>
</div>"#,
                selected = if model.active_pane == ActivePane::Notifications
                    && model.selected_notification_key.as_deref() == Some(group_key.as_str())
                {
                    "selected"
                } else {
                    ""
                },
                tabindex = if model.selected_notification_key.as_deref() == Some(group_key.as_str()) {
                    "0"
                } else {
                    "-1"
                },
                aria_selected = if model.selected_notification_key.as_deref() == Some(group_key.as_str()) {
                    "true"
                } else {
                    "false"
                },
                group_key = encode_attribute(&group_key),
                kind = encode_text(notification_kind_label(&group.notification_type)),
                created_at = encode_text(&short_timestamp(&group.created_at)),
                avatar = avatar_markup(
                    "status-avatar small",
                    &primary_account.avatar,
                    &display_name_from_status(&primary_account)
                ),
                display_name = encode_text(&actor_label),
                handle = actor_handle,
                count_badge = count_badge,
                preview = encode_text(&preview),
                jump_button = jump_button,
                thread_button = thread_button,
                notification_id = dismiss_ids,
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

fn timeline_selection_order(model: &Model) -> Vec<String> {
    model
        .statuses
        .iter()
        .map(|status| display_status(status).id.clone())
        .collect()
}

fn detail_selection_order(model: &Model) -> Vec<String> {
    let mut ids = model
        .thread
        .ancestors
        .iter()
        .map(|status| display_status(status).id.clone())
        .collect::<Vec<_>>();
    if let Some(focus) = model.thread.focus.as_ref() {
        ids.push(display_status(focus).id.clone());
    }
    ids.extend(
        model
            .thread
            .descendants
            .iter()
            .map(|status| display_status(status).id.clone()),
    );
    ids
}

fn notification_selection_order(model: &Model) -> Vec<String> {
    group_notifications(&model.notifications)
        .into_iter()
        .map(|group| notification_group_selection_key(&group))
        .collect()
}

fn next_selection_index(current_index: usize, len: usize, delta: isize) -> usize {
    if delta.is_negative() {
        current_index.saturating_sub(delta.unsigned_abs())
    } else {
        (current_index + delta as usize).min(len.saturating_sub(1))
    }
}

fn normalize_selected_status(model: &mut Model) {
    let timeline_order = timeline_selection_order(model);
    if timeline_order.is_empty() {
        model.selected_status_id = None;
    } else if !model
        .selected_status_id
        .as_deref()
        .is_some_and(|status_id| {
            timeline_order
                .iter()
                .any(|candidate| candidate == status_id)
        })
    {
        model.selected_status_id = timeline_order.first().cloned();
    }

    let notification_order = notification_selection_order(model);
    if notification_order.is_empty() {
        model.selected_notification_key = None;
    } else if !model
        .selected_notification_key
        .as_deref()
        .is_some_and(|group_key| {
            notification_order
                .iter()
                .any(|candidate| candidate == group_key)
        })
    {
        model.selected_notification_key = notification_order.first().cloned();
    }
}

fn feed_subtitle(model: &Model) -> String {
    if model.feed_loading {
        return "Refreshing feed and notifications…".to_string();
    }

    match model.feed_mode {
        FeedMode::Home => "Posts from accounts you follow.".to_string(),
        FeedMode::Public => "Recent local posts from the public timeline.".to_string(),
        FeedMode::Profile => "Posts from your account timeline.".to_string(),
        FeedMode::Hashtags => {
            let hashtags = parse_hashtag_query(&model.hashtag_query);
            if hashtags.is_empty() {
                "Merge multiple hashtags into one timeline.".to_string()
            } else {
                let preview = hashtags
                    .iter()
                    .take(3)
                    .map(|value| format!("#{value}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let remaining = hashtags.len().saturating_sub(3);
                if remaining > 0 {
                    format!("Posts matching {preview} and {remaining} more.")
                } else {
                    format!("Posts matching {preview}.")
                }
            }
        }
    }
}

fn feed_context_label(model: &Model) -> &'static str {
    match model.feed_mode {
        FeedMode::Home => "followed + recent",
        FeedMode::Public => "local public",
        FeedMode::Profile => "account archive",
        FeedMode::Hashtags => "merged hashtags",
    }
}

fn feed_mode_class(model: &Model) -> &'static str {
    match model.feed_mode {
        FeedMode::Home => "feed-home",
        FeedMode::Public => "feed-public",
        FeedMode::Profile => "feed-profile",
        FeedMode::Hashtags => "feed-hashtags",
    }
}

fn render_feed_query_chip(model: &Model) -> String {
    if model.feed_mode != FeedMode::Hashtags {
        return String::new();
    }
    let hashtags = parse_hashtag_query(&model.hashtag_query);
    if hashtags.is_empty() {
        return r#"<button type="button" class="timeline-feed-query empty" data-focus-hashtag-query="true">add hashtags</button>"#.to_string();
    }
    let preview = hashtags
        .iter()
        .take(3)
        .map(|value| format!("#{value}"))
        .collect::<Vec<_>>()
        .join(" ");
    let suffix = if hashtags.len() > 3 {
        format!(" +{}", hashtags.len() - 3)
    } else {
        String::new()
    };
    format!(
        r#"<button type="button" class="timeline-feed-query" data-focus-hashtag-query="true" title="{full_query}">{preview}</button>"#,
        full_query = encode_attribute(
            &hashtags
                .iter()
                .map(|value| format!("#{value}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        preview = encode_text(&format!("{preview}{suffix}"))
    )
}

fn render_notification_count(count: usize) -> String {
    if count == 0 {
        return r#"<div class="notification-count all-clear">All caught up</div>"#.to_string();
    }
    format!(
        r#"<div class="notification-count has-unread">{} unread</div>"#,
        count
    )
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

fn composer_has_saved_state(composer: &ComposerDraft) -> bool {
    !composer.status.trim().is_empty()
        || !composer.spoiler_text.trim().is_empty()
        || !composer.language.trim().is_empty()
        || composer.visibility != "public"
        || composer.in_reply_to_id.is_some()
        || composer.quoted_status_id.is_some()
}

fn restore_composer_draft(window: &Window) -> Option<ComposerDraft> {
    let storage = window.local_storage().ok().flatten()?;
    let raw = storage
        .get_item(COMPOSER_DRAFT_STORAGE_KEY)
        .ok()
        .flatten()?;
    serde_json::from_str(&raw).ok()
}

fn parse_hashtag_query(raw_query: &str) -> Vec<String> {
    raw_query
        .split([',', ' ', '\n', '\t'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_start_matches('#').to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .fold(Vec::<String>::new(), |mut values, hashtag| {
            if !values.iter().any(|existing| existing == &hashtag) {
                values.push(hashtag);
            }
            values
        })
}

fn compact_handle_markup(display_name: &str, acct: &str) -> String {
    let handle = short_acct(acct);
    if normalized_identity_label(display_name) == normalized_identity_label(&handle) {
        return String::new();
    }
    format!(r#"<span>@{}</span>"#, encode_text(&handle))
}

fn short_acct(acct: &str) -> String {
    acct.split('@')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(acct)
        .to_string()
}

fn normalized_identity_label(value: &str) -> String {
    value
        .trim()
        .chars()
        .flat_map(|character| character.to_lowercase())
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn notification_group_key(notification: &Notification) -> String {
    if !notification.group_key.trim().is_empty()
        && !notification.group_key.starts_with("ungrouped-")
    {
        return notification.group_key.clone();
    }
    let status_key = notification
        .status
        .as_ref()
        .map(|status| status.uri.as_str())
        .unwrap_or(notification.account.acct.as_str());
    format!("{}::{status_key}", notification.notification_type)
}

fn group_notifications(notifications: &[Notification]) -> Vec<NotificationGroup> {
    let mut groups = Vec::<NotificationGroup>::new();
    for notification in notifications {
        let key = notification_group_key(notification);
        if let Some(existing) = groups.iter_mut().find(|group| group.group_key == key) {
            existing.ids.push(notification.id.clone());
            existing.count += 1;
            if !existing
                .accounts
                .iter()
                .any(|account| account.acct == notification.account.acct)
            {
                existing.accounts.push(notification.account.clone());
            }
            continue;
        }

        groups.push(NotificationGroup {
            group_key: key,
            ids: vec![notification.id.clone()],
            notification_type: notification.notification_type.clone(),
            created_at: notification.created_at.clone(),
            accounts: vec![notification.account.clone()],
            status: notification.status.clone(),
            count: 1,
        });
    }
    groups
}

fn notification_group_selection_key(group: &NotificationGroup) -> String {
    if !group.group_key.trim().is_empty() {
        return group.group_key.clone();
    }
    group.ids.join("|")
}

fn grouped_actor_label(accounts: &[StatusAccount]) -> String {
    let Some(first) = accounts.first() else {
        return "Unknown actor".to_string();
    };
    let first_label = display_name_from_status(first);
    if accounts.len() == 1 {
        return first_label;
    }
    format!("{first_label} +{}", accounts.len() - 1)
}

fn notification_kind_label(kind: &str) -> &'static str {
    match kind {
        "mention" => "Mentions",
        "favourite" => "Likes",
        "reblog" => "Boosts",
        "follow" => "Follows",
        "status" => "Posts",
        _ => "Activity",
    }
}

fn notification_kind_class(kind: &str) -> &'static str {
    match kind {
        "mention" => "kind-mention",
        "favourite" => "kind-favourite",
        "reblog" => "kind-reblog",
        "follow" => "kind-follow",
        "status" => "kind-status",
        _ => "kind-generic",
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

fn summarize_text_for_collapsed_post(status: &Status) -> String {
    let source = if status.text.trim().is_empty() {
        summarize_html(&status.content)
    } else {
        status.text.trim().to_string()
    };
    let normalized = source.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated = normalized.chars().take(160).collect::<String>();
    if normalized.chars().count() > 160 {
        format!("{truncated}…")
    } else {
        normalized
    }
}

fn should_collapse_hashtag_stuffing(status: &Status) -> bool {
    let source = if status.text.trim().is_empty() {
        summarize_html(&status.content)
    } else {
        status.text.trim().to_string()
    };
    let tokens = source.split_whitespace().collect::<Vec<_>>();
    if tokens.len() < 4 {
        return false;
    }
    let hashtag_tokens = tokens
        .iter()
        .filter(|token| token.starts_with('#') && token.len() > 1)
        .count();
    hashtag_tokens >= 4 && hashtag_tokens * 10 >= tokens.len() * 6 && source.chars().count() > 80
}

fn filtered_labels(entries: &[serde_json::Value]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("title")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    entry
                        .get("filter")
                        .and_then(|value| value.get("title"))
                        .and_then(serde_json::Value::as_str)
                })
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .collect()
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
