/// dispatch_test.rs — Tests for dispatch_tool paths (permission, abort, errors).
///
/// Mirrors Go's dispatch_test.go. These tests use tokio because dispatch_tool
/// is an async function.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::llm::ToolCall;
use crate::session::session::{EntryType, Session, ToolCallData, ToolResultData};
use crate::tools::permission::{Decision, DecisionBehavior, PermissionChecker};
use crate::tools::tool::{Executor, NoopExecutor, Tool, ToolResult};

use crate::agent::test_support::{minimal_runtime, ImmediateLLM};
use crate::agent::runtime::Runtime;
use crate::agent::trace::Trace;

fn sample_tool_call() -> ToolCall {
    ToolCall {
        id: "tc_1".to_owned(),
        name: "read_file".to_owned(),
        input: serde_json::json!({"path": "/tmp/x"}),
    }
}

// ── FakeExecutor ──────────────────────────────────────────────────────────────

struct FakeExecutor {
    pub called: AtomicBool,
    pub result: ToolResult,
}

impl FakeExecutor {
    fn with_output(output: &str) -> Self {
        Self {
            called: AtomicBool::new(false),
            result: ToolResult::ok(output),
        }
    }
}

impl Executor for FakeExecutor {
    fn execute(&self, _name: &str, _input: Value) -> anyhow::Result<ToolResult> {
        self.called.store(true, Ordering::SeqCst);
        Ok(self.result.clone())
    }

    fn tool_defs(&self) -> Vec<crate::llm::ToolDef> {
        vec![]
    }

    fn names(&self) -> Vec<String> {
        vec![]
    }

    fn get(&self, _name: &str) -> Option<Arc<dyn Tool>> {
        None
    }
}

// ── FakeChecker ───────────────────────────────────────────────────────────────

struct FakeChecker {
    pub decision: Decision,
}

impl PermissionChecker for FakeChecker {
    fn check(&self, _agent_id: &str, _tool_name: &str, _input: &Value) -> Decision {
        self.decision.clone()
    }

    fn filter_tool_defs(&self, defs: &[crate::llm::ToolDef], _agent_id: &str) -> Vec<crate::llm::ToolDef> {
        defs.to_vec()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_dispatch_tool_clean_result() {
    let exec = Arc::new(FakeExecutor::with_output("hello"));
    let rt = Arc::new(Runtime {
        agent_id: "test_agent".to_owned(),
        tools: exec.clone() as Arc<dyn Executor>,
        session: Arc::new(Session::new("test_agent", "test_key")),
        llm: Arc::new(crate::agent::test_support::NeverCalledLLM),
        ..minimal_runtime()
    });

    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let tr = Trace::new("test", "model");
    let ctx = CancellationToken::new();

    let (result, aborted) = rt.dispatch_tool(&ctx, &tx, &sample_tool_call(), 0, &tr).await;

    assert!(!aborted);
    assert_eq!(result.output, "hello");
    assert!(result.error.is_empty());
    assert!(exec.called.load(Ordering::SeqCst));

    // Session should have tool_call + tool_result
    let entries: Vec<_> = rt.session.view();
    let last_two: Vec<_> = entries.iter().rev().take(2).collect::<Vec<_>>().into_iter().rev().collect();
    assert_eq!(last_two[0].entry_type, EntryType::ToolCall);
    assert_eq!(last_two[1].entry_type, EntryType::ToolResult);
    let tr_data: ToolResultData = serde_json::from_str(last_two[1].data.get()).unwrap();
    assert_eq!(tr_data.output, "hello");
    assert!(!tr_data.is_error);
}

#[tokio::test]
async fn test_dispatch_tool_permission_denied() {
    let exec = Arc::new(FakeExecutor::with_output("should not run"));
    let checker = Arc::new(FakeChecker {
        decision: Decision::deny("not allowed for agent"),
    });
    let rt = Arc::new(Runtime {
        agent_id: "test_agent".to_owned(),
        tools: exec.clone() as Arc<dyn Executor>,
        permission: Some(checker as Arc<dyn PermissionChecker>),
        session: Arc::new(Session::new("test_agent", "test_key")),
        llm: Arc::new(crate::agent::test_support::NeverCalledLLM),
        ..minimal_runtime()
    });

    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let tr = Trace::new("test", "model");
    let ctx = CancellationToken::new();

    let (result, aborted) = rt.dispatch_tool(&ctx, &tx, &sample_tool_call(), 0, &tr).await;

    assert!(!aborted);
    assert!(!result.error.is_empty());
    assert!(result.error.contains("not allowed"));
    assert!(!exec.called.load(Ordering::SeqCst), "execute must not run when denied");
}

#[tokio::test]
async fn test_dispatch_tool_pre_execute_abort() {
    let exec = Arc::new(FakeExecutor::with_output("should not run"));
    let rt = Arc::new(Runtime {
        agent_id: "test_agent".to_owned(),
        tools: exec.clone() as Arc<dyn Executor>,
        session: Arc::new(Session::new("test_agent", "test_key")),
        llm: Arc::new(crate::agent::test_support::NeverCalledLLM),
        ..minimal_runtime()
    });

    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let tr = Trace::new("test", "model");
    let ctx = CancellationToken::new();
    ctx.cancel(); // already cancelled

    let (result, aborted) = rt.dispatch_tool(&ctx, &tx, &sample_tool_call(), 0, &tr).await;

    assert!(aborted);
    assert!(result.error.contains("aborted"));
    assert!(!exec.called.load(Ordering::SeqCst), "execute must not run when pre-cancelled");
}