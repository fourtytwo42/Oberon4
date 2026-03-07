use super::*;
use axum::response::Html;
const CONSOLE_HTML: &str = include_str!("../assets/console.html");

pub(crate) async fn console_ui() -> impl IntoResponse {
    Html(CONSOLE_HTML)
}

pub(crate) async fn console_logo() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        include_bytes!("../assets/logo.png").as_slice(),
    )
}

pub(crate) async fn console_logo_dark() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        include_bytes!("../../../dark-logo.png").as_slice(),
    )
}

pub(crate) async fn console_logo_light() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        include_bytes!("../../../light-logo.png").as_slice(),
    )
}

pub(crate) async fn console_logo_hover_dark() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        include_bytes!("../../../logo-hover-black.png").as_slice(),
    )
}

pub(crate) async fn console_logo_hover_light() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        include_bytes!("../../../logo-hover-white.png").as_slice(),
    )
}

pub(crate) async fn console_google_logo() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/webp"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        include_bytes!("../../../google-white-icon.webp").as_slice(),
    )
}

const APP_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("GIT_HASH"));

pub(crate) async fn version_info() -> impl IntoResponse {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "git_hash": env!("GIT_HASH"),
        "build": APP_VERSION,
    }))
}
