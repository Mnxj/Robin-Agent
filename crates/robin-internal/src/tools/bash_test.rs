#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::{resolve_bash_command_paths, sanitize_llm_text, shell_single_quote};
    use crate::tools::tool::Tool;
    use super::super::BashTool;

    #[test]
    fn test_sanitize_llm_text_ascii_passthrough() {
        assert_eq!(sanitize_llm_text("ls -la /tmp"), "ls -la /tmp");
    }

    #[test]
    fn test_sanitize_llm_text_nbsp_between_words() {
        assert_eq!(
            sanitize_llm_text("open /Users/me/SGQR\u{00a0}Specs.pdf"),
            "open /Users/me/SGQR Specs.pdf"
        );
    }

    #[test]
    fn test_sanitize_llm_text_zero_width_joiner_stripped() {
        assert_eq!(sanitize_llm_text("echo foo\u{200d}bar"), "echo foobar");
    }

    #[test]
    fn test_sanitize_llm_text_bom_stripped() {
        assert_eq!(sanitize_llm_text("\u{feff}ls"), "ls");
    }

    #[test]
    fn test_sanitize_llm_text_line_separator_to_newline() {
        assert_eq!(sanitize_llm_text("ls\u{2028}pwd"), "ls\npwd");
    }

    #[test]
    fn test_sanitize_llm_text_preserves_real_tab_and_newline() {
        assert_eq!(sanitize_llm_text("ls\t-l\npwd"), "ls\t-l\npwd");
    }

    #[test]
    fn test_resolve_bash_command_paths_no_absolute_paths() {
        let (cmd, subs) = resolve_bash_command_paths("echo hello");
        assert_eq!(cmd, "echo hello");
        assert!(subs.is_empty());
    }

    #[test]
    fn test_resolve_bash_command_paths_existing_path_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let plain_path = dir.path().join("plain.txt");
        std::fs::write(&plain_path, b"x").unwrap();
        let cmd = format!("cat {}", plain_path.display());
        let (out_cmd, subs) = resolve_bash_command_paths(&cmd);
        assert_eq!(out_cmd, cmd);
        assert!(subs.is_empty());
    }

    #[test]
    fn test_resolve_bash_command_paths_nbsp_file_is_resolved() {
        let dir = tempfile::tempdir().unwrap();
        // File on disk has NBSP in its name
        let nbsp_path = dir.path().join("SGQR\u{00a0}Specifications.pdf");
        std::fs::write(&nbsp_path, b"x").unwrap();

        // LLM sends ASCII-space variant, double-quoted
        let ascii_path = dir.path().join("SGQR Specifications.pdf");
        let cmd = format!(r#"pdftotext "{}" /tmp/out.txt"#, ascii_path.display());
        let (out_cmd, subs) = resolve_bash_command_paths(&cmd);
        // Should have substituted the path
        assert_eq!(subs.len(), 1, "expected 1 substitution, got {:?}", subs);
        assert!(out_cmd.contains(shell_single_quote(&nbsp_path.to_string_lossy()).as_str()),
            "out_cmd: {}", out_cmd);
    }

    #[test]
    fn test_bash_tool_execute_echo() {
        let _g = crate::TEST_PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tool = BashTool::default();
        let input = serde_json::json!({"command": "echo hello"});
        let res = tool.execute(input).unwrap();
        assert_eq!(res.output.trim(), "hello");
        assert!(res.error.is_empty());
    }

    #[test]
    fn test_bash_tool_execute_missing_command() {
        let tool = BashTool::default();
        let res = tool.execute(serde_json::json!({})).unwrap();
        assert!(res.error.contains("command is required"), "error: {}", res.error);
    }

    #[test]
    fn test_bash_tool_execute_failing_command() {
        let _g = crate::TEST_PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tool = BashTool::default();
        let res = tool.execute(serde_json::json!({"command": "exit 1"})).unwrap();
        assert!(!res.error.is_empty(), "expected error for exit 1");
    }

    #[test]
    fn test_bash_tool_deny_policy() {
        use super::super::ExecPolicy;
        let tool = BashTool {
            exec_policy: Some(ExecPolicy { level: "deny".to_owned(), allowlist: vec![] }),
            ..Default::default()
        };
        let res = tool.execute(serde_json::json!({"command": "echo hi"})).unwrap();
        assert!(res.error.contains("disabled by policy"), "error: {}", res.error);
    }

    #[test]
    fn test_bash_tool_allowlist_blocks_unknown_command() {
        use super::super::ExecPolicy;
        let tool = BashTool {
            exec_policy: Some(ExecPolicy {
                level: "allowlist".to_owned(),
                allowlist: vec!["echo".to_owned()],
            }),
            ..Default::default()
        };
        let res = tool.execute(serde_json::json!({"command": "curl http://example.com"})).unwrap();
        assert!(res.error.contains("not in the exec allowlist"), "error: {}", res.error);
    }
}