use std::collections::HashMap;
use std::pin::Pin;

use serde_json::Value;

use super::tool::{Tool, ToolResult};

/// An event emitted by a subagent during execution.
#[derive(Debug, Clone, Default)]
pub struct AgentEventLike {
    pub event_type: i32,
    pub text: String,
    pub done: bool,
    pub aborted: bool,
    pub err: Option<String>,
}

/// Interface that TaskTool calls on a subagent.
pub trait SubagentRunner: Send + Sync {
    /// Executes the subagent with the given prompt and returns a channel of events.
    fn run(
        &self,
        prompt: String,
    ) -> anyhow::Result<std::sync::mpsc::Receiver<AgentEventLike>>;
}

/// Builds a SubagentRunner for the given subagent ID.
pub type SubagentFactory =
    Box<dyn Fn(&str, i32) -> anyhow::Result<Box<dyn SubagentRunner>> + Send + Sync>;

/// Lets a parent agent delegate work to a subagent registered in the config.
pub struct TaskTool {
    factory: SubagentFactory,
    parent_depth: i32,
    eligible: HashMap<String, String>,
    desc_block: String,
}

impl TaskTool {
    pub fn new(
        factory: SubagentFactory,
        parent_depth: i32,
        eligible: HashMap<String, String>,
    ) -> Self {
        let desc_block = format_eligible_block(&eligible);
        Self { factory, parent_depth, eligible, desc_block }
    }
}

/// Renders the alphabetically-sorted list of available subagents.
fn format_eligible_block(eligible: &HashMap<String, String>) -> String {
    if eligible.is_empty() {
        return String::new();
    }
    let mut ids: Vec<&str> = eligible.keys().map(|s| s.as_str()).collect();
    ids.sort_unstable();
    let mut b = String::from("\n\nAvailable subagents:\n");
    for id in ids {
        b.push_str(&format!("  - {}: {}\n", id, eligible[id]));
    }
    b
}

impl Tool for TaskTool {
    fn name(&self) -> &str { "task" }

    fn description(&self) -> &str {
        // Can't return &str with desc_block borrow; use a static-like pattern
        // by storing the full description in desc_block
        // We'll build description lazily via a method instead — but the trait
        // requires &str, so we store it in desc_block and return a reference.
        // This works because Self outlives the &str borrow here.
        // The description is: base + desc_block
        // We cannot return a dynamically built string from &str easily without
        // either a once_cell or a pre-computed string. Let's pre-compute in desc_block.
        &self.desc_block
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["agent_id", "prompt"],
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "ID of the subagent to invoke. Must be one of the listed subagents in this tool's description."
                },
                "prompt": {
                    "type": "string",
                    "description": "The instruction to send to the subagent. Be self-contained — the subagent has no access to the parent conversation."
                }
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let agent_id = match input.get("agent_id").and_then(|v| v.as_str()) {
            Some(id) if !id.is_empty() => id.to_owned(),
            _ => return Ok(ToolResult::err("task: agent_id and prompt are required")),
        };
        let prompt = match input.get("prompt").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_owned(),
            _ => return Ok(ToolResult::err("task: agent_id and prompt are required")),
        };

        if !self.eligible.contains_key(&agent_id) {
            let mut ids: Vec<&str> = self.eligible.keys().map(|s| s.as_str()).collect();
            ids.sort_unstable();
            return Ok(ToolResult::err(format!(
                "task: unknown subagent {:?} (eligible: {})",
                agent_id,
                ids.join(", ")
            )));
        }

        let runner = match (self.factory)(&agent_id, self.parent_depth) {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::err(format!("task: {}", e))),
        };

        let events = match runner.run(prompt) {
            Ok(ch) => ch,
            Err(e) => return Ok(ToolResult::err(format!("task: subagent run failed: {}", e))),
        };

        let mut out = String::new();
        for ev in events {
            if ev.aborted {
                return Ok(ToolResult::err("task: subagent aborted"));
            }
            if let Some(err) = ev.err {
                return Ok(ToolResult::err(format!("task: subagent error: {}", err)));
            }
            if !ev.text.is_empty() {
                out.push_str(&ev.text);
            }
        }

        Ok(ToolResult::ok(out))
    }
}

// We need a special description field since desc_block contains the full description.
// Override to make TaskTool's description() return a useful static prefix + the block.
impl TaskTool {
    pub fn full_description(&self) -> String {
        format!(
            "Delegate a subtask to a specialized subagent. The subagent runs independently with its own tools and system prompt; its final response is returned as this tool's output.{}",
            self.desc_block
        )
    }
}

#[path = "task_test.rs"]
#[cfg(test)]
mod task_test;