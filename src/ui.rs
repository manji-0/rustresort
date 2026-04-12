//! Integrated Rust/WASM UI routes.

use axum::{
    Router,
    body::Body,
    http::{HeaderValue, Response, StatusCode, header},
    response::Html,
    routing::get,
};
use std::sync::Arc;

use crate::config::AppConfig;

const UI_BOOTSTRAP_JS: &str = include_str!("../crates/rustresort-ui/dist/rustresort_ui.js");
const UI_BOOTSTRAP_WASM: &[u8] =
    include_bytes!("../crates/rustresort-ui/dist/rustresort_ui_bg.wasm");
const UI_STYLESHEET: &str = include_str!("../crates/rustresort-ui/dist/app.css");

pub fn ui_router<S>(config: Arc<AppConfig>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let stylesheet_config = config.clone();
    let bootstrap_js_config = config.clone();
    let bootstrap_wasm_config = config;

    Router::new()
        .route("/ui", get(ui_shell))
        .route("/ui/", get(ui_shell))
        .route(
            "/ui/app.css",
            get(move || ui_stylesheet(stylesheet_config.clone())),
        )
        .route(
            "/ui/rustresort_ui.js",
            get(move || ui_bootstrap_js(bootstrap_js_config.clone())),
        )
        .route(
            "/ui/rustresort_ui_bg.wasm",
            get(move || ui_bootstrap_wasm(bootstrap_wasm_config.clone())),
        )
}

async fn ui_shell() -> Html<&'static str> {
    Html(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>RustResort UI</title>
  <link rel="stylesheet" href="/ui/app.css">
</head>
<body>
  <div id="app" class="shell-loading">Loading RustResort UI…</div>
  <script type="module">
    import init from "/ui/rustresort_ui.js";
    init("/ui/rustresort_ui_bg.wasm");
  </script>
</body>
</html>"#,
    )
}

async fn ui_stylesheet(config: Arc<AppConfig>) -> Response<Body> {
    asset_response(
        config.as_ref(),
        "app.css",
        "text/css; charset=utf-8",
        UI_STYLESHEET.as_bytes(),
    )
    .await
}

async fn ui_bootstrap_js(config: Arc<AppConfig>) -> Response<Body> {
    asset_response(
        config.as_ref(),
        "rustresort_ui.js",
        "text/javascript; charset=utf-8",
        UI_BOOTSTRAP_JS.as_bytes(),
    )
    .await
}

async fn ui_bootstrap_wasm(config: Arc<AppConfig>) -> Response<Body> {
    asset_response(
        config.as_ref(),
        "rustresort_ui_bg.wasm",
        "application/wasm",
        UI_BOOTSTRAP_WASM,
    )
    .await
}

async fn asset_response(
    config: &AppConfig,
    asset_name: &str,
    content_type: &'static str,
    fallback_bytes: &'static [u8],
) -> Response<Body> {
    if let Some(bytes) = read_dev_asset(config, asset_name).await {
        return binary_response(bytes, content_type);
    }

    binary_response(fallback_bytes.to_vec(), content_type)
}

async fn read_dev_asset(config: &AppConfig, asset_name: &str) -> Option<Vec<u8>> {
    let dev_dir = config.ui.dev_dir.as_ref()?;
    let path = dev_dir.join(asset_name);
    if !path.is_file() {
        return None;
    }

    tokio::fs::read(path).await.ok()
}

fn binary_response(body: Vec<u8>, content_type: &'static str) -> Response<Body> {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
}
