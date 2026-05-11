#[cfg(test)]
mod tests {
        use crate::tools::tool::{Registry, Tool};
    use crate::tools::{
        bash::BashTool,
        browser::BrowserTool,
        editfile::EditFileTool,
        readfile::ReadFileTool,
        websearch::WebSearchTool,
        webfetch::WebFetchTool,
        writefile::WriteFileTool,
        todo::TodoWriteTool,
    };

    fn register_core_tools(reg: &Registry, work_dir: &str) {
        reg.register(ReadFileTool { work_dir: work_dir.to_owned() });
        reg.register(WriteFileTool { work_dir: work_dir.to_owned() });
        reg.register(EditFileTool { work_dir: work_dir.to_owned() });
        reg.register(BashTool { work_dir: work_dir.to_owned(), exec_policy: None });
        reg.register(WebFetchTool);
        reg.register(WebSearchTool::new());
        reg.register_arc(crate::tools::browser::new_browser_tool());
        reg.register(TodoWriteTool::new(work_dir));
    }

    #[test]
    fn test_registry() {
        let reg = Registry::new();
        register_core_tools(&reg, "");

        let names = reg.names();
        assert_eq!(names.len(), 8);

        for name in &["read_file", "write_file", "edit_file", "bash", "web_fetch", "web_search", "browser", "todo_write"] {
            let tool = reg.get(name);
            assert!(tool.is_some(), "tool {:?} should exist", name);
            assert_eq!(tool.as_ref().unwrap().name(), *name);
            assert!(!tool.unwrap().description().is_empty());
        }
    }

    #[test]
    fn test_tool_defs() {
        let reg = Registry::new();
        register_core_tools(&reg, "");
        let defs = reg.tool_defs();
        assert_eq!(defs.len(), 8);
    }

    #[test]
    fn test_read_file_tool() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, b"hello world").unwrap();

        let tool = ReadFileTool::default();
        let res = tool.execute(serde_json::json!({"path": path.to_str().unwrap()})).unwrap();
        assert_eq!(res.output, "hello world");
        assert!(res.error.is_empty(), "error: {}", res.error);
    }

    #[test]
    fn test_read_file_tool_missing() {
        let tool = ReadFileTool::default();
        let res = tool.execute(serde_json::json!({"path": "/nonexistent/file"})).unwrap();
        assert!(!res.error.is_empty(), "expected error for missing file");
    }

    #[test]
    fn test_write_file_tool() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subdir").join("output.txt");

        let tool = WriteFileTool::default();
        let res = tool.execute(serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": "test content"
        })).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert!(res.output.contains("Successfully wrote"), "output: {}", res.output);

        let data = std::fs::read_to_string(&path).unwrap();
        assert_eq!(data, "test content");
    }

    #[test]
    fn test_edit_file_tool() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edit.txt");
        std::fs::write(&path, b"hello world").unwrap();

        let tool = EditFileTool::default();
        let res = tool.execute(serde_json::json!({
            "path": path.to_str().unwrap(),
            "old_string": "world",
            "new_string": "Go"
        })).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);

        let data = std::fs::read_to_string(&path).unwrap();
        assert_eq!(data, "hello Go");
    }

    #[test]
    fn test_edit_file_tool_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edit.txt");
        std::fs::write(&path, b"hello world").unwrap();

        let tool = EditFileTool::default();
        let res = tool.execute(serde_json::json!({
            "path": path.to_str().unwrap(),
            "old_string": "missing",
            "new_string": "replacement"
        })).unwrap();
        assert!(res.error.contains("not found"), "error: {}", res.error);
    }

    #[test]
    fn test_bash_tool() {
        let _g = crate::TEST_PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tool = BashTool::default();
        let res = tool.execute(serde_json::json!({"command": "echo hello"})).unwrap();
        assert_eq!(res.output.trim(), "hello");
        assert!(res.error.is_empty(), "error: {}", res.error);
    }

    #[test]
    fn test_bash_tool_error() {
        let _g = crate::TEST_PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tool = BashTool::default();
        let res = tool.execute(serde_json::json!({"command": "exit 1"})).unwrap();
        assert!(!res.error.is_empty(), "expected error for exit 1");
    }
}