#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use axum::{Router, routing::post, extract::Request, response::IntoResponse};
    use serde_json::{json, Value};

    use super::super::adapter::ParallelSafeFn;
    use super::super::client::connect_http;
    use super::super::manager::{Manager, ServerEntry};
    use super::super::register::register_tools;
    use super::super::types::{HttpAuthConfig, HttpServerConfig, ManagerServerConfig};
    use crate::tools::{Registry, Tool};

    async fn fake_mcp_with_tools_axum(tools: Vec<Value>) -> (String, tokio::task::JoinHandle<()>) {
        let tools = Arc::new(tools);
        let app = Router::new().route("/", post(move |req: Request| {
            let tools = tools.clone();
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
                    "tools/list" => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {"tools": *tools}
                    }),
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

    fn make_manager_with_entry(id: &str, tool_prefix: &str, client: super::super::client::Client) -> Manager {
        Manager {
            servers: vec![Arc::new(ServerEntry {
                id: id.to_owned(),
                tool_prefix: tool_prefix.to_owned(),
                parallel_safe: false,
                client: parking_lot::RwLock::new(Some(Arc::new(client))),
                cfg: ManagerServerConfig {
                    id: id.to_owned(),
                    tool_prefix: tool_prefix.to_owned(),
                    transport: "http".to_owned(),
                    http: Some(HttpServerConfig {
                        url: String::new(),
                        auth: HttpAuthConfig { kind: "none".to_owned(), ..Default::default() },
                    }),
                    stdio: None,
                    parallel_safe: false,
                },
                consecutive_failures: parking_lot::Mutex::new(0),
            })],
        }
    }

    #[tokio::test]
    async fn test_register_tools_adds_prefixed_adapters() {
        let (url, _h) = fake_mcp_with_tools_axum(vec![
            json!({"name": "search", "description": "search", "inputSchema": {"type": "object"}}),
            json!({"name": "store", "description": "store", "inputSchema": {"type": "object"}}),
        ]).await;

        let client = connect_http(&url, reqwest::Client::new()).await.unwrap();
        let mgr = make_manager_with_entry("ltm", "ltm_", client);

        let reg = Registry::new();
        let mut names = register_tools(&reg, &mgr, None).await.unwrap();
        names.sort();

        let mut expected = vec!["ltm_search".to_owned(), "ltm_store".to_owned()];
        expected.sort();
        assert_eq!(names, expected);

        let mut reg_names = reg.names();
        reg_names.sort();
        assert_eq!(reg_names, expected);
    }

    #[tokio::test]
    async fn test_register_tools_no_prefix_no_collision() {
        let (url, _h) = fake_mcp_with_tools_axum(vec![
            json!({"name": "remote_only", "description": "x", "inputSchema": {"type": "object"}}),
        ]).await;

        let client = connect_http(&url, reqwest::Client::new()).await.unwrap();
        let mgr = make_manager_with_entry("x", "", client);

        let reg = Registry::new();
        let names = register_tools(&reg, &mgr, None).await.unwrap();
        assert_eq!(names, vec!["remote_only"]);
        assert_eq!(reg.names(), vec!["remote_only"]);
    }

    #[tokio::test]
    async fn test_register_tools_propagates_parallel_safe() {
        let (url_safe, _hs) = fake_mcp_with_tools_axum(vec![
            json!({"name": "search", "description": "search", "inputSchema": {"type": "object"}}),
        ]).await;
        let (url_unsafe, _hu) = fake_mcp_with_tools_axum(vec![
            json!({"name": "store", "description": "store", "inputSchema": {"type": "object"}}),
        ]).await;

        let c_safe = connect_http(&url_safe, reqwest::Client::new()).await.unwrap();
        let c_unsafe = connect_http(&url_unsafe, reqwest::Client::new()).await.unwrap();

        let mgr = Manager {
            servers: vec![
                Arc::new(ServerEntry {
                    id: "safe".to_owned(),
                    tool_prefix: "s_".to_owned(),
                    parallel_safe: true,
                    client: parking_lot::RwLock::new(Some(Arc::new(c_safe))),
                    cfg: ManagerServerConfig {
                        id: "safe".to_owned(), tool_prefix: "s_".to_owned(),
                        transport: "http".to_owned(), http: None, stdio: None, parallel_safe: true,
                    },
                    consecutive_failures: parking_lot::Mutex::new(0),
                }),
                Arc::new(ServerEntry {
                    id: "unsafe".to_owned(),
                    tool_prefix: "u_".to_owned(),
                    parallel_safe: false,
                    client: parking_lot::RwLock::new(Some(Arc::new(c_unsafe))),
                    cfg: ManagerServerConfig {
                        id: "unsafe".to_owned(), tool_prefix: "u_".to_owned(),
                        transport: "http".to_owned(), http: None, stdio: None, parallel_safe: false,
                    },
                    consecutive_failures: parking_lot::Mutex::new(0),
                }),
            ],
        };

        let parallel_safe: ParallelSafeFn = Arc::new(|id: &str| id == "safe");

        let reg = Registry::new();
        register_tools(&reg, &mgr, Some(parallel_safe)).await.unwrap();

        let safe_tool = reg.get("s_search").unwrap();
        let unsafe_tool = reg.get("u_store").unwrap();
        assert!(safe_tool.is_concurrency_safe(&Value::Null), "tool from parallel-safe server should be safe");
        assert!(!unsafe_tool.is_concurrency_safe(&Value::Null), "tool from default server should be unsafe");
    }

    #[tokio::test]
    async fn test_register_tools_live_read_picks_up_toggle() {
        let (url, _h) = fake_mcp_with_tools_axum(vec![
            json!({"name": "search", "description": "search", "inputSchema": {"type": "object"}}),
        ]).await;

        let client = connect_http(&url, reqwest::Client::new()).await.unwrap();
        let mgr = make_manager_with_entry("trusted", "t_", client);

        let live = Arc::new(AtomicBool::new(false));
        let live_clone = live.clone();
        let fn_ps: ParallelSafeFn = Arc::new(move |id: &str| id == "trusted" && live_clone.load(Ordering::SeqCst));

        let reg = Registry::new();
        register_tools(&reg, &mgr, Some(fn_ps)).await.unwrap();

        let tool = reg.get("t_search").unwrap();
        assert!(!tool.is_concurrency_safe(&Value::Null), "default state is unsafe");

        live.store(true, Ordering::SeqCst);
        assert!(tool.is_concurrency_safe(&Value::Null), "toggle should be visible on next call");

        live.store(false, Ordering::SeqCst);
        assert!(!tool.is_concurrency_safe(&Value::Null), "untoggle should also be live-read");
    }

    #[tokio::test]
    async fn test_register_tools_collision_fails() {
        let (url, _h) = fake_mcp_with_tools_axum(vec![
            json!({"name": "bash", "description": "fake bash", "inputSchema": {"type": "object"}}),
        ]).await;

        let client = connect_http(&url, reqwest::Client::new()).await.unwrap();
        let mgr = make_manager_with_entry("x", "", client);

        let reg = Registry::new();
        // Manually register a tool named "bash" to create a collision.
        use super::super::adapter::McpToolAdapter;
        let dummy_entry = Arc::new(ServerEntry {
            id: "core".to_owned(), tool_prefix: String::new(), parallel_safe: false,
            client: parking_lot::RwLock::new(None),
            cfg: ManagerServerConfig {
                id: "core".to_owned(), tool_prefix: String::new(), transport: "http".to_owned(),
                http: None, stdio: None, parallel_safe: false,
            },
            consecutive_failures: parking_lot::Mutex::new(0),
        });
        reg.register(McpToolAdapter::new("bash", "bash", "fake", json!({}), dummy_entry, None));

        let result = register_tools(&reg, &mgr, None).await;
        assert!(result.is_err(), "collision should return error");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("bash"), "error should mention 'bash', got: {}", err);
        assert!(err.contains("collision"), "error should mention 'collision', got: {}", err);
    }
}