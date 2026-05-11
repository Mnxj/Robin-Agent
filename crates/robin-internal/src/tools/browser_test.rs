#[cfg(test)]
mod tests {
    use super::super::{BrowserSession, BrowserTool, SESSION_MAX_COUNT};
    use crate::tools::tool::Tool;
    use std::time::Instant;

    #[test]
    fn test_browser_tool_name() {
        let tool = BrowserTool::new();
        assert_eq!(tool.name(), "browser");
    }

    #[test]
    fn test_browser_tool_parameters_valid_json() {
        let tool = BrowserTool::new();
        let params = tool.parameters();
        // Just verify it's an object
        assert!(params.is_object());
    }

    #[test]
    fn test_browser_tool_missing_action() {
        let tool = BrowserTool::new();
        let res = tool.execute(serde_json::json!({})).unwrap();
        assert!(res.error.contains("action is required"), "error: {}", res.error);
    }

    #[test]
    fn test_browser_tool_unknown_action() {
        let tool = BrowserTool::new();
        let res = tool.execute(serde_json::json!({"action": "fly"})).unwrap();
        assert!(res.error.contains("unknown action"), "error: {}", res.error);
        assert!(res.error.contains("fly"), "error: {}", res.error);
    }

    #[test]
    fn test_browser_navigate_missing_url() {
        let tool = BrowserTool::new();
        let res = tool.navigate_pub(super::super::BrowserInputForTest {
            action: "navigate".to_owned(),
            ..Default::default()
        }).unwrap();
        assert!(res.error.contains("url is required"), "error: {}", res.error);
    }

    #[test]
    fn test_browser_navigate_invalid_url() {
        let tool = BrowserTool::new();
        // Validate via execute path which has the URL check
        let res = tool.execute(serde_json::json!({
            "action": "navigate",
            "url": "ftp://example.com"
        })).unwrap();
        assert!(res.error.contains("http://") || res.error.contains("url must start"), "error: {}", res.error);
    }

    #[test]
    fn test_browser_click_missing_selector() {
        let tool = BrowserTool::new();
        // click without url and without session hits the about:blank trap
        // click with url but missing selector should fail on selector check
        // We test via the internal click method
        let res = tool.execute(serde_json::json!({
            "action": "click",
            "url": "https://example.com"
        })).unwrap();
        assert!(res.error.contains("selector is required"), "error: {}", res.error);
    }

    #[test]
    fn test_browser_type_missing_selector() {
        let tool = BrowserTool::new();
        let res = tool.execute(serde_json::json!({
            "action": "type",
            "url": "https://example.com",
            "text": "hello"
        })).unwrap();
        assert!(res.error.contains("selector is required"), "error: {}", res.error);
    }

    #[test]
    fn test_browser_type_missing_text() {
        let tool = BrowserTool::new();
        let res = tool.execute(serde_json::json!({
            "action": "type",
            "url": "https://example.com",
            "selector": "#input"
        })).unwrap();
        assert!(res.error.contains("text is required"), "error: {}", res.error);
    }

    #[test]
    fn test_browser_evaluate_missing_script() {
        let tool = BrowserTool::new();
        let res = tool.execute(serde_json::json!({
            "action": "evaluate",
            "url": "https://example.com"
        })).unwrap();
        assert!(res.error.contains("script is required"), "error: {}", res.error);
    }

    #[test]
    fn test_browser_about_blank_trap() {
        let tool = BrowserTool::new();
        for action in &["screenshot", "get_text", "evaluate", "click", "type"] {
            let mut input = serde_json::json!({"action": action});
            match *action {
                "click" => input["selector"] = serde_json::json!("#x"),
                "type" => {
                    input["selector"] = serde_json::json!("#x");
                    input["text"] = serde_json::json!("hi");
                }
                "evaluate" => input["script"] = serde_json::json!("1+1"),
                _ => {}
            }
            let res = tool.execute(input).unwrap();
            assert!(res.error.contains(action), "action={}, error: {}", action, res.error);
            assert!(res.error.contains("url"), "action={}, error: {}", action, res.error);
            assert!(res.error.contains("session"), "action={}, error: {}", action, res.error);
        }
    }

    #[test]
    fn test_browser_about_blank_trap_bypassed_by_url() {
        let tool = BrowserTool::new();
        let res = tool.execute(serde_json::json!({"action": "screenshot", "url": "ftp://nope"})).unwrap();
        assert!(res.error.contains("http://") || res.error.contains("url must start"), "error: {}", res.error);
        assert!(!res.error.contains("fresh browser"), "error: {}", res.error);
    }

    #[test]
    fn test_browser_close_requires_session() {
        let tool = BrowserTool::new();
        let res = tool.execute(serde_json::json!({"action": "close"})).unwrap();
        assert!(res.error.contains("session is required"), "error: {}", res.error);
    }

    #[test]
    fn test_browser_close_unknown_session() {
        let tool = BrowserTool::new();
        let res = tool.execute(serde_json::json!({"action": "close", "session": "nope"})).unwrap();
        assert!(res.error.is_empty(), "unexpected error: {}", res.error);
        assert!(res.output.contains("No active session"), "output: {}", res.output);
        assert!(res.output.contains("nope"), "output: {}", res.output);
    }

    #[test]
    fn test_browser_session_limit() {
        let tool = BrowserTool::new();
        // Pre-populate sessions up to the limit
        {
            let mut guard = tool.sessions.lock();
            for i in 0..SESSION_MAX_COUNT {
                guard.insert(
                    format!("s{}", i),
                    BrowserSession { last_used: Instant::now() },
                );
            }
        }
        let err = tool.get_or_create_session("overflow").unwrap_err();
        assert!(err.to_string().contains("session limit reached"), "err: {}", err);
    }

    #[test]
    fn test_browser_close_session_cleans_map() {
        let tool = BrowserTool::new();
        tool.sessions.lock().insert("a".to_owned(), BrowserSession { last_used: Instant::now() });
        assert!(tool.close_session("a"));
        assert!(!tool.sessions.lock().contains_key("a"));
        assert!(!tool.close_session("a"));
    }
}