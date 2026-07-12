use axum::{
    body::Body,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct FrontendAssets;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if is_api_path(path) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let asset_path = if path.is_empty() { "index.html" } else { path };
    if let Some(asset) = FrontendAssets::get(asset_path) {
        return asset_response(asset_path, asset.data.into_owned());
    }

    if !asset_path
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .contains('.')
    {
        if let Some(index) = FrontendAssets::get("index.html") {
            return asset_response("index.html", index.data.into_owned());
        }
    }

    StatusCode::NOT_FOUND.into_response()
}

fn is_api_path(path: &str) -> bool {
    ["api", "v1", "responses"]
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

fn asset_response(path: &str, body: Vec<u8>) -> Response {
    let mut content_type = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();
    if content_type.starts_with("text/") || content_type == "application/javascript" {
        content_type.push_str("; charset=utf-8");
    }
    let cache_control = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache_control)
        .header("x-content-type-options", "nosniff")
        .body(Body::from(body))
        .expect("static asset response headers are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serves_index_for_spa_routes() {
        let response = serve(Uri::from_static("/settings")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );
    }

    #[tokio::test]
    async fn does_not_fallback_for_unknown_api_routes() {
        let response = serve(Uri::from_static("/api/not-a-route")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn missing_assets_return_not_found() {
        let response = serve(Uri::from_static("/assets/missing.js")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
