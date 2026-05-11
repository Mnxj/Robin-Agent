#[cfg(test)]
mod tests {
    use crate::mcp_cmd::run_mcp_login;
    use std::io::Cursor;
    use tempfile::TempDir;

    #[tokio::test]
    #[ignore = "requires robin-internal MCP ported; skeleton only"]
    async fn test_run_mcp_login_end_to_end() {
        let dir = TempDir::new().unwrap();
        let cfg_path = dir.path().join("robin.json5");
        let cfg = serde_json::json!({
            "agents": { "list": [{ "id": "default", "model": "claude-sonnet-4-5-20250929" }] },
            "mcp_servers": [{
                "id": "test-gw",
                "enabled": true,
                "http": {
                    "url": "http://example.invalid/mcp",
                    "auth": {
                        "kind": "oauth2_authorization_code",
                        "auth_url": "http://127.0.0.1:9999/authorize",
                        "token_url": "http://127.0.0.1:9999/token",
                        "client_id": "cid",
                        "redirect_uri": "http://127.0.0.1:9998/cb",
                    }
                }
            }]
        });
        std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
        let mut buf = Cursor::new(Vec::new());
        run_mcp_login(cfg_path.to_str().unwrap(), "test-gw", &mut buf).unwrap();
        let out = String::from_utf8(buf.into_inner()).unwrap();
        assert!(out.contains("Logged in to test-gw"));
    }

    #[tokio::test]
    #[ignore = "requires robin-internal config ported; skeleton only"]
    async fn test_run_mcp_login_unknown_server_errors() {
        let dir = TempDir::new().unwrap();
        let cfg_path = dir.path().join("robin.json5");
        let cfg = serde_json::json!({ "agents": { "list": [] }, "mcp_servers": [] });
        std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
        let mut buf = Cursor::new(Vec::new());
        let err = run_mcp_login(cfg_path.to_str().unwrap(), "ghost", &mut buf).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }
}