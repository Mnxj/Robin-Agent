#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::super::{register_send_message, SendMessageRegistration, SendMessageTool};
    use crate::tools::tool::{Registry, Tool};

    const TELEGRAM_MAX_TEXT: usize = 4096;

    #[test]
    fn test_send_message_name() {
        let tool = SendMessageTool::default();
        assert_eq!(tool.name(), "send_message");
    }

    #[test]
    fn test_send_message_parameters_valid_json() {
        let tool = SendMessageTool::default();
        let params = tool.parameters();
        assert!(params.is_object());
    }

    #[test]
    fn test_send_message_unknown_channel() {
        let tool = SendMessageTool::default();
        let res = tool.execute(serde_json::json!({
            "channel": "smoke-signal",
            "text": "hi",
            "chat_id": "1"
        })).unwrap();
        assert!(res.error.contains("smoke-signal"), "error: {}", res.error);
        assert!(res.error.contains("is not supported"), "error: {}", res.error);
    }

    #[test]
    fn test_send_message_defaults_to_telegram() {
        // No token configured → telegram channel should report not-configured.
        let tool = SendMessageTool::default();
        let res = tool.execute(serde_json::json!({"text": "hi", "chat_id": "1"})).unwrap();
        assert!(res.error.contains("telegram channel is not configured"), "error: {}", res.error);
    }

    #[test]
    fn test_send_message_telegram_missing_text() {
        let tool = SendMessageTool {
            telegram_token: "t".to_owned(),
            telegram_default_chat_id: "1".to_owned(),
            ..Default::default()
        };
        let res = tool.execute(serde_json::json!({"text": "  "})).unwrap();
        assert!(res.error.contains("text is required"), "error: {}", res.error);
    }

    #[test]
    fn test_send_message_telegram_missing_chat() {
        let tool = SendMessageTool {
            telegram_token: "t".to_owned(),
            ..Default::default()
        };
        let res = tool.execute(serde_json::json!({"text": "hello"})).unwrap();
        assert!(res.error.contains("chat_id is required"), "error: {}", res.error);
    }

    #[test]
    fn test_send_message_telegram_too_long() {
        let tool = SendMessageTool {
            telegram_token: "t".to_owned(),
            telegram_default_chat_id: "1".to_owned(),
            ..Default::default()
        };
        let long_text = "a".repeat(TELEGRAM_MAX_TEXT + 1);
        let res = tool.execute(serde_json::json!({"text": long_text})).unwrap();
        assert!(res.error.contains("4096-character limit"), "error: {}", res.error);
    }

    #[test]
    fn test_send_message_telegram_invalid_parse_mode() {
        let tool = SendMessageTool {
            telegram_token: "t".to_owned(),
            telegram_default_chat_id: "1".to_owned(),
            ..Default::default()
        };
        let res = tool.execute(serde_json::json!({"text": "x", "parse_mode": "rtf"})).unwrap();
        assert!(res.error.contains("invalid parse_mode"), "error: {}", res.error);
    }

    #[test]
    fn test_send_message_telegram_success() {
        use std::net::TcpListener;

        // Spin up a minimal HTTP mock server
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let captured: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
        let captured_clone = Arc::clone(&captured);

        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                // Find the body (after \r\n\r\n)
                if let Some(idx) = request.find("\r\n\r\n") {
                    let body = &request[idx + 4..];
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
                        *captured_clone.lock().unwrap() = Some(v);
                    }
                }
                let resp = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 38\r\n\r\n{\"ok\":true,\"result\":{\"message_id\":42}}";
                let _ = stream.write_all(resp);
            }
        });

        let base_url = format!("http://127.0.0.1:{}", port);
        let tool = SendMessageTool {
            telegram_token: "SECRET".to_owned(),
            telegram_default_chat_id: "9999".to_owned(),
            telegram_base_url: base_url,
            ..Default::default()
        };
        let res = tool.execute(serde_json::json!({
            "text": "hello",
            "parse_mode": "Markdown",
            "disable_link_preview": true
        })).unwrap();

        assert!(res.error.is_empty(), "error: {}", res.error);
        assert!(res.output.contains("message_id=42"), "output: {}", res.output);
        assert!(res.output.contains("9999"), "output: {}", res.output);
        assert!(res.output.contains("telegram"), "output: {}", res.output);
    }

    #[test]
    fn test_register_send_message_always_registers() {
        let reg = Registry::new();
        register_send_message(&reg, None);
        let tool = reg.get("send_message");
        assert!(tool.is_some());
        assert_eq!(tool.unwrap().name(), "send_message");
    }

    #[test]
    fn test_register_send_message_with_config_fn() {
        let reg = Registry::new();
        register_send_message(&reg, Some(Box::new(|| SendMessageRegistration {
            telegram_enabled: true,
            telegram_bot_token: "T".to_owned(),
            telegram_default_chat_id: "1".to_owned(),
        })));
        let tool = reg.get("send_message");
        assert!(tool.is_some());
    }

    #[test]
    fn test_send_message_config_fn_disabled_hides_token() {
        let tool = SendMessageTool {
            config_fn: Some(Box::new(|| SendMessageRegistration {
                telegram_enabled: false,
                telegram_bot_token: "T".to_owned(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let res = tool.execute(serde_json::json!({"text": "hi", "chat_id": "1"})).unwrap();
        assert!(res.error.contains("telegram channel is not configured"), "error: {}", res.error);
    }
}