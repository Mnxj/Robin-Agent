#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{Router, routing::post, extract::Request, response::IntoResponse};
    use serde_json::{json, Value};

    use super::super::manager::Manager;
    use super::super::types::{HttpAuthConfig, HttpServerConfig, ManagerServerConfig};

    async fn fake_mcp_with_tools(tools: Vec<Value>) -> (String, tokio::task::JoinHandle<()>) {
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

    async fn fake_token_server() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route("/token", post(|_req: Request| async move {
            let resp = json!({
                "access_token": "tok-abc",
                "token_type": "Bearer",
                "expires_in": 3600,
            });
            (axum::http::StatusCode::OK,
             [(axum::http::header::CONTENT_TYPE, "application/json")],
             axum::Json(resp)).into_response()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        (format!("http://{}", addr), handle)
    }

    fn http_cfg_with_oauth(url: &str, token_url: &str) -> ManagerServerConfig {
        ManagerServerConfig {
            id: String::new(),
            tool_prefix: String::new(),
            transport: "http".to_owned(),
            http: Some(HttpServerConfig {
                url: url.to_owned(),
                auth: HttpAuthConfig {
                    kind: "oauth2_client_credentials".to_owned(),
                    token_url: token_url.to_owned(),
                    client_id: "cid".to_owned(),
                    client_secret: "sec".to_owned(),
                    ..Default::default()
                },
            }),
            stdio: None,
            parallel_safe: false,
        }
    }

    #[tokio::test]
    async fn test_manager_opens_all_enabled_servers() {
        let (url_a, _ha) = fake_mcp_with_tools(vec![
            json!({"name": "a_tool", "description": "from A", "inputSchema": {"type": "object"}}),
        ]).await;
        let (url_b, _hb) = fake_mcp_with_tools(vec![
            json!({"name": "b_tool", "description": "from B", "inputSchema": {"type": "object"}}),
        ]).await;
        let (tok_url, _ht) = fake_token_server().await;

        let mut cfg_a = http_cfg_with_oauth(&url_a, &format!("{}/token", tok_url));
        cfg_a.id = "a".to_owned();
        cfg_a.tool_prefix = "a_".to_owned();

        let mut cfg_b = http_cfg_with_oauth(&url_b, &format!("{}/token", tok_url));
        cfg_b.id = "b".to_owned();

        let mgr = Manager::new(vec![cfg_a, cfg_b]).await.unwrap();
        let servers = mgr.servers();
        assert_eq!(servers.len(), 2);

        let ids: Vec<&str> = servers.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"a"), "expected server 'a'");
        assert!(ids.contains(&"b"), "expected server 'b'");

        mgr.close().unwrap();
    }

    #[tokio::test]
    async fn test_manager_skips_unreachable_server() {
        let (url_ok, _h) = fake_mcp_with_tools(vec![
            json!({"name": "ok", "description": "alive", "inputSchema": {"type": "object"}}),
        ]).await;
        let (tok_url, _ht) = fake_token_server().await;

        let mut cfg_ok = http_cfg_with_oauth(&url_ok, &format!("{}/token", tok_url));
        cfg_ok.id = "ok".to_owned();

        let mut cfg_dead = http_cfg_with_oauth("http://127.0.0.1:1/closed", &format!("{}/token", tok_url));
        cfg_dead.id = "dead".to_owned();

        let mgr = Manager::new(vec![cfg_ok, cfg_dead]).await.unwrap();
        let servers = mgr.servers();
        assert_eq!(servers.len(), 1, "dead server should be skipped");
        assert_eq!(servers[0].id, "ok");

        mgr.close().unwrap();
    }
}