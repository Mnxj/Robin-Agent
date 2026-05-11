use serde_json::Value;

use super::tool::{Tool, ToolResult};

/// Fetches a skill body by name. Decoupled from the skill package via
/// the `lookup` function — keeps the tools module free of skill/memory imports.
pub struct LoadSkillTool {
    /// Returns `(body, true)` if a skill with the given name exists.
    pub lookup: Option<Box<dyn Fn(&str) -> Option<String> + Send + Sync>>,
}

impl LoadSkillTool {
    pub fn new(lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        Self { lookup: Some(Box::new(lookup)) }
    }
}

impl Tool for LoadSkillTool {
    fn name(&self) -> &str { "load_skill" }

    fn description(&self) -> &str {
        "Load the full body of a skill by name. Use after consulting the Skills Index to read a skill's instructions; the body is returned as the tool output and becomes available in your next response. Pass the exact skill name from the index."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Exact skill name as listed in the Skills Index"
                }
            },
            "required": ["name"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { true }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let name = match input.get("name").and_then(|v| v.as_str()) {
            Some(n) if !n.is_empty() => n.to_owned(),
            Some(_) => return Ok(ToolResult::err("name is required")),
            None => return Ok(ToolResult::err("name is required")),
        };

        match &self.lookup {
            None => Ok(ToolResult::err("no skill loader configured")),
            Some(f) => match f(&name) {
                None => Ok(ToolResult::err(format!("skill not found: {:?}", name))),
                Some(body) if body.is_empty() => Ok(ToolResult::ok(format!(
                    "(skill {:?} loaded but has no body content)",
                    name
                ))),
                Some(body) => Ok(ToolResult::ok(body)),
            },
        }
    }
}

/// Fetches a memory entry body by id. Same decoupling pattern as
/// `LoadSkillTool`.
pub struct LoadMemoryTool {
    pub lookup: Option<Box<dyn Fn(&str) -> Option<String> + Send + Sync>>,
}

impl LoadMemoryTool {
    pub fn new(lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        Self { lookup: Some(Box::new(lookup)) }
    }
}

impl Tool for LoadMemoryTool {
    fn name(&self) -> &str { "load_memory" }

    fn description(&self) -> &str {
        "Load the full body of a memory entry by id. Use after consulting the Memory Index to read an entry's content; the body is returned as the tool output. Pass the exact id from the index."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Exact memory entry id as listed in the Memory Index"
                }
            },
            "required": ["id"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { true }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let id = match input.get("id").and_then(|v| v.as_str()) {
            Some(i) if !i.is_empty() => i.to_owned(),
            Some(_) => return Ok(ToolResult::err("id is required")),
            None => return Ok(ToolResult::err("id is required")),
        };

        match &self.lookup {
            None => Ok(ToolResult::err("no memory manager configured")),
            Some(f) => match f(&id) {
                None => Ok(ToolResult::err(format!("memory entry not found: {:?}", id))),
                Some(body) if body.is_empty() => Ok(ToolResult::ok(format!(
                    "(memory entry {:?} loaded but is empty)",
                    id
                ))),
                Some(body) => Ok(ToolResult::ok(body)),
            },
        }
    }
}

#[path = "load_test.rs"]
#[cfg(test)]
mod load_test;