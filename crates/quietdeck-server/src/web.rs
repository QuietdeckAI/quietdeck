//! The built web app (web/dist), embedded in the binary, or a frontend of your own from QUIETDECK_UI_DIR. Any path
//! that isn't a file gets index.html, so the app's own routes work on reload.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

#[derive(rust_embed::Embed)]
#[folder = "../../web/dist/"]
#[allow_missing = true]
struct Assets;

fn not_an_endpoint(uri: &Uri) -> Option<Response> {
    let path = uri.path().trim_start_matches('/');
    (path == "api" || path.starts_with("api/") || path.starts_with("hooks/"))
        .then(|| crate::error::AppError::NotFound(format!("no endpoint {}", uri.path())).into_response())
}

fn file_response(path: &str, data: impl IntoResponse) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    // Vite's hashed assets never change; everything else is checked each time.
    let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    ([(header::CONTENT_TYPE, mime.as_ref()), (header::CACHE_CONTROL, cache)], data).into_response()
}

const NO_UI: &str = "Quietdeck's deck is running. This build has no web app (web/dist was empty).";

/// A frontend of your own from a directory (QUIETDECK_UI_DIR): its files, and its index.html for any other path.
pub async fn serve_dir(dir: Arc<PathBuf>, uri: Uri) -> Response {
    if let Some(res) = not_an_endpoint(&uri) {
        return res;
    }
    let path = uri.path().trim_start_matches('/');
    // Only plain names inside the directory: no `..`, no absolute paths.
    let safe = Path::new(path).components().all(|c| matches!(c, Component::Normal(_)));
    if safe && !path.is_empty() {
        let file = dir.join(path);
        if let Ok(data) = tokio::fs::read(&file).await {
            return file_response(path, data);
        }
    }
    match tokio::fs::read(dir.join("index.html")).await {
        Ok(index) => file_response("index.html", index),
        Err(_) => {
            (StatusCode::OK, "Quietdeck's deck is running, with no web app (QUIETDECK_UI_DIR has no index.html).")
                .into_response()
        }
    }
}

pub async fn serve(uri: Uri) -> Response {
    if let Some(res) = not_an_endpoint(&uri) {
        return res;
    }
    let path = uri.path().trim_start_matches('/');
    if let Some(file) = Assets::get(path).filter(|_| !path.is_empty()) {
        return file_response(path, file.data);
    }
    match Assets::get("index.html") {
        Some(index) => {
            ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], index.data)
                .into_response()
        }
        None => (StatusCode::OK, NO_UI).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unknown_api_paths_are_404_not_the_app() {
        let res = serve("/api/nope".parse().unwrap()).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let res = serve("/settings".parse().unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    async fn body(res: Response) -> String {
        use http_body_util::BodyExt;
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn a_ui_dir_serves_its_files_and_its_index_and_nothing_outside() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("ui");
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("index.html"), "<h1>mine</h1>").unwrap();
        std::fs::write(dir.join("assets/app.js"), "console.log(1)").unwrap();
        std::fs::write(tmp.path().join("secret.txt"), "no").unwrap();
        let dir = Arc::new(dir);
        let get = |p: &str| serve_dir(dir.clone(), p.parse().unwrap());
        let js = get("/assets/app.js").await;
        assert!(js.headers()[header::CACHE_CONTROL].to_str().unwrap().contains("immutable"));
        assert_eq!(body(js).await, "console.log(1)");
        assert_eq!(body(get("/inbox/42").await).await, "<h1>mine</h1>");
        assert_eq!(body(get("/../secret.txt").await).await, "<h1>mine</h1>");
        assert_eq!(body(get("/assets/../../secret.txt").await).await, "<h1>mine</h1>");
        assert_eq!(get("/api/v1/nope").await.status(), StatusCode::NOT_FOUND);
    }
}
