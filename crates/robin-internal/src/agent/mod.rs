/// Agent package — the think-act loop and related infrastructure.
///
/// Mirrors the Go `internal/agent` package.

pub mod builder;
pub mod context;
pub mod depth;
pub mod partition;
pub mod runtime;
pub mod spill;
pub mod streaming;
pub mod subagent;
pub mod trace;

#[cfg(test)]
pub mod test_support;

// Test modules (each file is linked from its source module via #[path = "..."])
// The following are declared separately here so they compile as part of the crate.
#[cfg(test)]
mod agent_test;
#[cfg(test)]
mod cache_stability_test;
#[cfg(test)]
mod context_test;
#[cfg(test)]
mod dispatch_test;
#[cfg(test)]
mod inheritance_test;
#[cfg(test)]
mod postcompact_test;
#[cfg(test)]
mod streamfallback_test;
#[cfg(test)]
mod streaming_test;

// Re-exports for convenience.
pub use builder::{build_runtime_for_agent, RuntimeDeps, RuntimeInputs};
pub use context::{
    assemble_messages, build_config_summary, build_dynamic_system_prompt_suffix,
    build_static_system_prompt, format_date_line, load_agent_memory_files,
    prepend_post_compact_restore, prune_tool_results, SpillConfig,
    DEFAULT_IDENTITY_BASE, MAX_TOOL_RESULT_LEN, TRUNCATION_MARKER, SPILL_MARKER,
};
pub use partition::{is_call_concurrency_safe, partition_tool_calls, Batch};
pub use runtime::{AgentEvent, AgentEventType, Runtime};
pub use spill::{cleanup_orphaned_spills, remove_session_spill, spill_dir_for_session, spill_root};
pub use streaming::KickoffResult;
pub use subagent::{
    adapt_event, inherit_parent_history, make_subagent_factory, new_subagent_session,
    SubagentBuildFn,
};
pub use trace::{new_trace_id, Trace, TraceHandle};