#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use std::sync::Arc;

    use axum::{Router, routing::post, extract::Request, response::IntoResponse};
    use serde_json::{json, Value};

    use super::super::adapter::{is_auth_failure, McpToolAdapter, ParallelSafeFn};
    use super::super::client::connect_http;
    use super::super::manager::{ServerEntry, MAX_CONSECUTIVE_AUTH_FAILURES};
    use super::super::types::{HttpAuthConfig, HttpServerConfig, ManagerServerConfig};
    use crate::tools::Tool;

    /// Fake MCP server that echoes the 'text' argument back.
    async fn fake_mcp_with_echo() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route("/", post(|req: Request| async move {
            let body = axum::body::to_bytes(req.into_body(), usize::MAX).await.unwrap_or_default();
            let envelope: Value = serde_json::from_slice(&body).unwrap_or_default();
            let method = envelope.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let id = envelope.get("id").cloned().unwrap_or(Value::Null);

            let resp = match method {
                "initialize" => json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "fake", "version": "0"},
                    }
                }),
                "tools/call" => {
                    let text = envelope
                        .pointer("/params/arguments/text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "content": [{"type": "text", "text": format!("echo: {}", text)}]
                        }
                    })
                }
                _ => return (axum::http::StatusCode::ACCEPTED, axum::body::Body::empty()).into_response(),
            };
            (axum::http::StatusCode::OK,
             [(axum::http::header::CONTENT_TYPE, "application/json")],
             axum::body::Body::from(serde_json::to_vec(&resp).unwrap())).into_response()
        }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        (format!("http://{}", addr), handle)
    }

    /// Fake MCP server that returns auth errors for the first N tools/calls,
    /// then switches to echo. call_count is incremented on every tools/call.
    async fn fake_mcp_with_flaky_auth(
        call_count: Arc<AtomicI32>,
        failures_before_ok: i32,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route("/", post(move |req: Request| {
            let call_count = call_count.clone();
            async move {
                let body = axum::body::to_bytes(req.into_body(), usize::MAX).await.unwrap_or_default();
                let envelope: Value = serde_json::from_slice(&body).unwrap_or_default();
                let method = envelope.get("method").and_then(|v| v.as_str()).unwrap_or("");
                let id = envelope.get("id").cloned().unwrap_or(Value::Null);

                let resp = match method {
                    "initialize" => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "protocolVersion": "2025-06-18",
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "fake", "version": "0"},
                        }
                    }),
                    "tools/call" => {
                        let n = call_count.fetch_add(1, Ordering::SeqCst) + 1;
                        if n <= failures_before_ok {
                            json!({
                                "jsonrpc": "2.0", "id": id,
                                "error": {"code": -32000, "message": "session not found"}
                            })
                        } else {
                            let text = envelope
                                .pointer("/params/arguments/text")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            json!({
                                "jsonrpc": "2.0", "id": id,
                                "result": {
                                    "content": [{"type": "text", "text": format!("echo: {}", text)}]
                                }
                            })
                        }
                    }
                    _ => return (axum::http::StatusCode::ACCEPTED, axum::body::Body::empty()).into_response(),
                };
                (axum::http::StatusCode::OK,
                 [(axum::http::header::CONTENT_TYPE, "application/json")],
                 axum::body::Body::from(serde_json::to_vec(&resp).unwrap())).into_response()
            }
        }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        (format!("http://{}", addr), handle)
    }

    fn make_entry(id: &str, client: super::super::client::Client, url: &str) -> Arc<ServerEntry> {
        Arc::new(ServerEntry {
            id: id.to_owned(),
            tool_prefix: String::new(),
            parallel_safe: false,
            client: parking_lot::RwLock::new(Some(Arc::new(client))),
            cfg: ManagerServerConfig {
                id: id.to_owned(),
                tool_prefix: String::new(),
                transport: "http".to_owned(),
                http: Some(HttpServerConfig {
                    url: url.to_owned(),
                    auth: HttpAuthConfig { kind: "none".to_owned(), ..Default::default() },
                }),
                stdio: None,
                parallel_safe: false,
            },
            consecutive_failures: parking_lot::Mutex::new(0),
        })
    }

    #[tokio::test]
    async fn test_adapter_execute() {
        let (url, _handle) = fake_mcp_with_echo().await;
        let client = connect_http(&url, reqwest::Client::new()).await.unwrap();
        let entry = make_entry("ltm", client, &url);
        let adapter = McpToolAdapter::new(
            "ltm_echo", "echo", "Echo back text",
            json!({"type":"object","properties":{"text":{"type":"string"}}}),
            entry, None,
        );

        assert_eq!(adapter.name(), "ltm_echo");
        assert_eq!(adapter.description(), "Echo back text");

        let result = tokio::task::spawn_blocking(move || {
            adapter.execute(json!({"text": "hi"}))
        }).await.unwrap().unwrap();

        assert_eq!(result.output, "echo: hi");
        assert!(result.error.is_empty());
    }

    #[tokio::test]
    async fn test_adapter_bad_input() {
        // entry with no client → "not connected"
        let entry = Arc::new(ServerEntry {
            id: "x".to_owned(),
            tool_prefix: String::new(),
            parallel_safe: false,
            client: parking_lot::RwLock::new(None),
            cfg: ManagerServerConfig {
                id: "x".to_owned(),
                tool_prefix: String::new(),
                transport: "http".to_owned(),
                http: None,
                stdio: None,
                parallel_safe: false,
            },
            consecutive_failures: parking_lot::Mutex::new(0),
        });
        let adapter = McpToolAdapter::new("x", "x", "", json!({}), entry, None);

        // Pass an invalid JSON string that cannot be parsed as arguments.
        // We test via the direct JSON value path: a string value that isn't valid JSON.
        // The adapter handles this gracefully as a tool error.
        let result = tokio::task::spawn_blocking(move || {
            adapter.execute(Value::String("not json".to_owned()))
        }).await.unwrap().unwrap();

        // Either "invalid arguments" or "not connected" — both are valid outcomes.
        assert!(!result.error.is_empty(), "should surface an error");
    }

    #[test]
    fn test_adapter_is_concurrency_safe_func_returns_live_value() {
        let current = Arc::new(AtomicBool::new(false));
        let current_clone = current.clone();
        let f: ParallelSafeFn = Arc::new(move |id: &str| {
            if id != "myserver" { return false; }
            current_clone.load(Ordering::SeqCst)
        });
        let entry = Arc::new(ServerEntry {
            id: "myserver".to_owned(),
            tool_prefix: String::new(),
            parallel_safe: false,
            client: parking_lot::RwLock::new(None),
            cfg: ManagerServerConfig {
                id: "myserver".to_owned(),
                tool_prefix: String::new(),
                transport: "http".to_owned(),
                http: None,
                stdio: None,
                parallel_safe: false,
            },
            consecutive_failures: parking_lot::Mutex::new(0),
        });
        let adapter = McpToolAdapter::new("x_t", "t", "", json!({}), entry, Some(f));

        assert!(!adapter.is_concurrency_safe(&Value::Null));
        current.store(true, Ordering::SeqCst);
        assert!(adapter.is_concurrency_safe(&Value::Null), "live read should pick up the toggle");
        current.store(false, Ordering::SeqCst);
        assert!(!adapter.is_concurrency_safe(&Value::Null));
    }

    #[test]
    fn test_adapter_is_concurrency_safe_nil_fn_returns_false() {
        let entry = Arc::new(ServerEntry {
            id: "anything".to_owned(),
            tool_prefix: String::new(),
            parallel_safe: false,
            client: parking_lot::RwLock::new(None),
            cfg: ManagerServerConfig {
                id: "anything".to_owned(),
                tool_prefix: String::new(),
                transport: "http".to_owned(),
                http: None,
                stdio: None,
                parallel_safe: false,
            },
            consecutive_failures: parking_lot::Mutex::new(0),
        });
        let adapter = McpToolAdapter::new("x_t", "t", "", json!({}), entry, None);
        assert!(!adapter.is_concurrency_safe(&Value::Null));
    }

    #[tokio::test]
    async fn test_adapter_execute_retries_after_auth_failure() {
        let call_count = Arc::new(AtomicI32::new(0));
        // Fail only the first tools/call; the post-reconnect retry succeeds.
        let (url, _handle) = fake_mcp_with_flaky_auth(call_count.clone(), 1).await;
        let client = connect_http(&url, reqwest::Client::new()).await.unwrap();
        let entry = make_entry("flaky", client, &url);
        let adapter = McpToolAdapter::new(
            "flaky_echo", "echo", "Echo back text",
            json!({"type":"object","properties":{"text":{"type":"string"}}}),
            entry, None,
        );

        let result = tokio::task::spawn_blocking(move || {
            adapter.execute(json!({"text": "hi"}))
        }).await.unwrap().unwrap();

        assert_eq!(result.output, "echo: hi", "retry should succeed after reconnect");
        assert!(result.error.is_empty(), "retry success must not surface an error");
        assert!(result.metadata.is_none(), "retry success must not stamp auth_required");
        assert!(call_count.load(Ordering::SeqCst) >= 2, "expected at least original + retry calls");
    }

    #[tokio::test]
    async fn test_adapter_execute_auth_failure_surfaces_when_retry_also_fails() {
        let call_count = Arc::new(AtomicI32::new(0));
        let (url, _handle) = fake_mcp_with_flaky_auth(call_count.clone(), i32::MAX).await;
        let client = connect_http(&url, reqwest::Client::new()).await.unwrap();
        let entry = make_entry("flaky", client, &url);
        let adapter = McpToolAdapter::new(
            "flaky_echo", "echo", "", json!({}), entry, None,
        );

        let result = tokio::task::spawn_blocking(move || {
            adapter.execute(json!({"text": "hi"}))
        }).await.unwrap().unwrap();

        assert!(!result.error.is_empty(), "persistent auth failure must surface as tool error");
        assert!(result.metadata.is_some(), "auth_required metadata must be set");
        let meta = result.metadata.unwrap();
        assert_eq!(meta.get("auth_required").and_then(|v| v.as_str()), Some("flaky"));
        assert!(call_count.load(Ordering::SeqCst) >= 2, "should attempt original + at least one retry");
    }

    #[tokio::test]
    async fn test_adapter_circuit_breaker_trips_after_max_consecutive_failures() {
        let call_count = Arc::new(AtomicI32::new(0));
        let (url, _handle) = fake_mcp_with_flaky_auth(call_count.clone(), i32::MAX).await;
        let client = connect_http(&url, reqwest::Client::new()).await.unwrap();
        let entry = make_entry("tripped", client, &url);
        let entry_clone = entry.clone();
        let url_clone = url.clone();

        let adapter = Arc::new(McpToolAdapter::new(
            "tripped_echo", "echo", "", json!({}), entry, None,
        ));

        // Drive the breaker to its trip threshold.
        for _ in 0..MAX_CONSECUTIVE_AUTH_FAILURES {
            let adapter = adapter.clone();
            let res = tokio::task::spawn_blocking(move || {
                adapter.execute(json!({"text": "hi"}))
            }).await.unwrap().unwrap();
            assert!(!res.error.is_empty());
            assert!(res.metadata.is_some());
            assert_eq!(
                res.metadata.as_ref().and_then(|m| m.get("auth_required")).and_then(|v| v.as_str()),
                Some("tripped")
            );
        }

        let pre_trip_calls = call_count.load(Ordering::SeqCst);
        assert_eq!(entry_clone.failure_count(), MAX_CONSECUTIVE_AUTH_FAILURES);

        // Next call must short-circuit: no network, strong "stop calling" message.
        let adapter_final = adapter.clone();
        let res = tokio::task::spawn_blocking(move || {
            adapter_final.execute(json!({"text": "hi"}))
        }).await.unwrap().unwrap();

        assert!(res.metadata.is_some());
        let meta = res.metadata.unwrap();
        assert_eq!(meta.get("auth_required").and_then(|v| v.as_str()), Some("tripped"));
        assert_eq!(meta.get("circuit_breaker").and_then(|v| v.as_bool()), Some(true));
        assert!(
            res.error.to_lowercase().contains("stop calling tools from this server"),
            "short-circuit message should instruct agent to stop, got: {}", res.error
        );
        assert_eq!(call_count.load(Ordering::SeqCst), pre_trip_calls, "short-circuit must not touch network");
    }

    #[test]
    fn test_server_entry_record_success_resets_breaker() {
        let entry = ServerEntry {
            id: "x".to_owned(),
            tool_prefix: String::new(),
            parallel_safe: false,
            client: parking_lot::RwLock::new(None),
            cfg: ManagerServerConfig {
                id: "x".to_owned(),
                tool_prefix: String::new(),
                transport: "http".to_owned(),
                http: None,
                stdio: None,
                parallel_safe: false,
            },
            consecutive_failures: parking_lot::Mutex::new(0),
        };
        assert_eq!(entry.failure_count(), 0);
        assert_eq!(entry.record_failure(), 1);
        assert_eq!(entry.record_failure(), 2);
        assert_eq!(entry.failure_count(), 2);
        entry.record_success();
        assert_eq!(entry.failure_count(), 0, "successful call must reset the breaker");
    }

    #[test]
    fn test_server_entry_reset_failures_private_method_resets_breaker() {
        let entry = ServerEntry {
            id: "x".to_owned(),
            tool_prefix: String::new(),
            parallel_safe: false,
            client: parking_lot::RwLock::new(None),
            cfg: ManagerServerConfig {
                id: "x".to_owned(),
                tool_prefix: String::new(),
                transport: "http".to_owned(),
                http: None,
                stdio: None,
                parallel_safe: false,
            },
            consecutive_failures: parking_lot::Mutex::new(0),
        };
        for _ in 0..MAX_CONSECUTIVE_AUTH_FAILURES + 5 {
            entry.record_failure();
        }
        assert!(entry.failure_count() >= MAX_CONSECUTIVE_AUTH_FAILURES);
        entry.reset_failures();
        assert_eq!(entry.failure_count(), 0, "manual reconnect path must reset the breaker");
    }
}