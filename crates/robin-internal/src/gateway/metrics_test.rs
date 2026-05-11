use super::*;
use std::sync::Arc;

#[tokio::test]
async fn test_metrics_handler() {
    let m = Arc::new(Metrics::new());

    // Increment some counters
    m.inc_requests();
    m.inc_requests();
    m.inc_ws_connections();
    m.inc_ws_messages();
    m.inc_tool_calls("bash");
    m.inc_tool_calls("bash");
    m.inc_tool_calls("read_file");
    m.inc_llm_calls();
    m.inc_errors();

    let handler = m.handler();
    let response = handler().await;
    assert_eq!(response.status(), 200);

    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(content_type.starts_with("text/plain"));

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();

    assert!(body.contains("robin_http_requests_total 2"));
    assert!(body.contains("robin_ws_connections_active 1"));
    assert!(body.contains("robin_ws_messages_total 1"));
    assert!(body.contains("robin_tool_calls_total 3"));
    assert!(body.contains("robin_llm_calls_total 1"));
    assert!(body.contains("robin_errors_total 1"));
    assert!(body.contains(r#"robin_tool_calls_by_tool{tool="bash"} 2"#));
    assert!(body.contains(r#"robin_tool_calls_by_tool{tool="read_file"} 1"#));
    assert!(body.contains("robin_uptime_seconds"));
}

#[tokio::test]
async fn test_metrics_dec_ws_connections() {
    let m = Arc::new(Metrics::new());
    m.inc_ws_connections();
    m.inc_ws_connections();
    m.dec_ws_connections();

    let handler = m.handler();
    let response = handler().await;
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body.contains("robin_ws_connections_active 1"));
}