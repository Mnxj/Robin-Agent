/// partition.rs — Tool-call batch partitioner and concurrency cap.
///
/// Mirrors Go's partition.go. Groups consecutive concurrency-safe tool calls
/// into single parallel batches, and emits one-call batches for unsafe calls.
use std::sync::Arc;

use crate::llm::ToolCall;
use crate::tools::tool::Executor;

use super::runtime::Runtime;

// ── Batch ─────────────────────────────────────────────────────────────────────

/// A contiguous group of tool calls that may be dispatched together.
///
/// `concurrency_safe = true` means all calls in the batch can run in parallel.
/// `concurrency_safe = false` means this is a single-call batch that must run
/// alone.
#[derive(Debug, Clone)]
pub struct Batch {
    pub concurrency_safe: bool,
    pub calls: Vec<ToolCall>,
}

// ── partition_tool_calls ──────────────────────────────────────────────────────

/// Groups consecutive concurrency-safe calls into one batch each, and emits a
/// single-call batch for every unsafe call. Order is preserved both within and
/// across batches.
///
/// Tools not found in the executor are treated as unsafe (defensive). If a
/// tool's `is_concurrency_safe` panics, the recover treats it as unsafe.
pub fn partition_tool_calls(tcs: &[ToolCall], ex: &dyn Executor) -> Vec<Batch> {
    let mut out: Vec<Batch> = Vec::new();
    for tc in tcs {
        let safe = is_call_concurrency_safe(tc, ex);
        // Append to the previous safe batch if both are safe; otherwise start
        // a new batch. Unsafe calls always start their own batch (single-call).
        if safe {
            if let Some(last) = out.last_mut() {
                if last.concurrency_safe {
                    last.calls.push(tc.clone());
                    continue;
                }
            }
        }
        out.push(Batch {
            concurrency_safe: safe,
            calls: vec![tc.clone()],
        });
    }
    out
}

/// Looks up the tool and asks whether it is concurrency-safe. Treats panics
/// and missing tools as unsafe (defensive).
pub fn is_call_concurrency_safe(tc: &ToolCall, ex: &dyn Executor) -> bool {
    match ex.get(&tc.name) {
        Some(tool) => {
            // Use std::panic::catch_unwind to mirror Go's recover.
            // Note: the Tool trait requires Send + Sync so this is safe.
            let input = tc.input.clone();
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tool.is_concurrency_safe(&input)
            })) {
                Ok(safe) => safe,
                Err(_) => {
                    log::warn!(
                        "tool IsConcurrencySafe panicked; treating as unsafe: tool={}",
                        tc.name
                    );
                    false
                }
            }
        }
        None => false, // unknown tool → unsafe (dispatch will report the error)
    }
}

// ── maxToolConcurrency ────────────────────────────────────────────────────────

impl Runtime {
    /// Returns the cap on concurrent tool dispatch within a safe batch.
    ///
    /// Precedence:
    ///   1. `agent_loop.max_tool_concurrency` > 0 — config wins.
    ///   2. `ROBIN_MAX_TOOL_CONCURRENCY` env var > 0 — env fallback.
    ///   3. Default 10.
    pub fn max_tool_concurrency(&self) -> usize {
        if self.agent_loop.max_tool_concurrency > 0 {
            return self.agent_loop.max_tool_concurrency as usize;
        }
        if let Ok(v) = std::env::var("ROBIN_MAX_TOOL_CONCURRENCY") {
            if let Ok(n) = v.trim().parse::<usize>() {
                if n > 0 {
                    return n;
                }
            }
        }
        10
    }
}

#[cfg(test)]
#[path = "partition_test.rs"]
mod partition_test;