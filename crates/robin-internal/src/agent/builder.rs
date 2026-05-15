/// builder.rs — Runtime factory for AgentConfig.
///
/// Mirrors Go's builder.go. Centralises construction of a Runtime from
/// an AgentConfig + shared dependencies + per-call inputs.
use std::sync::Arc;

use crate::compaction::compaction::Manager as CompactionManager;
use crate::config::config::{AgentConfig, AgentLoopConfig, Config};
use crate::llm::{parse_provider_model, LLMProvider, ReasoningMode};
use crate::memory::Manager as MemoryManager;
use crate::session::session::Session;
use crate::tokens::persist::CalibratorStore;
use crate::tokens::tokens::Calibrator;
use crate::tools::permission::PermissionChecker;
use crate::tools::tool::Executor;

use super::context::{build_config_summary, build_static_system_prompt, load_agent_memory_files};
use super::runtime::Runtime;

// ── RuntimeDeps ───────────────────────────────────────────────────────────────

/// Long-lived dependencies that every Runtime in this process shares. Built
/// once at startup and reused for every Runtime construction (including
/// subagent runtimes built by the task tool factory).
#[derive(Clone)]
pub struct RuntimeDeps {
    /// Resolved permission checker for all agents (shared across runtimes).
    pub permission: Option<Arc<dyn PermissionChecker>>,
    /// Live config reference. Used during `build_runtime_for_agent` to
    /// pre-compute the configuration summary for the static system prompt.
    pub config: Option<Arc<Config>>,
    /// Per-(agentID, sessionKey) persistence layer for the token Calibrator.
    /// `None` disables persistence; in-memory learning still happens.
    pub calibrator_store: Option<Arc<CalibratorStore>>,
    /// The agent-loop config block (concurrency cap, depth cap,
    /// streaming-tools toggle). Copied into every Runtime built by
    /// `build_runtime_for_agent`.
    pub agent_loop: AgentLoopConfig,
}

impl Default for RuntimeDeps {
    fn default() -> Self {
        Self {
            permission: None,
            config: None,
            calibrator_store: None,
            agent_loop: AgentLoopConfig::default(),
        }
    }
}

// ── RuntimeInputs ─────────────────────────────────────────────────────────────

/// Per-Runtime-instance inputs that genuinely vary per call site.
pub struct RuntimeInputs {
    /// Resolved LLM provider for this agent's model.
    pub provider: Option<Arc<dyn LLMProvider>>,
    /// Tool executor (different per cron / chat / subagent path).
    pub tools: Option<Arc<dyn Executor>>,
    /// Session handle.
    pub session: Option<Arc<Session>>,
    /// Per-agent compaction manager (may be None for subagent paths).
    pub compaction: Option<Arc<CompactionManager>>,
    /// Controls whether this run writes to Cortex: "" | "chat" | "cron".
    pub ingest_source: String,
    /// Optional pre-computed skills index text to inject into the static prompt.
    pub skills_index: String,
    /// Optional pre-computed memory index text to inject into the static prompt.
    pub memory_index: String,
    /// Optional memory manager used for per-turn relevant-memory recall.
    pub memory_manager: Option<Arc<MemoryManager>>,
}

impl Default for RuntimeInputs {
    fn default() -> Self {
        Self {
            provider: None,
            tools: None,
            session: None,
            compaction: None,
            ingest_source: String::new(),
            skills_index: String::new(),
            memory_index: String::new(),
            memory_manager: None,
        }
    }
}

// ── BuildRuntimeForAgent ──────────────────────────────────────────────────────

/// Constructs a Runtime for the given AgentConfig using the supplied deps +
/// inputs. Centralises three patterns currently duplicated across call sites:
///
///   1. Parsing the model identifier (provider/model) for `Runtime.model`.
///   2. Parsing the reasoning mode (with default-to-off + warning on invalid).
///   3. Pre-computing the static system prompt (cached, built once).
///
/// Returns `(Runtime, error)`. Callers MUST check the error: the return is
/// reserved for future validation (e.g., "agent config requires X feature
/// this build doesn't have"). Discarding it and unwrapping would panic the
/// moment any validation lands.
pub fn build_runtime_for_agent(
    deps: RuntimeDeps,
    inputs: RuntimeInputs,
    a: &AgentConfig,
) -> anyhow::Result<Runtime> {
    let (provider_name, model_name) = parse_provider_model(&a.model);
    let provider_name = provider_name.to_owned();
    let model_name = model_name.to_owned();

    let reasoning = match parse_reasoning_mode(&a.reasoning) {
        Ok(r) => r,
        Err(e) => {
            log::error!(
                "invalid reasoning mode in agent config; defaulting to off: agent={} value={} err={}",
                a.id, a.reasoning, e
            );
            ReasoningMode::Off
        }
    };

    // Pre-compute the static portion of the system prompt so the per-turn hot
    // loop never reads config or rebuilds the skills/memory indices.
    let config_summary = deps
        .config
        .as_deref()
        .map(|c| build_config_summary(c))
        .unwrap_or_default();

    let tool_names: Vec<String> = inputs
        .tools
        .as_ref()
        .map(|ex| ex.names())
        .unwrap_or_default();

    let memory_files = load_agent_memory_files(&a.workspace);

    let static_prompt = build_static_system_prompt(
        &a.workspace,
        &a.system_prompt,
        &a.id,
        &a.name,
        &tool_names,
        &config_summary,
        &inputs.skills_index,
        &inputs.memory_index,
        &memory_files,
    );

    // Strip the provider prefix off FallbackModel so the runtime hands the
    // same provider client a bare model id on retry. Cross-provider fallback
    // isn't supported — Runtime.llm is one client; if the configured fallback
    // names a different provider it's a config bug, so we log and discard.
    let fallback_model = if !a.fallback_model.is_empty() {
        let (fb_provider, fb_model) = parse_provider_model(&a.fallback_model);
        if !fb_provider.is_empty() && fb_provider != provider_name {
            log::warn!(
                "fallbackModel ignored: cross-provider fallback not supported: agent={} primary_provider={} fallback={}",
                a.id, provider_name, a.fallback_model
            );
            String::new()
        } else {
            fb_model.to_owned()
        }
    } else {
        String::new()
    };

    // Build the Runtime. Use a placeholder Arc<dyn LLMProvider> if none is
    // provided (unit tests that don't call Run can omit it).
    let llm = inputs
        .provider
        .unwrap_or_else(|| Arc::new(NoopLLMProvider));

    let tools_arc = inputs
        .tools
        .unwrap_or_else(|| Arc::new(crate::tools::tool::NoopExecutor));

    let session = inputs
        .session
        .unwrap_or_else(|| Arc::new(Session::new(&a.id, "")));

    let rt = Runtime {
        llm,
        tools: tools_arc,
        session: Arc::clone(&session),
        agent_id: a.id.clone(),
        agent_name: a.name.clone(),
        model: model_name,
        fallback_model,
        context_window: a.context_window,
        provider: provider_name,
        reasoning,
        workspace: a.workspace.clone(),
        max_turns: a.max_turns,
        system_prompt: a.system_prompt.clone(),
        permission: deps.permission,
        compaction: inputs.compaction,
        ingest_source: inputs.ingest_source,
        agent_loop: deps.agent_loop,
        static_system_prompt: static_prompt,
        calibrator_store: deps.calibrator_store.clone(),
        depth: 0,
        parent_events: None,
        parent_agent_id: String::new(),
        calibrator: std::sync::Mutex::new(None),
        touched_files: std::sync::Mutex::new(Vec::new()),
        memory_manager: inputs.memory_manager,
    };

    // Seed the calibrator from prior (ratio, count) for this session so a
    // long session that's been split across many calls retains its learned
    // chars→tokens ratio. Skipped for subagent sessions (Session.key ==
    // "subagent") and when no store is configured.
    if let Some(store) = &deps.calibrator_store {
        let sess_key = &rt.session.key;
        if !sess_key.is_empty() && sess_key != "subagent" {
            let (ratio, count) = store.load(&a.id, sess_key);
            if count > 0 {
                let cal = Calibrator::new(); // returns Arc<Calibrator>
                cal.restore(ratio, count);
                *rt.calibrator.lock().unwrap() = Some(cal);
            }
        }
    }

    Ok(rt)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parses a reasoning mode string, returning `ReasoningMode::Off` by default.
fn parse_reasoning_mode(s: &str) -> anyhow::Result<ReasoningMode> {
    // Delegate to the LLM provider's canonical parser.
    crate::llm::parse_reasoning_mode(s).or_else(|_| {
        // Accept aliases used in Go's AgentConfig.
        match s.to_lowercase().as_str() {
            "none" | "on" | "always" | "auto" => Ok(ReasoningMode::Off),
            _ => anyhow::bail!("unknown reasoning mode: {:?}", s),
        }
    })
}

// ── NoopLLMProvider (placeholder for tests) ────────────────────────────────────

/// A do-nothing LLM provider used when no real provider is supplied.
struct NoopLLMProvider;

#[async_trait::async_trait]
impl crate::llm::LLMProvider for NoopLLMProvider {
    async fn chat_stream(
        &self,
        _req: crate::llm::ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<crate::llm::ChatEvent>> {
        anyhow::bail!("NoopLLMProvider: no LLM configured")
    }

    fn models(&self) -> Vec<crate::llm::ModelInfo> {
        vec![]
    }

    fn normalize_tool_schema(
        &self,
        defs: Vec<crate::llm::ToolDef>,
    ) -> (Vec<crate::llm::ToolDef>, Vec<crate::llm::Diagnostic>) {
        (defs, vec![])
    }
}

#[cfg(test)]
#[path = "builder_test.rs"]
mod builder_test;
