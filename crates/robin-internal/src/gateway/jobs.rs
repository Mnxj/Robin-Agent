use axum::response::{Html, IntoResponse, Response};

/// Returns an axum response that serves the cron jobs management page.
pub async fn jobs_page_handler(port: u16) -> impl IntoResponse {
    let html = JOBS_HTML.replace("{PORT}", &port.to_string());
    Html(html)
}

/// Returns an axum handler function (closure) bound to the given port.
pub fn new_jobs_handler(
    port: u16,
) -> impl Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>>
       + Clone
       + Send
       + 'static {
    move || {
        let p = port;
        Box::pin(async move { jobs_page_handler(p).await.into_response() })
    }
}

const JOBS_HTML: &str = include_str!("jobs_template.html");