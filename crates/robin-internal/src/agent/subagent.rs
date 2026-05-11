/// subagent.rs — Subagent factory + adapter.
///
/// Mirrors Go's subagent.go. Wires the per-Runtime task tool: constructs a
/// `tools::SubagentFactory` that builds a fresh agent::Runtime for the named
/// subagent, sets its parent (for event forwarding) and depth (for the
/// recursion cap), and adapts `Runtime::run_sync` to `tools::SubagentRunner`
/// so the task tool can drain it.
use std::sync::Arc;

use crate::config::config::Config;
use crate::session::session::Session;
use crate::tools::task::{AgentEventLike, SubagentFactory, SubagentRunner};

use super::builder::{build_runtime_for_agent, RuntimeDeps, RuntimeInputs};
use super::runtime::{AgentEvent, AgentEventType, Runtime};

// ── SubagentBuildFn ───────────────────────────────────────────────────────────

/// Constructs the per-subagent `RuntimeInputs` given the resolved `AgentConfig`.
/// Each call site provides this closure because tool-registry construction
/// (RegisterCoreTools / MCP / send_message) is environment-specific.
///
/// Implementations MUST:
///   - Build a fresh `Executor` for the subagent (workspace = a.workspace)
///   - Resolve the LLM provider from `a.model`
///   - Create a fresh in-memory Session via `new_subagent_session`
///   - Set `ingest_source` to `""` (subagents are short-lived; no Cortex ingest)
///   - Set `compaction` to whatever the call site uses for this agent
pub type SubagentBuildFn =
    Box<dyn Fn(&crate::config::config::AgentConfig) -> anyhow::Result<RuntimeInputs> + Send + Sync>;

// ── make_subagent_factory ─────────────────────────────────────────────────────

/// Returns a `tools::SubagentFactory` that builds a Runtime for the named
/// subagent and adapts it to `tools::SubagentRunner`. Enforces the recursion
/// depth cap before constructing anything.
///
/// `parent` is the invoking Runtime — its `depth` is captured so the cap check
/// (`parent.depth + 1 <= max_agent_depth()`) fires before `build_runtime_for_agent`.
///
/// `cfg` is the live config so `eligible_subagents()` / `get_agent()` reflect
/// current registered subagents (config hot-reload safe by reading through the
/// Arc<Config> pointer at factory-call time, not at registration time).
pub fn make_subagent_factory(
    cfg: Arc<Config>,
    deps: RuntimeDeps,
    build_inputs: SubagentBuildFn,
    parent: Arc<Runtime>,
) -> SubagentFactory {
    let deps = Arc::new(deps);
    let build_inputs = Arc::new(build_inputs);

    Box::new(move |agent_id: &str, parent_depth: i32| {
        let max_depth = parent.max_agent_depth();
        if (parent_depth + 1) as usize > max_depth {
            return Err(anyhow::anyhow!(
                "subagent depth limit {} reached",
                max_depth
            ));
        }

        let a = match cfg.get_agent(agent_id) {
            Some(a) => a,
            None => {
                return Err(anyhow::anyhow!(
                    "subagent {:?} not found in config",
                    agent_id
                ))
            }
        };

        if !a.subagent {
            return Err(anyhow::anyhow!(
                "agent {:?} is not registered as a subagent",
                agent_id
            ));
        }

        let inputs = (build_inputs)(&a)
            .map_err(|e| anyhow::anyhow!("subagent {:?}: build inputs: {}", agent_id, e))?;

        // InheritContext: pre-populate the subagent's fresh session with copies of
        // parent's session entries. Done before build_runtime_for_agent so the
        // static prompt isn't accidentally rebuilt from a half-populated session.
        if a.inherit_context {
            if let Some(sub_sess) = &inputs.session {
                inherit_parent_history(sub_sess, &parent.session);
            }
        }

        let mut rt = build_runtime_for_agent(
            // We need to reconstruct a RuntimeDeps since Arc<RuntimeDeps> isn't Clone.
            // Clone the fields that are Clone, share Arcs for the rest.
            RuntimeDeps {
                permission: deps.permission.clone(),
                config: deps.config.clone(),
                calibrator_store: deps.calibrator_store.clone(),
                agent_loop: deps.agent_loop.clone(),
            },
            inputs,
            &a,
        )
        .map_err(|e| anyhow::anyhow!("subagent {:?}: build runtime: {}", agent_id, e))?;

        rt.depth = parent_depth + 1;
        // Wire the subagent's parent_events to the parent runtime's channel.
        // The parent runtime's run_inner holds an `mpsc::Sender<AgentEvent>`; we
        // can't capture that here directly (it's not stored on Runtime). Instead,
        // the subagent adapter forwards events by storing a weak reference to the
        // parent via parent_agent_id so callers can match forwarded events.
        rt.parent_agent_id = parent.agent_id.clone();

        let rt = Arc::new(rt);
        Ok(Box::new(SubagentRunnerAdapter { rt }) as Box<dyn SubagentRunner>)
    })
}

// ── inherit_parent_history ────────────────────────────────────────────────────

/// Copies the parent session's current view (post any compaction) into the
/// destination subagent session, so the destination ends up with the same
/// chain of entries. ToolCallID linkage between tool_call and tool_result
/// entries is preserved by keeping the original IDs intact.
///
/// The FIRST inherited entry gets its `parent_id` cleared so the subagent's
/// empty leaf doesn't cause Append to leave a dangling pointer to a parent
/// entry that exists only in the parent's session.
pub fn inherit_parent_history(dst: &Session, src: &Session) {
    let view = src.view();
    for (i, mut e) in view.into_iter().enumerate() {
        if i == 0 {
            e.parent_id = String::new();
        }
        dst.append(e);
    }
}

// ── SubagentRunnerAdapter ─────────────────────────────────────────────────────

/// Satisfies `tools::SubagentRunner` by adapting `Runtime::run_sync`.
///
/// The adapter runs the subagent synchronously (blocking the calling thread via
/// `tokio::task::block_in_place`) and converts the collected text into
/// `AgentEventLike` events.
pub struct SubagentRunnerAdapter {
    rt: Arc<Runtime>,
}

impl SubagentRunner for SubagentRunnerAdapter {
    fn run(&self, prompt: String) -> anyhow::Result<std::sync::mpsc::Receiver<AgentEventLike>> {
        let rt = Arc::clone(&self.rt);

        // Run the subagent on a new Tokio runtime (the task tool is called
        // from a synchronous context — the parent runtime's Executor::execute).
        let (tx, rx) = std::sync::mpsc::channel::<AgentEventLike>();

        let handle = std::thread::spawn(move || {
            let tokio_rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build subagent tokio runtime");

            tokio_rt.block_on(async move {
                let cancel = tokio_util::sync::CancellationToken::new();
                let mut agent_rx = match rt.run(cancel, prompt, vec![]).await {
                    Ok(rx) => rx,
                    Err(e) => {
                        let _ = tx.send(AgentEventLike {
                            event_type: AgentEventType::Error as i32,
                            text: String::new(),
                            done: false,
                            aborted: false,
                            err: Some(e.to_string()),
                        });
                        return;
                    }
                };

                while let Some(ev) = agent_rx.recv().await {
                    let like = adapt_event(ev);
                    let done = like.done || like.aborted;
                    let _ = tx.send(like);
                    if done {
                        break;
                    }
                }
            });
        });

        // Join the spawned thread so we can propagate any panic.
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("subagent thread panicked"))?;

        Ok(rx)
    }
}

// ── adapt_event ───────────────────────────────────────────────────────────────

/// Translates an `AgentEvent` into the `AgentEventLike` shape that `TaskTool`
/// understands. Only the fields `TaskTool` actually inspects are filled.
pub fn adapt_event(ev: AgentEvent) -> AgentEventLike {
    let done = ev.event_type == AgentEventType::Done;
    let aborted = ev.event_type == AgentEventType::Aborted;
    let err_str = ev.error.as_ref().map(|e| e.to_string());

    AgentEventLike {
        event_type: ev.event_type as i32,
        text: ev.text,
        done,
        aborted,
        err: err_str,
    }
}

// ── new_subagent_session ──────────────────────────────────────────────────────

/// Returns a fresh in-memory Session for a subagent run. Centralised here so
/// all call sites build subagent sessions the same way.
///
/// `set_store` is NOT called — subagent sessions are ephemeral and do NOT
/// write JSONL to disk. The parent's session is the durable record; the
/// subagent's transcript lives only in memory and is lost on completion.
pub fn new_subagent_session(agent_id: &str) -> Arc<Session> {
    Arc::new(Session::new(agent_id, "subagent"))
}

#[cfg(test)]
#[path = "subagent_test.rs"]
mod subagent_test;