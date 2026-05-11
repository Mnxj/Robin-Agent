/// depth.rs — Subagent recursion-depth helpers.
///
/// Mirrors Go's depth.go. Reads ROBIN_MAX_AGENT_DEPTH from the environment
/// when the config field is zero.
use super::runtime::Runtime;

impl Runtime {
    /// Returns the maximum subagent recursion depth for this Runtime.
    ///
    /// Precedence:
    ///   1. `agent_loop.max_agent_depth` > 0  — config wins.
    ///   2. `ROBIN_MAX_AGENT_DEPTH` env var > 0  — env fallback.
    ///   3. Default 3.
    pub fn max_agent_depth(&self) -> usize {
        if self.agent_loop.max_agent_depth > 0 {
            return self.agent_loop.max_agent_depth as usize;
        }
        if let Ok(v) = std::env::var("ROBIN_MAX_AGENT_DEPTH") {
            if let Ok(n) = v.trim().parse::<usize>() {
                if n > 0 {
                    return n;
                }
            }
        }
        3
    }
}

#[cfg(test)]
#[path = "depth_test.rs"]
mod depth_test;
