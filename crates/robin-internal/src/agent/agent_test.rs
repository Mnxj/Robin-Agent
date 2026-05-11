/// agent_test.rs — Integration tests for the agent run loop.
///
/// Mirrors Go's agent_test.go. Tests the full agent loop with fake LLM
/// providers and fake executors.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::llm::{ChatEvent, ChatRequest, Diagnostic, EventType, ModelInfo, ToolCall, ToolDef};
use crate::session::session::{
    assistant_message_entry, tool_call_entry, tool_result_entry, user_message_entry,
    EntryType, Session, ToolCallData, ToolResultData,
};
use crate::tools::tool::{Executor, NoopExecutor, Tool, ToolResult};

use crate::agent::runtime::{AgentEvent, AgentEventType, Runtime};
use crate::agent::test_support::minimal_runtime;

// ── ImmediateLLM variant — scripted per-call ──────────────────────────────────

/// LLMProvider that returns a pre-scripted sequence of events per call.
struct ScriptedLLM {
    calls: AtomicI32,
    first_events: Vec<ChatEvent>,
    subsequent_events: Vec<ChatEvent>,
}

impl ScriptedLLM {
    fn text_then_done(text: &str) -> Self {
        let events = vec![
            ChatEvent {
                event_type: EventType::TextDelta,
                text: text.to_owned(),
                ..Default::default()
            },
            ChatEvent {
                event_type: EventType::Done,
                ..Default::default()
            },
        ];
        Self {
            calls: AtomicI32::new(0),
            first_events: events.clone(),
            subsequent_events: events,
        }
    }

    fn tool_call_then_done(tc: ToolCall, final_text: &str) -> Self {
        let first_events = vec![
            ChatEvent {
                event_type: EventType::ToolCallStart,
                tool_call: Some(tc.clone()),
                ..Default::default()
            },
            ChatEvent {
                event_type: EventType::ToolCallDone,
                tool_call: Some(tc),
                ..Default::default()
            },
            ChatEvent {
                event_type: EventType::Done,
                ..Default::default()
            },
        ];
        let subsequent_events = vec![
            ChatEvent {
                event_type: EventType::TextDelta,
                text: final_text.to_owned(),
                ..Default::default()
            },
            ChatEvent {
                event_type: EventType::Done,
                ..Default::default()
            },
        ];
        Self {
            calls: AtomicI32::new(0),
            first_events,
            subsequent_events,
        }
    }
}

#[async_trait::async_trait]
impl crate::llm::LLMProvider for ScriptedLLM {
    async fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let events = if n == 0 {
            self.first_events.clone()
        } else {
            self.subsequent_events.clone()
        };
        let (tx, rx) = mpsc::channel(events.len() + 1);
        for ev in events {
            let _ = tx.send(ev).await;
        }
        Ok(rx)
    }

    fn models(&self) -> Vec<ModelInfo> { vec![] }

    fn normalize_tool_schema(&self, defs: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        (defs, vec![])
    }
}

// ── MockTool ──────────────────────────────────────────────────────────────────

struct MockTool {
    name: String,
    output: String,
    is_safe: bool,
}

impl Tool for MockTool {
    fn name(&self) -> &str { &self.name }
    fn description(&self) -> &str { "mock tool" }
    fn parameters(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    fn execute(&self, _input: Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::ok(&self.output))
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool { self.is_safe }
}

// ── SimpleRegistry ────────────────────────────────────────────────────────────

struct SimpleRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl SimpleRegistry {
    fn new(tools: Vec<Arc<dyn Tool>>) -> Self {
        Self { tools }
    }
}

impl Executor for SimpleRegistry {
    fn execute(&self, name: &str, input: Value) -> anyhow::Result<ToolResult> {
        for t in &self.tools {
            if t.name() == name {
                return t.execute(input);
            }
        }
        Ok(ToolResult::err(format!("unknown tool: {}", name)))
    }

    fn tool_defs(&self) -> Vec<ToolDef> {
        self.tools.iter().map(|t| ToolDef {
            name: t.name().to_owned(),
            description: t.description().to_owned(),
            parameters: t.parameters(),
        }).collect()
    }

    fn names(&self) -> Vec<String> {
        self.tools.iter().map(|t| t.name().to_owned()).collect()
    }

    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_runtime_run_text_response() {
    let llm = Arc::new(ScriptedLLM::text_then_done("Hello world!"));
    let rt = Arc::new(Runtime {
        llm: llm as Arc<dyn crate::llm::LLMProvider>,
        tools: Arc::new(NoopExecutor),
        session: Arc::new(Session::new("test-agent", "test-key")),
        model: "mock-model".to_owned(),
        workspace: tempfile::tempdir().unwrap().path().to_string_lossy().into_owned(),
        max_turns: 5,
        ..minimal_runtime()
    });

    let ctx = CancellationToken::new();
    let mut rx = rt.run(ctx, "hi".to_owned(), vec![]).await.unwrap();

    let mut text_parts = Vec::new();
    let mut got_done = false;
    while let Some(ev) = rx.recv().await {
        match ev.event_type {
            AgentEventType::TextDelta => text_parts.push(ev.text),
            AgentEventType::Done => got_done = true,
            AgentEventType::Error => panic!("unexpected error: {:?}", ev.error),
            _ => {}
        }
    }

    assert_eq!(text_parts.join(""), "Hello world!");
    assert!(got_done);
}

#[tokio::test]
async fn test_runtime_run_with_tool_calls() {
    let tc = ToolCall {
        id: "tc_1".to_owned(),
        name: "read_file".to_owned(),
        input: serde_json::json!({"path": "/tmp/test.txt"}),
    };
    let llm = Arc::new(ScriptedLLM::tool_call_then_done(tc, "File contents: hello"));
    let exec = Arc::new(SimpleRegistry::new(vec![
        Arc::new(MockTool {
            name: "read_file".to_owned(),
            output: "hello".to_owned(),
            is_safe: false,
        }) as Arc<dyn Tool>,
    ]));

    let rt = Arc::new(Runtime {
        llm: llm as Arc<dyn crate::llm::LLMProvider>,
        tools: exec as Arc<dyn Executor>,
        session: Arc::new(Session::new("test-agent", "test-key")),
        model: "mock-model".to_owned(),
        workspace: tempfile::tempdir().unwrap().path().to_string_lossy().into_owned(),
        max_turns: 5,
        ..minimal_runtime()
    });

    let ctx = CancellationToken::new();
    let mut rx = rt.run(ctx, "read test.txt".to_owned(), vec![]).await.unwrap();

    let mut got_tool_result = false;
    let mut got_done = false;
    while let Some(ev) = rx.recv().await {
        match ev.event_type {
            AgentEventType::ToolResult => {
                got_tool_result = true;
                assert_eq!(ev.tool_call.as_ref().unwrap().name, "read_file");
            }
            AgentEventType::Done => got_done = true,
            AgentEventType::Error => panic!("unexpected error"),
            _ => {}
        }
    }

    assert!(got_tool_result, "should have received tool result");
    assert!(got_done, "should have received done event");
}

#[tokio::test]
async fn test_runtime_run_sync() {
    let llm = Arc::new(ScriptedLLM::text_then_done("Hello world!"));
    let rt = Arc::new(Runtime {
        llm: llm as Arc<dyn crate::llm::LLMProvider>,
        tools: Arc::new(NoopExecutor),
        session: Arc::new(Session::new("test-agent", "test-key")),
        model: "mock-model".to_owned(),
        workspace: tempfile::tempdir().unwrap().path().to_string_lossy().into_owned(),
        max_turns: 5,
        ..minimal_runtime()
    });

    let ctx = CancellationToken::new();
    let text = rt.run_sync(ctx, "hi".to_owned(), vec![]).await.unwrap();
    assert_eq!(text, "Hello world!");
}

#[tokio::test]
async fn test_runtime_run_abort_via_cancel() {
    use std::time::Duration;

    // LLM that never finishes: keeps the sender alive, blocking the receiver forever.
    struct HangingLLM;
    #[async_trait::async_trait]
    impl crate::llm::LLMProvider for HangingLLM {
        async fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
            let (tx, rx) = mpsc::channel(1);
            // Keep the sender alive in a background task so recv() blocks forever
            // (until the sender is dropped when the task completes).
            tokio::spawn(async move {
                // Block for up to 5 seconds, holding the sender open.
                tokio::time::sleep(Duration::from_secs(5)).await;
                drop(tx); // silence unused warning
            });
            Ok(rx)
        }
        fn models(&self) -> Vec<ModelInfo> { vec![] }
        fn normalize_tool_schema(&self, defs: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<Diagnostic>) {
            (defs, vec![])
        }
    }

    let rt = Arc::new(Runtime {
        llm: Arc::new(HangingLLM),
        tools: Arc::new(NoopExecutor),
        session: Arc::new(Session::new("test-agent", "test-key")),
        model: "mock-model".to_owned(),
        max_turns: 5,
        ..minimal_runtime()
    });

    let ctx = CancellationToken::new();
    let cancel = ctx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
    });

    let mut rx = rt.run(ctx, "hi".to_owned(), vec![]).await.unwrap();
    let mut saw_aborted = false;
    while let Some(ev) = rx.recv().await {
        if ev.event_type == AgentEventType::Aborted {
            saw_aborted = true;
        }
    }
    assert!(saw_aborted, "expected EventAborted after context cancel");
}

// ── assemble_messages tests (mirrors agent_test.go's inline tests) ────────────

#[test]
fn test_assemble_messages_compaction_continuation_directive() {
    use crate::session::session::compaction_entry;
    use crate::agent::context::assemble_messages;

    let sess = Arc::new(Session::new("test-agent", "test-key"));
    sess.append(user_message_entry("first user msg"));
    sess.append(assistant_message_entry("first reply"));
    sess.append(compaction_entry(
        "User asked about Wasm; we recommended Extism. They then asked for details on how it works.",
        "",
        "",
        "test-model",
        0,
        0,
        2,
    ));

    let msgs = assemble_messages(&sess.view());
    assert!(!msgs.is_empty());

    let summary_msg = msgs.iter().find(|m| m.content.contains("Previous conversation summary"));
    assert!(summary_msg.is_some(), "compaction entry must produce a user message");
    let summary = &summary_msg.unwrap().content;
    assert!(summary.contains("Wasm"), "summary text must be present");
    assert!(summary.contains("Resume directly"), "continuation directive must instruct resume");
    assert!(
        summary.contains("do not acknowledge the summary"),
        "continuation directive must forbid restarting"
    );
}

#[test]
fn test_assemble_messages_mid_history_orphans_are_rescued() {
    use crate::agent::context::assemble_messages;

    let sess = Arc::new(Session::new("a", "k"));
    sess.append(user_message_entry("go"));
    sess.append(tool_call_entry("tc_0", "noop", b"{}"));
    sess.append(tool_call_entry("tc_1", "noop", b"{}"));
    sess.append(tool_call_entry("tc_2", "noop", b"{}"));
    sess.append(tool_result_entry("tc_0", "ok", "", vec![]));
    sess.append(user_message_entry("continue"));

    let msgs = assemble_messages(&sess.view());

    // Walk msgs: every assistant.tool_calls must have matching tool_results.
    for (i, m) in msgs.iter().enumerate() {
        if m.tool_calls.is_empty() {
            continue;
        }
        let mut expected: std::collections::HashSet<String> =
            m.tool_calls.iter().map(|tc| tc.id.clone()).collect();
        let mut j = i + 1;
        while j < msgs.len() && msgs[j].role == "user" && !msgs[j].tool_call_id.is_empty() {
            expected.remove(&msgs[j].tool_call_id);
            j += 1;
        }
        assert!(
            expected.is_empty(),
            "assistant at idx {} has unpaired tool_calls: {:?}",
            i,
            expected
        );
    }
}