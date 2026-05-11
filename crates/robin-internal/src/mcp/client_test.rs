#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use axum::{Router, routing::post, extract::Request, response::IntoResponse};
    use serde_json::Value;

    use super::super::client::{connect_http, ToolInfo};

    /// Start a minimal fake MCP server that handles initialize and tools/list.
    async fn fake_mcp_server() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route("/", post(|req: Request| async move {
            let body = axum::body::to_bytes(req.into_body(), usize::MAX).await.unwrap_or_default();
            let envelope: Value = serde_json::from_slice(&body).unwrap_or_default();
            let method = envelope.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let id = envelope.get("id").cloned().unwrap_or(Value::Null);

            let resp = match method {
                "initialize" => serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "fake", "version": "0"},
                    }
                }),
                "tools/list" => serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "tools": [{
                            "name": "echo",
                            "description": "Echo input",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"text": {"type": "string"}},
                            }
                        }]
                    }
                }),
                _ => {
                    // notifications/initialized etc. — return 202
                    return (axum::http::StatusCode::ACCEPTED, axum::body::Body::empty()).into_response();
                }
            };

            (
                axum::http::StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                axum::body::Body::from(serde_json::to_vec(&resp).unwrap()),
            ).into_response()
        }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{}", addr), handle)
    }

    #[tokio::test]
    async fn test_client_list_tools() {
        let (url, _handle) = fake_mcp_server().await;
        let client = connect_http(&url, reqwest::Client::new()).await.unwrap();
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        assert_eq!(tools[0].description, "Echo input");
        let schema_str = tools[0].input_schema.to_string();
        assert!(schema_str.contains("text"), "schema should mention 'text', got: {}", schema_str);
    }

    #[tokio::test]
    async fn test_client_connect_fails_on_bad_url() {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            connect_http("http://127.0.0.1:1/definitely-closed", reqwest::Client::new()),
        ).await;

        match result {
            Err(_) => {} // timeout is fine — connection refused should be fast though
            Ok(r) => {
                assert!(r.is_err(), "connecting to a closed port should fail");
                let err = r.unwrap_err().to_string().to_lowercase();
                assert!(
                    err.contains("connect") || err.contains("refused") || err.contains("initialize") || err.contains("error"),
                    "unexpected error: {}", err
                );
            }
        }
    }
}