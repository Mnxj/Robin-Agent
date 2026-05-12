/// test_support.rs — Shared test helpers for the agent module.
///
/// Provides minimal `Runtime` constructors and fake LLM/Executor implementations
/// used across multiple agent test files.
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc;

use crate::config::config::AgentLoopConfig;
use crate::llm::{ChatEvent, ChatRequest, Diagnostic, ModelInfo, ToolDef};
use crate::session::session::Session;
use crate::tools::tool::{Executor, NoopExecutor, Tool, ToolResult};

use super::runtime::Runtime;

// ── MinimalRuntime ────────────────────────────────────────────────────────────

/// Constructs a Runtime with all defaults/no-ops. Tests set individual fields
/// they need; everything else is a safe zero value.
pub fn minimal_runtime() -> Runtime {
    Runtime {
        llm: Arc::new(NeverCalledLLM),
        tools: Arc::new(NoopExecutor),
        session: Arc::new(Session::new("test", "test")),
        agent_id: String::new(),
        agent_name: String::new(),
        model: String::new(),
        reasoning: crate::llm::ReasoningMode::Off,
        workspace: String::new(),
        max_turns: 0,
        agent_loop: AgentLoopConfig::default(),
        system_prompt: String::new(),
        compaction: None,
        provider: String::new(),
        fallback_model: String::new(),
        context_window: 0,
        static_system_prompt: String::new(),
        permission: None,
        depth: 0,
        parent_events: None,
        parent_agent_id: String::new(),
        ingest_source: String::new(),
        calibrator_store: None,
        calibrator: std::sync::Mutex::new(None),
        touched_files: std::sync::Mutex::new(Vec::new()),
        memory_manager: None,
    }
}

// ── NeverCalledLLM ────────────────────────────────────────────────────────────

/// LLMProvider that panics if called. Used in tests that only exercise non-LLM
/// code paths (e.g., depth/concurrency checks).
pub struct NeverCalledLLM;

#[async_trait::async_trait]
impl crate::llm::LLMProvider for NeverCalledLLM {
    async fn chat_stream(
        &self,
        _req: ChatRequest,
    ) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        panic!("NeverCalledLLM::chat_stream should not be called in this test")
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn normalize_tool_schema(
        &self,
        defs: Vec<ToolDef>,
    ) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        (defs, vec![])
    }
}

// ── ImmediateLLM ──────────────────────────────────────────────────────────────

/// LLMProvider that immediately emits a configurable sequence of events.
pub struct ImmediateLLM {
    pub events: Vec<ChatEvent>,
}

impl ImmediateLLM {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            events: vec![
                ChatEvent {
                    event_type: crate::llm::EventType::TextDelta,
                    text: text.into(),
                    ..Default::default()
                },
                ChatEvent {
                    event_type: crate::llm::EventType::Done,
                    ..Default::default()
                },
            ],
        }
    }

    pub fn done() -> Self {
        Self {
            events: vec![ChatEvent {
                event_type: crate::llm::EventType::Done,
                ..Default::default()
            }],
        }
    }
}

#[async_trait::async_trait]
impl crate::llm::LLMProvider for ImmediateLLM {
    async fn chat_stream(
        &self,
        _req: ChatRequest,
    ) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        let (tx, rx) = mpsc::channel(self.events.len() + 1);
        for ev in &self.events {
            let _ = tx.send(ev.clone()).await;
        }
        Ok(rx)
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn normalize_tool_schema(
        &self,
        defs: Vec<ToolDef>,
    ) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        (defs, vec![])
    }
}

// ── ClassifyExecutor ─────────────────────────────────────────────────────────

/// Executor that classifies tools as safe/unsafe by name lookup.
pub struct ClassifyExecutor {
    pub safe: std::collections::HashMap<String, bool>,
    pub panics: std::collections::HashMap<String, bool>,
}

impl Executor for ClassifyExecutor {
    fn execute(&self, _name: &str, _input: Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::ok(""))
    }

    fn tool_defs(&self) -> Vec<ToolDef> {
        vec![]
    }

    fn names(&self) -> Vec<String> {
        vec![]
    }

    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let safe = *self.safe.get(name)?;
        let do_panic = self.panics.get(name).copied().unwrap_or(false);
        Some(Arc::new(ClassifyTool {
            name: name.to_owned(),
            safe,
            do_panic,
        }))
    }
}

pub struct ClassifyTool {
    pub name: String,
    pub safe: bool,
    pub do_panic: bool,
}

impl Tool for ClassifyTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        ""
    }

    fn parameters(&self) -> Value {
        serde_json::json!({})
    }

    fn execute(&self, _input: Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::ok(""))
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        if self.do_panic {
            panic!("test-induced panic");
        }
        self.safe
    }
}
