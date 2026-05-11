use serde_json::Value;

use super::tool::{expand_home, validate_path_in_work_dir, Tool, ToolResult};

/// Creates or overwrites a file.
pub struct WriteFileTool {
    /// If set, restricts writes to this directory.
    pub work_dir: String,
}

impl Default for WriteFileTool {
    fn default() -> Self { Self { work_dir: String::new() } }
}

impl Tool for WriteFileTool {
    fn name(&self) -> &str { "write_file" }

    fn description(&self) -> &str {
        "Write content to a file at the given path. Creates the file and any parent directories if they don't exist. Overwrites existing files."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The absolute or relative path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_owned(),
            _ => return Ok(ToolResult::err("path is required")),
        };
        let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("");

        let path = expand_home(&path);

        if !self.work_dir.is_empty() {
            if let Err(e) = validate_path_in_work_dir(&path, &self.work_dir) {
                return Ok(ToolResult::err(e.to_string()));
            }
        }

        // Create parent directories
        if let Some(parent) = std::path::Path::new(&path).parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Ok(ToolResult::err(format!("failed to create directory: {}", e)));
            }
        }

        if let Err(e) = std::fs::write(&path, content.as_bytes()) {
            return Ok(ToolResult::err(format!("failed to write file: {}", e)));
        }

        Ok(ToolResult::ok(format!(
            "Successfully wrote {} bytes to {}",
            content.len(),
            path
        )))
    }
}