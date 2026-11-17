//! End-to-end coverage of the embedded SPA fallback served by `build_app`.

use std::{net::SocketAddr, path::PathBuf};

use axum::Router;
use ferrogate::{config::AppConfig, db, AppState};
use serde_json::{json, Value};

/// A running app over a fresh database, including the SPA fallback.
struct Harness {
    base_url: String,
    _dir: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self._dir);
    }
}

impl Harness {
    async fn start() -> Self {
        std::env::set_var(
            "FERROGATE_ENCRYPTION_KEY",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        );
        let dir = std::env::temp_dir().join(format!("ferrogate-spa-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            dir.join("spa.db").display().to_string().replace('\\', "/")
        );

        let db = db::init_db(&database_url)
            .await
            .expect("bootstrap database");
        let (state, worker) = AppState::from_db_and_config(db, AppConfig::default()).await;
        drop(worker);

        let app: Router = ferrogate::build_app(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            base_url: format!("http://{addr}"),
            _dir: dir,
        }
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        reqwest::Client::new()
            .get(format!("{}{path}", self.base_url))
            .send()
            .await
            .unwrap()
    }

    async fn get_text(&self, path: &str) -> (reqwest::StatusCode, String, String) {
        let response = self.get(path).await;
        let status = response.status();
        let content_type = header(&response, "content-type");
        let cache = header(&response, "cache-control");
        let body = response.text().await.unwrap_or_default();
        (status, body, format!("{content_type}|{cache}"))
    }
}

fn header(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// Pull the first `/assets/*.js` or `.css` reference out of the shell HTML.
fn asset_reference(html: &str) -> String {
    let at = html
        .find("/assets/")
        .unwrap_or_else(|| panic!("no /assets/ reference in HTML: {html}"));

    let rest = &html[at..];
    let end = rest
        .find(['"', '\'', ')'])
        .unwrap_or_else(|| panic!("unterminated asset reference: {rest}"));

    rest[..end].to_owned()
}

#[tokio::test]
async fn spa_routes_serve_the_shell_html() {
    let harness = Harness::start().await;

    for route in ["/", "/keys", "/routing", "/logs", "/playground"] {
        let response = harness.get(route).await;
        assert_eq!(response.status(), 200, "route {route}");
        let content_type = header(&response, "content-type");
        assert!(
            content_type.starts_with("text/html"),
            "route {route} content-type: {content_type}"
        );
        assert_eq!(
            header(&response, "cache-control"),
            "no-cache",
            "route {route}"
        );
        let body = response.text().await.unwrap();
        assert!(
            body.contains("<div id=\"root\">"),
            "route {route} missing root element"
        );
    }
}

#[tokio::test]
async fn asset_bundle_is_served_immutable_with_real_mime() {
    let harness = Harness::start().await;
    let (_, html, _) = harness.get_text("/").await;

    let asset = asset_reference(&html);
    let response = harness.get(&asset).await;
    assert_eq!(response.status(), 200, "asset {asset}");
    assert_eq!(
        header(&response, "cache-control"),
        "public, max-age=31536000, immutable",
        "asset {asset}"
    );

    let content_type = header(&response, "content-type");
    let is_expected = if asset.ends_with(".css") {
        content_type.starts_with("text/css")
    } else {
        content_type.contains("javascript")
    };
    assert!(is_expected, "asset {asset} content-type: {content_type}");
    assert!(
        !response.bytes().await.unwrap().is_empty(),
        "asset {asset} empty"
    );
}

#[tokio::test]
async fn missing_static_file_is_not_answered_with_html() {
    let harness = Harness::start().await;

    let response = harness.get("/assets/does-not-exist.js").await;
    assert_eq!(response.status(), 404);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "not_found");
}

#[tokio::test]
async fn api_prefixes_are_never_served_the_shell() {
    let harness = Harness::start().await;

    for path in ["/v1/nonexistent", "/admin/api/nonexistent"] {
        let (status, body, _) = harness.get_text(path).await;
        assert_eq!(status, 404, "path {path}");
        assert!(
            !body.contains("<div id=\"root\">"),
            "path {path} served HTML"
        );
        let parsed: Value = serde_json::from_str(&body).expect("json body");
        assert_eq!(parsed["error"]["type"], "not_found", "path {path}");
    }
}

#[tokio::test]
async fn health_endpoint_still_reports_json() {
    let harness = Harness::start().await;

    let response = harness.get("/health").await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body, json!({ "status": "ok" }));
}

#[tokio::test]
async fn metrics_endpoint_requires_an_operator_session() {
    let harness = Harness::start().await;

    let response = harness.get("/metrics").await;
    assert_eq!(
        response.status(),
        401,
        "/metrics must not leak per-key spend"
    );
}
