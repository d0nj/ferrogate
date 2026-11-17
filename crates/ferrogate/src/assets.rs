use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;
use serde_json::json;

#[derive(RustEmbed)]
#[folder = "../../apps/web/dist"]
#[allow_missing = true]
struct FrontendAssets;

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "no-cache";

pub async fn serve_spa(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    if is_reserved(path) {
        return not_found();
    }

    if !path.is_empty() {
        if let Some(asset) = FrontendAssets::get(path) {
            return file_response(path, asset.data.into_owned());
        }
    }

    if has_extension(path) {
        return not_found();
    }

    match FrontendAssets::get("index.html") {
        Some(asset) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, REVALIDATE)
            .body(Body::from(asset.data.into_owned()))
            .expect("valid static response"),
        None => (StatusCode::SERVICE_UNAVAILABLE, "frontend bundle not built").into_response(),
    }
}

fn file_response(path: &str, bytes: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path.starts_with("assets/") {
        IMMUTABLE
    } else {
        REVALIDATE
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(bytes))
        .expect("valid asset response")
}

fn is_reserved(path: &str) -> bool {
    let api = |prefix: &str| path == prefix || path.starts_with(&format!("{prefix}/"));
    api("v1") || api("admin") || path == "health"
}

fn has_extension(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'))
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        axum::Json(json!({
            "error": {
                "message": "route not found",
                "type": "not_found"
            }
        })),
    )
        .into_response()
}
