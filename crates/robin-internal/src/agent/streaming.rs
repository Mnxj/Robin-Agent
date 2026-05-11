/// streaming.rs — Streaming-tool-kickoff feature flag and kickoff helpers.
///
/// Mirrors Go's streaming.go.
use super::runtime::Runtime;
use crate::llm::ToolCall;
use crate::tools::tool::ToolResult;

impl Runtime {
    /// Returns true when streaming tool kickoff is enabled.
    ///
    /// Precedence:
    ///   1. `agent_loop.streaming_tools == true` — config wins.
    ///   2. `ROBIN_STREAMING_TOOLS == "1"` — env fallback.
    ///   3. Off by default.
    pub fn streaming_tools_enabled(&self) -> bool {
        if self.agent_loop.streaming_tools {
            return true;
        }
        std::env::var("ROBIN_STREAMING_TOOLS").as_deref() == Ok("1")
    }
}

/// The payload sent by a streaming-kickoff goroutine once `execute_tool_kickoff`
/// returns. Mirrors Go's `kickoffResult`.
pub struct KickoffResult {
    pub tc: ToolCall,
    pub result: ToolResult,
    pub aborted: bool,
}