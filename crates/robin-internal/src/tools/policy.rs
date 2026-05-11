/// Policy defines allow/deny rules for tool execution.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// Tool names to allow (empty = allow all).
    pub allow: Vec<String>,
    /// Tool names to deny (checked after allow).
    pub deny: Vec<String>,
}

impl Policy {
    /// Checks whether a tool name is permitted by this policy.
    ///
    /// Logic:
    /// - Deny list is checked first; if the tool is in deny, it is blocked.
    /// - If allow list is non-empty, the tool must be in it.
    /// - Otherwise the tool is allowed.
    pub fn is_allowed(&self, tool_name: &str) -> bool {
        // Check deny list first
        for d in &self.deny {
            if d == tool_name || d == "*" {
                return false;
            }
        }

        // If allow list is non-empty, tool must be in it
        if !self.allow.is_empty() {
            for a in &self.allow {
                if a == tool_name || a == "*" {
                    return true;
                }
            }
            return false;
        }

        true
    }
}

#[path = "policy_test.rs"]
#[cfg(test)]
mod policy_test;