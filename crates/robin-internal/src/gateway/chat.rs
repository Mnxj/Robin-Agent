use axum::{
    http::HeaderValue,
    response::{Html, IntoResponse, Response},
};

/// Returns an axum handler that serves the chat web interface.
pub async fn chat_handler(port: u16) -> impl IntoResponse {
    let html = CHAT_HTML.replace("{PORT}", &port.to_string());

    let mut response = Html(html).into_response();
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src \'self\'; script-src \'unsafe-inline\'; style-src \'unsafe-inline\'; connect-src \'self\' ws: wss:; img-src \'self\' data:",
        ),
    );
    response
}

pub const CHAT_HTML: &str = include_str!("../../../../web/dist/index.html");
