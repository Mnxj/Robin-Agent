use axum::{
    body::Body,
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use std::{collections::HashSet, sync::Arc};

/// Bearer auth middleware factory.
/// Returns a closure usable as an axum middleware layer.
/// If `token` is empty, the middleware is a no-op (no auth required).
pub fn bearer_auth_middleware(
    token: String,
) -> impl Fn(Request, Next) -> std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>>
       + Clone
       + Send
       + 'static {
    move |req: Request, next: Next| {
        let token = token.clone();
        Box::pin(async move {
            if token.is_empty() {
                return next.run(req).await;
            }

            // Allow health check without auth
            if req.uri().path() == "/health" {
                return next.run(req).await;
            }

            // Check Authorization header
            let auth_header = req
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            let auth = match auth_header {
                Some(h) if !h.is_empty() => h,
                _ => {
                    // Also check query parameter for WebSocket clients that
                    // can't set custom headers
                    let query = req.uri().query().unwrap_or("");
                    let token_param = form_urlencoded_token(query);
                    match token_param {
                        Some(t) => format!("Bearer {}", t),
                        None => String::new(),
                    }
                }
            };

            if !auth.starts_with("Bearer ") {
                return unauthorized_response();
            }

            let provided_token = auth.trim_start_matches("Bearer ");

            // Constant-time comparison to prevent timing attacks
            if !constant_time_eq(provided_token.as_bytes(), token.as_bytes()) {
                return unauthorized_response();
            }

            next.run(req).await
        })
    }
}

fn form_urlencoded_token(query: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        if let (Some(key), Some(val)) = (parts.next(), parts.next()) {
            if key == "token" {
                return Some(percent_decode(val));
            }
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    // Simple percent decoding for the token query param
    s.replace('+', " ")
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

fn unauthorized_response() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"error":"unauthorized"}"#))
        .unwrap()
}

/// Returns a WebSocket CheckOrigin function that validates the request
/// origin against a list of allowed origins.
/// If no origins are configured, it defaults to allowing localhost only.
pub fn allowed_origins(
    origins: Vec<String>,
) -> Arc<dyn Fn(&axum::http::HeaderMap) -> bool + Send + Sync + 'static> {
    if origins.is_empty() {
        // Default: allow localhost origins only
        return Arc::new(|headers: &axum::http::HeaderMap| {
            let origin = headers
                .get(header::ORIGIN)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");

            if origin.is_empty() {
                return true; // no origin header (e.g. CLI tools, curl)
            }

            origin.starts_with("http://127.0.0.1")
                || origin.starts_with("http://localhost")
                || origin.starts_with("https://127.0.0.1")
                || origin.starts_with("https://localhost")
        });
    }

    let allowed: HashSet<String> = origins
        .into_iter()
        .map(|o| o.trim_end_matches('/').to_string())
        .collect();

    Arc::new(move |headers: &axum::http::HeaderMap| {
        let origin = headers
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if origin.is_empty() {
            return true;
        }

        allowed.contains(origin.trim_end_matches('/'))
    })
}

#[cfg(test)]
#[path = "auth_test.rs"]
mod auth_test;