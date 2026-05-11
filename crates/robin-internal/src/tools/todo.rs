use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::tool::{Tool, ToolResult};

/// Status of a todo item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

impl TodoStatus {
    fn order(&self) -> u8 {
        match self {
            TodoStatus::InProgress => 0,
            TodoStatus::Pending => 1,
            TodoStatus::Completed => 2,
        }
    }
}

impl Default for TodoStatus {
    fn default() -> Self { TodoStatus::Pending }
}

impl std::fmt::Display for TodoStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TodoStatus::Pending => write!(f, "pending"),
            TodoStatus::InProgress => write!(f, "in_progress"),
            TodoStatus::Completed => write!(f, "completed"),
        }
    }
}

/// One entry in the per-workspace todo list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: TodoStatus,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

/// Maintains a per-workspace todo list. Persisted to `<WorkDir>/.robin/todos.json`.
pub struct TodoWriteTool {
    pub work_dir: String,
    mu: Mutex<()>,
}

impl Default for TodoWriteTool {
    fn default() -> Self {
        Self { work_dir: String::new(), mu: Mutex::new(()) }
    }
}

impl TodoWriteTool {
    pub fn new(work_dir: impl Into<String>) -> Self {
        Self { work_dir: work_dir.into(), mu: Mutex::new(()) }
    }

    fn todos_path(&self) -> PathBuf {
        PathBuf::from(&self.work_dir).join(".robin").join("todos.json")
    }

    fn load(&self) -> anyhow::Result<Vec<TodoItem>> {
        if self.work_dir.is_empty() {
            return Ok(Vec::new());
        }
        let path = self.todos_path();
        match std::fs::read(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
            Ok(data) if data.is_empty() => Ok(Vec::new()),
            Ok(data) => {
                serde_json::from_slice(&data)
                    .map_err(|e| anyhow::anyhow!("corrupt todos.json: {}", e))
            }
        }
    }

    fn save(&self, items: &[TodoItem]) -> anyhow::Result<()> {
        if self.work_dir.is_empty() {
            return Ok(());
        }
        let path = self.todos_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec_pretty(items)?;
        std::fs::write(&path, &data)?;
        Ok(())
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn next_todo_id(items: &[TodoItem]) -> String {
    let max = items.iter().filter_map(|it| {
        it.id.strip_prefix('t').and_then(|n| n.parse::<usize>().ok())
    }).max().unwrap_or(0);
    format!("t{}", max + 1)
}

fn find_todo(items: &[TodoItem], id: &str) -> Option<usize> {
    items.iter().position(|it| it.id == id)
}

fn valid_status(s: &str) -> Option<TodoStatus> {
    match s {
        "pending" => Some(TodoStatus::Pending),
        "in_progress" => Some(TodoStatus::InProgress),
        "completed" => Some(TodoStatus::Completed),
        _ => None,
    }
}

/// Renders the list as a checklist sorted: in_progress → pending → completed.
pub fn format_todos(items: &[TodoItem]) -> String {
    if items.is_empty() {
        return "(no todos)".to_owned();
    }
    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| {
        a.status.order().cmp(&b.status.order()).then(a.id.cmp(&b.id))
    });
    let mut b = String::new();
    for it in &sorted {
        let marker = match it.status {
            TodoStatus::Completed => "[x]",
            TodoStatus::InProgress => "[~]",
            TodoStatus::Pending => "[ ]",
        };
        b.push_str(&format!("{} {} — {}\n", marker, it.id, it.content));
    }
    b.trim_end_matches('\n').to_owned()
}

impl Tool for TodoWriteTool {
    fn name(&self) -> &str { "todo_write" }

    fn description(&self) -> &str {
        "Persistent todo list for tracking long, multi-stage work. DO NOT use this as a planning scratchpad before starting a task — start working directly and call your real tools. Reserve todo_write for genuinely long-running work with roughly 5+ independent subtasks that will span many turns. When initializing a list, emit every `add` call as a parallel tool call in the SAME assistant response (the runtime serialises them safely) — never call `add` once per turn, that wastes round trips. Operations: list (default), add, update, complete, remove. The full current list is returned every call."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "op": {
                    "type": "string",
                    "enum": ["list", "add", "update", "complete", "remove"],
                    "description": "Operation to perform. Default: list."
                },
                "id": {
                    "type": "string",
                    "description": "Todo id, required for update/complete/remove. Returned by add."
                },
                "content": {
                    "type": "string",
                    "description": "Todo body, required for add and optional for update."
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed"],
                    "description": "Status, used by update. Add starts items as pending; complete is shorthand for status=completed."
                }
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let _guard = self.mu.lock();

        let op = input.get("op").and_then(|v| v.as_str()).unwrap_or("list");
        let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let status_str = input.get("status").and_then(|v| v.as_str()).unwrap_or("");

        let mut items = match self.load() {
            Ok(items) => items,
            Err(e) => return Ok(ToolResult::err(format!("load todos: {}", e))),
        };

        let now = now_unix();

        match op {
            "list" => {}
            "add" => {
                if content.is_empty() {
                    return Ok(ToolResult::err("content is required for add"));
                }
                let new_id = next_todo_id(&items);
                items.push(TodoItem {
                    id: new_id,
                    content: content.to_owned(),
                    status: TodoStatus::Pending,
                    created_at: now,
                    updated_at: now,
                });
            }
            "update" => {
                if id.is_empty() {
                    return Ok(ToolResult::err("id is required for update"));
                }
                let idx = match find_todo(&items, id) {
                    Some(i) => i,
                    None => return Ok(ToolResult::err(format!("todo not found: {:?}", id))),
                };
                if !content.is_empty() {
                    items[idx].content = content.to_owned();
                }
                if !status_str.is_empty() {
                    match valid_status(status_str) {
                        Some(s) => items[idx].status = s,
                        None => return Ok(ToolResult::err(format!("invalid status: {:?}", status_str))),
                    }
                }
                items[idx].updated_at = now;
            }
            "complete" => {
                if id.is_empty() {
                    return Ok(ToolResult::err("id is required for complete"));
                }
                let idx = match find_todo(&items, id) {
                    Some(i) => i,
                    None => return Ok(ToolResult::err(format!("todo not found: {:?}", id))),
                };
                items[idx].status = TodoStatus::Completed;
                items[idx].updated_at = now;
            }
            "remove" => {
                if id.is_empty() {
                    return Ok(ToolResult::err("id is required for remove"));
                }
                let idx = match find_todo(&items, id) {
                    Some(i) => i,
                    None => return Ok(ToolResult::err(format!("todo not found: {:?}", id))),
                };
                items.remove(idx);
            }
            other => return Ok(ToolResult::err(format!("unknown op: {:?}", other))),
        }

        if op != "list" {
            if let Err(e) = self.save(&items) {
                return Ok(ToolResult::err(format!("save todos: {}", e)));
            }
        }

        Ok(ToolResult::ok(format_todos(&items)))
    }
}

#[path = "todo_test.rs"]
#[cfg(test)]
mod todo_test;