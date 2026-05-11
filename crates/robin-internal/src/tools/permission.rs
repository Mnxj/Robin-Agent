use std::collections::HashMap;

use serde_json::Value;

use crate::llm::ToolDef;
use super::policy::Policy;

/// The outcome of a permission check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionBehavior {
    Allow,
    Deny,
}

/// The result of a `PermissionChecker::check` call.
#[derive(Debug, Clone)]
pub struct Decision {
    pub behavior: DecisionBehavior,
    /// Reason is surfaced into the tool result when the behavior is Deny;
    /// ignored when Allow.
    pub reason: String,
}

impl Decision {
    pub fn allow() -> Self {
        Self { behavior: DecisionBehavior::Allow, reason: String::new() }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self { behavior: DecisionBehavior::Deny, reason: reason.into() }
    }
}

/// Decides whether a tool call may proceed.
pub trait PermissionChecker: Send + Sync {
    fn check(&self, agent_id: &str, tool_name: &str, input: &Value) -> Decision;

    /// Returns the subset of `tool_defs` visible to the given agent.
    fn filter_tool_defs(&self, tool_defs: &[ToolDef], agent_id: &str) -> Vec<ToolDef>;
}

/// The default `PermissionChecker`. Wraps per-agent `Policy` values keyed by
/// agent ID. An agent not present in the map is treated as allow-all.
pub struct StaticChecker {
    per_agent: HashMap<String, Policy>,
}

impl StaticChecker {
    /// Creates a new `StaticChecker`. A `None` or empty map means allow-all
    /// for every agent.
    pub fn new(per_agent: HashMap<String, Policy>) -> Self {
        Self { per_agent }
    }
}

impl PermissionChecker for StaticChecker {
    fn check(&self, agent_id: &str, tool_name: &str, _input: &Value) -> Decision {
        match self.per_agent.get(agent_id) {
            None => Decision::allow(),
            Some(p) if p.is_allowed(tool_name) => Decision::allow(),
            _ => Decision::deny(format!(
                "tool {:?} is not allowed for agent {:?}",
                tool_name, agent_id
            )),
        }
    }

    fn filter_tool_defs(&self, tool_defs: &[ToolDef], agent_id: &str) -> Vec<ToolDef> {
        match self.per_agent.get(agent_id) {
            None => tool_defs.to_vec(),
            Some(p) => tool_defs.iter().filter(|td| p.is_allowed(&td.name)).cloned().collect(),
        }
    }
}

#[path = "permission_test.rs"]
#[cfg(test)]
mod permission_test;