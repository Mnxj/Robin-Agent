#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::{format_todos, TodoItem, TodoStatus, TodoWriteTool};
    use crate::tools::tool::Tool;

    fn next_todo_id(items: &[TodoItem]) -> String {
        let max = items.iter().filter_map(|it| {
            it.id.strip_prefix('t').and_then(|n| n.parse::<usize>().ok())
        }).max().unwrap_or(0);
        format!("t{}", max + 1)
    }

    fn find_idx(s: &str, sub: &str) -> Option<usize> {
        s.find(sub)
    }

    #[test]
    fn test_todo_write_list_empty() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool::new(dir.path().to_str().unwrap());
        let res = tool.execute(serde_json::json!({"op": "list"})).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert_eq!(res.output, "(no todos)");
    }

    #[test]
    fn test_todo_write_add_returns_item_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool::new(dir.path().to_str().unwrap());
        let res = tool.execute(serde_json::json!({"op": "add", "content": "first task"})).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert!(res.output.contains("[ ] t1 — first task"), "output: {}", res.output);

        // Persisted to disk
        let path = dir.path().join(".robin").join("todos.json");
        let data = std::fs::read_to_string(&path).unwrap();
        assert!(data.contains("first task"), "file: {}", data);

        // A fresh tool instance reads it
        let tool2 = TodoWriteTool::new(dir.path().to_str().unwrap());
        let res2 = tool2.execute(serde_json::json!({"op": "list"})).unwrap();
        assert!(res2.output.contains("first task"), "output2: {}", res2.output);
    }

    #[test]
    fn test_todo_write_add_missing_content_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool::new(dir.path().to_str().unwrap());
        let res = tool.execute(serde_json::json!({"op": "add"})).unwrap();
        assert!(res.error.contains("content is required"), "error: {}", res.error);
    }

    #[test]
    fn test_todo_write_complete_shortcut() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool::new(dir.path().to_str().unwrap());
        tool.execute(serde_json::json!({"op": "add", "content": "x"})).unwrap();
        let res = tool.execute(serde_json::json!({"op": "complete", "id": "t1"})).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert!(res.output.contains("[x] t1 — x"), "output: {}", res.output);
    }

    #[test]
    fn test_todo_write_update_changes_status_and_content() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool::new(dir.path().to_str().unwrap());
        tool.execute(serde_json::json!({"op": "add", "content": "orig"})).unwrap();
        let res = tool.execute(serde_json::json!({
            "op": "update", "id": "t1", "content": "revised", "status": "in_progress"
        })).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert!(res.output.contains("[~] t1 — revised"), "output: {}", res.output);
    }

    #[test]
    fn test_todo_write_update_invalid_status_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool::new(dir.path().to_str().unwrap());
        tool.execute(serde_json::json!({"op": "add", "content": "x"})).unwrap();
        let res = tool.execute(serde_json::json!({
            "op": "update", "id": "t1", "status": "frobnicated"
        })).unwrap();
        assert!(res.error.contains("invalid status"), "error: {}", res.error);
    }

    #[test]
    fn test_todo_write_remove() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool::new(dir.path().to_str().unwrap());
        tool.execute(serde_json::json!({"op": "add", "content": "x"})).unwrap();
        tool.execute(serde_json::json!({"op": "add", "content": "y"})).unwrap();
        let res = tool.execute(serde_json::json!({"op": "remove", "id": "t1"})).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert!(!res.output.contains("t1"), "output should not contain t1: {}", res.output);
        assert!(res.output.contains("t2 — y"), "output: {}", res.output);
    }

    #[test]
    fn test_todo_write_remove_unknown_id() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool::new(dir.path().to_str().unwrap());
        let res = tool.execute(serde_json::json!({"op": "remove", "id": "ghost"})).unwrap();
        assert!(res.error.contains("todo not found"), "error: {}", res.error);
    }

    #[test]
    fn test_todo_write_unknown_op_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool::new(dir.path().to_str().unwrap());
        let res = tool.execute(serde_json::json!({"op": "frobnicate"})).unwrap();
        assert!(res.error.contains("unknown op"), "error: {}", res.error);
    }

    #[test]
    fn test_todo_write_empty_input_defaults_to_list() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool::new(dir.path().to_str().unwrap());
        let res = tool.execute(serde_json::json!({})).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert_eq!(res.output, "(no todos)");
    }

    #[test]
    fn test_next_todo_id_gaps_are_safe() {
        let items = vec![
            TodoItem { id: "t1".to_owned(), content: String::new(), status: TodoStatus::Pending, created_at: 0, updated_at: 0 },
            TodoItem { id: "t3".to_owned(), content: String::new(), status: TodoStatus::Pending, created_at: 0, updated_at: 0 },
        ];
        assert_eq!(next_todo_id(&items), "t4");
    }

    #[test]
    fn test_next_todo_id_ignores_non_numeric() {
        let items = vec![
            TodoItem { id: "legacy".to_owned(), content: String::new(), status: TodoStatus::Pending, created_at: 0, updated_at: 0 },
            TodoItem { id: "t2".to_owned(), content: String::new(), status: TodoStatus::Pending, created_at: 0, updated_at: 0 },
        ];
        assert_eq!(next_todo_id(&items), "t3");
    }

    #[test]
    fn test_format_todos_sorts_by_status_then_id() {
        let items = vec![
            TodoItem { id: "t3".to_owned(), content: "c3".to_owned(), status: TodoStatus::Completed, created_at: 0, updated_at: 0 },
            TodoItem { id: "t1".to_owned(), content: "c1".to_owned(), status: TodoStatus::Pending, created_at: 0, updated_at: 0 },
            TodoItem { id: "t2".to_owned(), content: "c2".to_owned(), status: TodoStatus::InProgress, created_at: 0, updated_at: 0 },
            TodoItem { id: "t4".to_owned(), content: "c4".to_owned(), status: TodoStatus::InProgress, created_at: 0, updated_at: 0 },
        ];
        let got = format_todos(&items);
        let t2_idx = find_idx(&got, "t2").unwrap();
        let t4_idx = find_idx(&got, "t4").unwrap();
        let t1_idx = find_idx(&got, "t1").unwrap();
        let t3_idx = find_idx(&got, "t3").unwrap();
        assert!(t2_idx < t4_idx, "in_progress sorted by id");
        assert!(t4_idx < t1_idx, "in_progress before pending");
        assert!(t1_idx < t3_idx, "pending before completed");
    }

    #[test]
    fn test_todo_write_corrupt_store_surfaces_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".robin")).unwrap();
        std::fs::write(dir.path().join(".robin").join("todos.json"), b"{not valid json").unwrap();

        let tool = TodoWriteTool::new(dir.path().to_str().unwrap());
        let res = tool.execute(serde_json::json!({"op": "list"})).unwrap();
        assert!(res.error.contains("load todos"), "error: {}", res.error);
    }

    #[test]
    fn test_todo_write_headless_no_work_dir() {
        let tool = TodoWriteTool::default();
        let res = tool.execute(serde_json::json!({"op": "add", "content": "x"})).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert!(res.output.contains("[ ] t1 — x"), "output: {}", res.output);

        // No persistence, so the second list call sees nothing.
        let res2 = tool.execute(serde_json::json!({"op": "list"})).unwrap();
        assert_eq!(res2.output, "(no todos)");
    }

    #[test]
    fn test_todo_write_is_concurrency_safe_false() {
        let tool = TodoWriteTool::default();
        assert!(!tool.is_concurrency_safe(&serde_json::json!(null)));
    }
}