use serde_json::Value;

use super::tool::{Tool, ToolResult};

/// Tool for managing core memory.
pub struct ManageCoreMemoryTool {
    pub add: Option<Box<dyn Fn(&str, &str) -> anyhow::Result<()> + Send + Sync>>,
    pub update: Option<Box<dyn Fn(&str, &str, &str) -> anyhow::Result<()> + Send + Sync>>,
    pub delete: Option<Box<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>>,
}

impl ManageCoreMemoryTool {
    pub fn new(
        add: impl Fn(&str, &str) -> anyhow::Result<()> + Send + Sync + 'static,
        update: impl Fn(&str, &str, &str) -> anyhow::Result<()> + Send + Sync + 'static,
        delete: impl Fn(&str) -> anyhow::Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            add: Some(Box::new(add)),
            update: Some(Box::new(update)),
            delete: Some(Box::new(delete)),
        }
    }
}

impl Tool for ManageCoreMemoryTool {
    fn name(&self) -> &str { "manage_core_memory" }

    fn description(&self) -> &str {
        "Extract important information from recent conversations and perform memory management, thereby helping maintain semantic coherence in long conversation tasks. Operations include ADD, UPDATE, and DELETE."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": {
                                "type": "string",
                                "enum": ["ADD", "UPDATE", "DELETE"]
                            },
                            "category": { "type": "string" },
                            "content": { "type": "string" },
                            "keywords": { "type": "string" },
                            "memento_id": { "type": "string" },
                            "scope": { "type": "string" },
                            "title": { "type": "string" },
                            "via": { "type": "string" }
                        },
                        "required": ["action"]
                    }
                }
            },
            "required": ["operations"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let ops = match input.get("operations").and_then(|v| v.as_array()) {
            Some(ops) => ops,
            None => return Ok(ToolResult::err("operations array is required")),
        };

        let mut results = Vec::new();
        let mut has_error = false;

        for op in ops {
            let action = op.get("action").and_then(|v| v.as_str()).unwrap_or("");
            match action {
                "ADD" => {
                    let title = op.get("title").and_then(|v| v.as_str()).unwrap_or("Memory");
                    let content = op.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let id = uuid::Uuid::new_v4().to_string();
                    if let Some(f) = &self.add {
                        match f(&id, &format!("{}\n\n{}", title, content)) {
                            Ok(_) => results.push(format!("ADD: id={} title={:?}", id, title)),
                            Err(e) => {
                                has_error = true;
                                results.push(format!("ADD failed: {}", e));
                            }
                        }
                    }
                }
                "UPDATE" => {
                    let id = op.get("memento_id").and_then(|v| v.as_str()).unwrap_or("");
                    let title = op.get("title").and_then(|v| v.as_str()).unwrap_or("Memory");
                    let content = op.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(f) = &self.update {
                        match f(id, title, content) {
                            Ok(_) => results.push(format!("UPDATE: id={}", id)),
                            Err(e) => {
                                has_error = true;
                                results.push(format!("UPDATE failed for id {}: {}", id, e));
                            }
                        }
                    }
                }
                "DELETE" => {
                    let id = op.get("memento_id").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(f) = &self.delete {
                        match f(id) {
                            Ok(_) => results.push(format!("DELETE: id={}", id)),
                            Err(e) => {
                                has_error = true;
                                results.push(format!("DELETE failed for id {}: {}", id, e));
                            }
                        }
                    }
                }
                _ => {
                    has_error = true;
                    results.push(format!("Unknown action: {}", action));
                }
            }
        }

        if has_error {
            Ok(ToolResult::err(results.join("\n")))
        } else {
            Ok(ToolResult::ok(results.join("\n")))
        }
    }
}
