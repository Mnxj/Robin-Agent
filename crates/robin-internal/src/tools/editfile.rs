use serde_json::Value;

use super::tool::{expand_home, resolve_existing_path, validate_path_in_work_dir, Tool, ToolResult};

/// Performs a string-replace edit on a file.
pub struct EditFileTool {
    /// If set, restricts edits to this directory.
    pub work_dir: String,
}

impl Default for EditFileTool {
    fn default() -> Self { Self { work_dir: String::new() } }
}

impl Tool for EditFileTool {
    fn name(&self) -> &str { "edit_file" }

    fn description(&self) -> &str {
        "Edit a file by replacing an exact string match. The old_string must match exactly one occurrence in the file. Use this for targeted edits rather than rewriting entire files."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text"
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_owned(),
            _ => return Ok(ToolResult::err("path is required")),
        };
        let old_string = input.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
        let new_string = input.get("new_string").and_then(|v| v.as_str()).unwrap_or("");

        let path = expand_home(&path);
        let path = resolve_existing_path(&path);

        if !self.work_dir.is_empty() {
            if let Err(e) = validate_path_in_work_dir(&path, &self.work_dir) {
                return Ok(ToolResult::err(e.to_string()));
            }
        }

        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) => return Ok(ToolResult::err(format!("failed to read file: {}", e))),
        };

        let count = data.matches(old_string).count();
        if count == 0 {
            return Ok(ToolResult::err("old_string not found in file"));
        }
        if count > 1 {
            return Ok(ToolResult::err(format!(
                "old_string found {} times in file, must be unique",
                count
            )));
        }

        let new_content = data.replacen(old_string, new_string, 1);

        if let Err(e) = std::fs::write(&path, new_content.as_bytes()) {
            return Ok(ToolResult::err(format!("failed to write file: {}", e)));
        }

        Ok(ToolResult::ok("Successfully edited file"))
    }
}