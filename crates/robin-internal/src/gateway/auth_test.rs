use super::*;
use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    middleware,
    response::IntoResponse,
    routing::get,
    Router,
};
use tower::ServiceExt;

async fn ok_handler() -> impl IntoResponse {
    StatusCode::OK
}

fn make_app(token: &str) -> Router {
    let token = token.to_string();
    let mw = middleware::from_fn(move |req: Request<Body>, next: middleware::Next| {
        let f = bearer_auth_middleware(token.clone());
        f(req, next)
    });
    Router::new()
        .route("/ws", get(ok_handler))
        .route("/health", get(ok_handler))
        .layer(mw)
}

#[tokio::test]
async fn test_bearer_auth_middleware_no_token() {
    // When no token is configured, everything is allowed
    let app = make_app("");
    let req = Request::builder()
        .uri("/ws")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_bearer_auth_middleware_valid_token() {
    let app = make_app("secret123");
    let req = Request::builder()
        .uri("/ws")
        .header(header::AUTHORIZATION, "Bearer secret123")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_bearer_auth_middleware_invalid_token() {
    let app = make_app("secret123");
    let req = Request::builder()
        .uri("/ws")
        .header(header::AUTHORIZATION, "Bearer wrong")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_bearer_auth_middleware_missing_header() {
    let app = make_app("secret123");
    let req = Request::builder()
        .uri("/ws")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_bearer_auth_middleware_health_bypass() {
    let app = make_app("secret123");
    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_bearer_auth_middleware_query_param() {
    let app = make_app("secret123");
    let req = Request::builder()
        .uri("/ws?token=secret123")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[test]
fn test_allowed_origins_default() {
    let check = allowed_origins(vec![]);

    // Localhost origins should pass
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://127.0.0.1:18789"),
    );
    assert!(check(&headers));

    let mut headers = HeaderMap::new();
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://localhost:3000"),
    );
    assert!(check(&headers));

    // External origins should fail
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://evil.com"),
    );
    assert!(!check(&headers));

    // No origin header should pass (CLI clients)
    let headers = HeaderMap::new();
    assert!(check(&headers));
}

#[test]
fn test_allowed_origins_custom() {
    let check = allowed_origins(vec![
        "https://mydomain.com".to_string(),
        "http://localhost:3000".to_string(),
    ]);

    let mut headers = HeaderMap::new();
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("https://mydomain.com"),
    );
    assert!(check(&headers));

    let mut headers = HeaderMap::new();
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("https://evil.com"),
    );
    assert!(!check(&headers));
}