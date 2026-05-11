/// runtime.rs — The core agent think-act loop.
///
/// Mirrors Go's runtime.go. The Runtime struct drives: LLM calls, tool
/// dispatch, session management, compaction, and event streaming.
///
/// Because Rust lacks goroutines we use Tokio tasks and `mpsc` channels in
/// place of Go channels. The public API mirrors the Go one closely:
/// `run()` returns a `tokio::sync::mpsc::Receiver<AgentEvent>`.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::compaction::compaction::{CompactionResult, Manager as CompactionManager, Reason as CompactionReason};
use crate::compaction::overflow::is_context_overflow;
use crate::config::config::{AgentConfig, AgentLoopConfig};
use crate::llm::{
    join_system_prompt_parts, parse_provider_model, EventType as LLMEventType,
    ImageContent, Message, SystemPromptPart, ToolCall, ToolDef,
};
use crate::llm::{LLMProvider, NonStreamingProvider};
use crate::memory::{format_for_prompt as format_memory_prompt, Manager as MemoryManager};
use crate::llm::retry::is_retryable_model_error;
use crate::session::session::{ImageData, Session, SessionEntry};
use crate::tokens::persist::CalibratorStore;
use crate::tokens::tokens::{context_window_for, estimate, Calibrator};
use crate::tools::permission::PermissionChecker;
use crate::tools::tool::{Executor, ToolResult};

use super::context::{
    assemble_messages, build_dynamic_system_prompt_suffix, extract_path_from_input,
    format_date_line, prepend_post_compact_restore, prune_tool_results, SpillConfig,
    MAX_TOOL_RESULT_LEN,
};
use super::partition::{is_call_concurrency_safe, partition_tool_calls};
use super::trace::Trace;

// ── Event types ───────────────────────────────────────────────────────────────

/// Event type for agent events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentEventType {
    TextDelta = 0,
    ToolCallStart = 1,
    ToolResult = 2,
    Done = 3,
    Error = 4,
    Aborted = 5,
    CompactionStart = 6,
    CompactionDone = 7,
    CompactionSkipped = 8,
}

/// A single streaming event emitted by the agent.
#[derive(Debug, Default)]
pub struct AgentEvent {
    pub event_type: AgentEventType,
    /// Populated when this event was forwarded from a subagent.
    pub agent_id: String,
    pub text: String,
    pub tool_call: Option<ToolCall>,
    pub result: Option<ToolResult>,
    pub error: Option<anyhow::Error>,
    pub compaction: Option<CompactionResult>,
    pub usage: Option<crate::llm::Usage>,
}

impl Default for AgentEventType {
    fn default() -> Self {
        AgentEventType::Done
    }
}

// ── Runtime ───────────────────────────────────────────────────────────────────

/// The main agent think-act loop.
pub struct Runtime {
    pub llm: Arc<dyn LLMProvider>,
    pub tools: Arc<dyn Executor>,
    pub session: Arc<Session>,
    pub agent_id: String,
    pub agent_name: String,
    pub model: String,
    pub reasoning: crate::llm::ReasoningMode,
    pub workspace: String,
    pub max_turns: i32,
    pub agent_loop: AgentLoopConfig,
    pub system_prompt: String,
    pub compaction: Option<Arc<CompactionManager>>,
    pub provider: String,
    pub fallback_model: String,
    pub context_window: i64,
    /// Pre-computed cacheable static system prompt.
    pub static_system_prompt: String,
    pub permission: Option<Arc<dyn PermissionChecker>>,
    pub depth: i32,
    /// When non-None, this runtime is a subagent of `parent`.
    pub parent_events: Option<mpsc::Sender<AgentEvent>>,
    /// Human-readable parent agent ID (used to populate forwarded events).
    pub parent_agent_id: String,
    pub ingest_source: String,
    pub calibrator_store: Option<Arc<CalibratorStore>>,

    // Internal state, protected by mutexes.
    pub(crate) calibrator: Mutex<Option<Arc<Calibrator>>>,
    pub(crate) touched_files: Mutex<Vec<String>>,
    pub memory_manager: Option<Arc<MemoryManager>>,
}

impl Runtime {
    /// Emits an event to the current events channel and optionally forwards
    /// it to the parent's channel (with `agent_id` populated).
    fn emit_event(
        &self,
        tx: &mpsc::Sender<AgentEvent>,
        ev: AgentEvent,
    ) {
        // Forward a copy to parent (non-blocking).
        if let Some(parent_tx) = &self.parent_events {
            let mut fwd = AgentEvent {
                event_type: ev.event_type,
                agent_id: self.agent_id.clone(),
                text: ev.text.clone(),
                tool_call: ev.tool_call.clone(),
                result: ev.result.clone(),
                // Can't clone anyhow::Error; skip error forwarding to parent.
                error: None,
                compaction: ev.compaction.clone(),
                usage: ev.usage.clone(),
            };
            let _ = parent_tx.try_send(fwd);
        }
        let _ = tx.try_send(ev);
    }

    /// Records a file path as touched (for post-compact restore), deduping by
    /// moving an existing entry to the back.
    pub fn record_file_touch(&self, path: &str) {
        if path.is_empty() {
            return;
        }
        let mut guard = self.touched_files.lock().unwrap();
        if let Some(i) = guard.iter().position(|p| p == path) {
            guard.remove(i);
        }
        guard.push(path.to_owned());
    }

    /// Returns a snapshot of the touched-files list.
    pub fn snapshot_touched_files(&self) -> Vec<String> {
        self.touched_files.lock().unwrap().clone()
    }

    /// Determines whether the provider supports Anthropic-style caching.
    pub fn provider_supports_caching(&self) -> bool {
        self.provider == "anthropic"
    }

    /// Determines whether the provider supports mid-loop compaction cleanly.
    pub fn provider_supports_mid_loop_compaction(&self) -> bool {
        matches!(self.provider.as_str(), "anthropic" | "openai" | "gemini")
    }

    /// Runs the agent loop for a user message. Returns a channel of events.
    pub async fn run(
        self: &Arc<Self>,
        ctx: tokio_util::sync::CancellationToken,
        user_msg: String,
        images: Vec<ImageContent>,
    ) -> anyhow::Result<mpsc::Receiver<AgentEvent>> {
        let (tx, rx) = mpsc::channel(100);
        let runtime = Arc::clone(self);
        let tx_clone = tx.clone();

        tokio::spawn(async move {
            runtime.run_inner(ctx, user_msg, images, tx_clone).await;
        });

        Ok(rx)
    }

    async fn run_inner(
        self: &Arc<Self>,
        ctx: tokio_util::sync::CancellationToken,
        user_msg: String,
        images: Vec<ImageContent>,
        tx: mpsc::Sender<AgentEvent>,
    ) {
        let tr = Trace::new(&self.agent_id, &self.model);

        tr.mark("agent.run.start", &[
            ("user_msg_len".to_owned(), Value::from(user_msg.len())),
            ("images".to_owned(), Value::from(images.len())),
        ]);

        // Append user message to session.
        if images.is_empty() {
            self.session.append(crate::session::session::user_message_entry(&user_msg));
        } else {
            let img_data: Vec<ImageData> = images
                .iter()
                .map(|img| ImageData {
                    mime_type: img.mime_type.clone(),
                    data: B64.encode(&img.data),
                })
                .collect();
            self.session.append(
                crate::session::session::user_message_with_images_entry(&user_msg, img_data),
            );
        }

        let max_turns = if self.max_turns <= 0 { 25 } else { self.max_turns };
        let date_line = format_date_line(&chrono::Local::now());
        let recalled_memory = if let Some(memory) = &self.memory_manager {
            let entries = memory.search(&user_msg, 5).await;
            format_memory_prompt(&entries)
        } else {
            String::new()
        };

        let spill_cfg = SpillConfig {
            workspace: self.workspace.clone(),
            session_key: self.session.key.clone(),
        };

        'turn_loop: for turn in 0..max_turns {
            // Cancellation check.
            if ctx.is_cancelled() {
                self.emit_event(&tx, AgentEvent {
                    event_type: AgentEventType::Aborted,
                    ..Default::default()
                });
                return;
            }

            let dynamic_suffix = build_dynamic_system_prompt_suffix(&date_line, &recalled_memory);
            let static_text = self.static_system_prompt.clone();
            let mut parts = vec![SystemPromptPart { text: static_text.clone(), cache: true }];
            if !dynamic_suffix.is_empty() {
                parts.push(SystemPromptPart { text: dynamic_suffix.clone(), cache: false });
            }

            let history = self.session.view();
            let mut msgs = assemble_messages(&history);
            prune_tool_results(&mut msgs, MAX_TOOL_RESULT_LEN, &spill_cfg);

            let mut tool_defs = self.tools.tool_defs();
            if let Some(perm) = &self.permission {
                tool_defs = perm.filter_tool_defs(&tool_defs, &self.agent_id);
            }
            tool_defs.sort_by(|a, b| a.name.cmp(&b.name));
            let (tool_defs, _diags) = self.llm.normalize_tool_schema(tool_defs);

            // Wait for in-flight async compaction on first turn.
            if turn == 0 {
                if let Some(compaction) = &self.compaction {
                    if let Some(res) = compaction.wait_for_in_flight(&self.session, std::time::Duration::from_secs(8)).await {
                        if res.compacted {
                            self.emit_event(&tx, AgentEvent {
                                event_type: AgentEventType::CompactionDone,
                                compaction: Some(res),
                                ..Default::default()
                            });
                            let history2 = self.session.view();
                            msgs = assemble_messages(&history2);
                            prune_tool_results(&mut msgs, MAX_TOOL_RESULT_LEN, &spill_cfg);
                            msgs = prepend_post_compact_restore(msgs, &self.snapshot_touched_files());
                        }
                    }
                }
            }

            // Preventive compaction.
            let compaction_allowed = turn == 0 || self.provider_supports_mid_loop_compaction();
            if compaction_allowed {
                if let Some(compaction) = &self.compaction {
                    if !self.model.is_empty() {
                        let cal = {
                            let mut guard = self.calibrator.lock().unwrap();
                            if guard.is_none() {
                                *guard = Some(Calibrator::new());
                            }
                            guard.as_ref().unwrap().clone()
                        };
                        let est = cal.adjust(estimate(&msgs, &join_system_prompt_parts(&parts), &tool_defs));
                        let window = context_window_for(&self.model, self.context_window);
                        let threshold = if compaction.threshold > 0.0 { compaction.threshold } else { 0.6 };
                        let threshold_hit = window > 0 && est > (threshold * window as f64) as usize;
                        let msg_cap = compaction.message_cap;
                        let count_hit = msg_cap > 0 && msgs.len() > msg_cap as usize;
                        if threshold_hit || count_hit {
                            self.emit_event(&tx, AgentEvent {
                                event_type: AgentEventType::CompactionStart,
                                ..Default::default()
                            });
                            let res = compaction.maybe_compact(&self.session, CompactionReason::Preventive, "").await;
                            let res = res.unwrap_or_default();
                            if res.compacted {
                                self.emit_event(&tx, AgentEvent {
                                    event_type: AgentEventType::CompactionDone,
                                    compaction: Some(res),
                                    ..Default::default()
                                });
                                let history2 = self.session.view();
                                msgs = assemble_messages(&history2);
                                prune_tool_results(&mut msgs, MAX_TOOL_RESULT_LEN, &spill_cfg);
                                msgs = prepend_post_compact_restore(msgs, &self.snapshot_touched_files());
                            } else {
                                self.emit_event(&tx, AgentEvent {
                                    event_type: AgentEventType::CompactionSkipped,
                                    compaction: Some(res),
                                    ..Default::default()
                                });
                            }
                        }
                    }
                }
            }

            let req = crate::llm::ChatRequest {
                model: self.model.clone(),
                messages: msgs.clone(),
                tools: tool_defs.clone(),
                max_tokens: 8192,
                system_prompt_parts: parts.clone(),
                cache_last_message: self.provider_supports_caching(),
                reasoning: self.reasoning,
                ..Default::default()
            };

            // Call LLM.
            let mut stream = match self.llm.chat_stream(req.clone()).await {
                Ok(s) => s,
                Err(e) => {
                    // Reactive compaction on context overflow.
                    if is_context_overflow(&e) {
                        if let Some(compaction) = &self.compaction {
                            self.emit_event(&tx, AgentEvent {
                                event_type: AgentEventType::CompactionStart,
                                ..Default::default()
                            });
                            let res = compaction.maybe_compact(&self.session, CompactionReason::Reactive, "").await.unwrap_or_default();
                            if res.compacted {
                                self.emit_event(&tx, AgentEvent {
                                    event_type: AgentEventType::CompactionDone,
                                    compaction: Some(res),
                                    ..Default::default()
                                });
                                let history2 = self.session.view();
                                msgs = assemble_messages(&history2);
                                prune_tool_results(&mut msgs, MAX_TOOL_RESULT_LEN, &spill_cfg);
                                msgs = prepend_post_compact_restore(msgs, &self.snapshot_touched_files());
                                let mut req2 = req.clone();
                                req2.messages = msgs.clone();
                                match self.llm.chat_stream(req2).await {
                                    Ok(s) => { // reassign stream and continue
                                        // Process this stream below
                                        self.process_stream_and_dispatch(
                                            &ctx, &tx, s, &msgs, &parts, &tool_defs, &spill_cfg, turn, &tr,
                                        ).await;
                                        continue 'turn_loop;
                                    }
                                    Err(e2) => {
                                        self.emit_event(&tx, AgentEvent {
                                            event_type: AgentEventType::Error,
                                            error: Some(e2),
                                            ..Default::default()
                                        });
                                        return;
                                    }
                                }
                            } else {
                                self.emit_event(&tx, AgentEvent {
                                    event_type: AgentEventType::CompactionSkipped,
                                    compaction: Some(res),
                                    ..Default::default()
                                });
                            }
                        }
                    }
                    // Fallback model.
                    if !self.fallback_model.is_empty()
                        && self.fallback_model != req.model
                        && is_retryable_model_error(&e)
                    {
                        let mut req2 = req.clone();
                        req2.model = self.fallback_model.clone();
                        match self.llm.chat_stream(req2).await {
                            Ok(s) => {
                                self.process_stream_and_dispatch(
                                    &ctx, &tx, s, &msgs, &parts, &tool_defs, &spill_cfg, turn, &tr,
                                ).await;
                                continue 'turn_loop;
                            }
                            Err(e2) => {
                                self.emit_event(&tx, AgentEvent {
                                    event_type: AgentEventType::Error,
                                    error: Some(e2),
                                    ..Default::default()
                                });
                                return;
                            }
                        }
                    }
                    self.emit_event(&tx, AgentEvent {
                        event_type: AgentEventType::Error,
                        error: Some(e),
                        ..Default::default()
                    });
                    return;
                }
            };

            let outcome = self.process_stream_and_dispatch(
                &ctx, &tx, stream, &msgs, &parts, &tool_defs, &spill_cfg, turn, &tr,
            ).await;
            match outcome {
                LoopOutcome::Done(usage) => {
                    // Fire async compaction between turns.
                    if let Some(compaction) = &self.compaction {
                        if !self.model.is_empty() && !compaction.has_in_flight(&self.session) {
                            let cal = {
                                let guard = self.calibrator.lock().unwrap();
                                guard.clone()
                            };
                            if let Some(cal) = cal {
                                let est = cal.adjust(estimate(&msgs, &join_system_prompt_parts(&parts), &tool_defs));
                                let window = context_window_for(&self.model, self.context_window);
                                let threshold = if compaction.threshold > 0.0 { compaction.threshold } else { 0.6 };
                                let preempt_token = window > 0 && (est as f64) > 0.8 * threshold * window as f64;
                                let preempt_count = compaction.message_cap > 0
                                    && msgs.len() > (0.8 * compaction.message_cap as f64) as usize;
                                if preempt_token || preempt_count {
                                    let compaction = Arc::clone(compaction);
                                    let sess = Arc::clone(&self.session);
                                    tokio::spawn(async move {
                                        compaction.maybe_compact_async(sess, CompactionReason::Preventive);
                                    });
                                }
                            }
                        }
                    }
                    self.emit_event(&tx, AgentEvent {
                        event_type: AgentEventType::Done,
                        usage,
                        ..Default::default()
                    });
                    tr.summary();
                    return;
                }
                LoopOutcome::Continue => {
                    // Loop for next LLM turn.
                }
                LoopOutcome::Aborted => {
                    tr.summary();
                    return;
                }
                LoopOutcome::Error(e) => {
                    self.emit_event(&tx, AgentEvent {
                        event_type: AgentEventType::Error,
                        error: Some(e),
                        ..Default::default()
                    });
                    tr.summary();
                    return;
                }
            }
        }

        // Exceeded max turns.
        self.emit_event(&tx, AgentEvent {
            event_type: AgentEventType::Error,
            error: Some(anyhow::anyhow!("agent exceeded maximum turns ({})", max_turns)),
            ..Default::default()
        });
        tr.summary();
    }

    /// Processes one LLM stream, dispatches tool calls, and returns what the
    /// outer loop should do next.
    async fn process_stream_and_dispatch(
        self: &Arc<Self>,
        ctx: &tokio_util::sync::CancellationToken,
        tx: &mpsc::Sender<AgentEvent>,
        mut stream: mpsc::Receiver<crate::llm::ChatEvent>,
        msgs: &[Message],
        parts: &[SystemPromptPart],
        tool_defs: &[ToolDef],
        spill_cfg: &SpillConfig,
        turn: i32,
        tr: &Arc<Trace>,
    ) -> LoopOutcome {
        let mut text_content = String::new();
        let mut last_usage: Option<crate::llm::Usage> = None;
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut got_first_token = false;

        // Collect the response, racing against context cancellation.
        loop {
            let maybe_event = tokio::select! {
                biased;
                _ = ctx.cancelled() => {
                    self.emit_event(tx, AgentEvent {
                        event_type: AgentEventType::Aborted,
                        ..Default::default()
                    });
                    return LoopOutcome::Aborted;
                }
                ev = stream.recv() => ev,
            };
            let event = match maybe_event {
                Some(e) => e,
                None => break, // stream closed
            };
            match event.event_type {
                LLMEventType::TextDelta => {
                    if !got_first_token {
                        got_first_token = true;
                    }
                    text_content.push_str(&event.text);
                    self.emit_event(tx, AgentEvent {
                        event_type: AgentEventType::TextDelta,
                        text: event.text,
                        ..Default::default()
                    });
                }
                LLMEventType::ToolCallStart => {
                    if !got_first_token {
                        got_first_token = true;
                    }
                    self.emit_event(tx, AgentEvent {
                        event_type: AgentEventType::ToolCallStart,
                        tool_call: event.tool_call.clone(),
                        ..Default::default()
                    });
                }
                LLMEventType::ToolCallDone => {
                    if let Some(tc) = event.tool_call {
                        tool_calls.push(tc);
                    }
                }
                LLMEventType::Done => {
                    if let Some(u) = event.usage {
                        last_usage = Some(u.clone());
                        // Update calibrator.
                        let cal = {
                            let mut guard = self.calibrator.lock().unwrap();
                            if guard.is_none() {
                                *guard = Some(Calibrator::new());
                            }
                            guard.as_ref().unwrap().clone()
                        };
                        let est = estimate(msgs, &join_system_prompt_parts(parts), tool_defs);
                        cal.update(u.input_tokens as usize, est);
                        if let Some(store) = &self.calibrator_store {
                            let (ratio, count) = cal.snapshot();
                            store.save(&self.agent_id, &self.session.key, ratio, count);
                        }
                    }
                }
                LLMEventType::Error => {
                    // Mid-stream failure: if we got a token and the provider
                    // implements NonStreamingProvider, retry non-streaming.
                    if got_first_token {
                        // Can't easily downcast Arc<dyn LLMProvider> to NonStreamingProvider;
                        // leave retry as not implemented here (provider-specific retry logic).
                    }
                    return LoopOutcome::Error(
                        event.error.as_deref()
                            .map(|e| anyhow::anyhow!("{}", e))
                            .unwrap_or_else(|| anyhow::anyhow!("stream error")),
                    );
                }
                _ => {}
            }
        }

        // Save assistant response.
        if !text_content.is_empty() {
            self.session.append(
                crate::session::session::assistant_message_entry(&text_content),
            );
        }

        // If no tool calls, we're done with this turn.
        if tool_calls.is_empty() {
            return LoopOutcome::Done(last_usage);
        }

        // Partition and dispatch tool calls.
        let batches = partition_tool_calls(&tool_calls, &*self.tools);
        for batch in batches {
            let aborted = self.run_batch(ctx, tx, &batch, turn, tr).await;
            if aborted {
                self.emit_event(tx, AgentEvent {
                    event_type: AgentEventType::Aborted,
                    ..Default::default()
                });
                return LoopOutcome::Aborted;
            }
        }

        LoopOutcome::Continue
    }

    /// Dispatches a single tool call, writes session entries, and emits events.
    pub async fn dispatch_tool(
        self: &Arc<Self>,
        ctx: &tokio_util::sync::CancellationToken,
        tx: &mpsc::Sender<AgentEvent>,
        tc: &ToolCall,
        turn: i32,
        tr: &Arc<Trace>,
    ) -> (ToolResult, bool /* aborted */) {
        // 1. Save tool call.
        let input_bytes = serde_json::to_vec(&tc.input).unwrap_or_default();
        self.session.append(
            crate::session::session::tool_call_entry(&tc.id, &tc.name, &input_bytes),
        );

        // 2. Permission gate.
        if let Some(perm) = &self.permission {
            let d = perm.check(&self.agent_id, &tc.name, &tc.input);
            if d.behavior == crate::tools::permission::DecisionBehavior::Deny {
                let result = self.append_denial_result(&tc.id, &d.reason);
                self.emit_tool_result_event(tx, tr, turn, tc, &result, false);
                return (result, false);
            }
        }

        // 3. Pre-execute cancel check.
        if ctx.is_cancelled() {
            let result = self.append_aborted_result(&tc.id);
            self.emit_tool_result_event(tx, tr, turn, tc, &result, true);
            return (result, true);
        }

        // 4. Execute.
        let result = match self.tools.execute(&tc.name, tc.input.clone()) {
            Ok(r) => r,
            Err(e) => ToolResult::err(e.to_string()),
        };

        // 5. Post-execute cancel check.
        if ctx.is_cancelled() {
            let result = self.append_aborted_result(&tc.id);
            self.emit_tool_result_event(tx, tr, turn, tc, &result, true);
            return (result, true);
        }

        // 6. Save result.
        let img_data: Vec<ImageData> = result
            .images
            .iter()
            .map(|img| ImageData {
                mime_type: img.mime_type.clone(),
                data: B64.encode(&img.data),
            })
            .collect();
        self.session.append(crate::session::session::tool_result_entry(
            &tc.id,
            &result.output,
            &result.error,
            img_data,
        ));

        // 7. Track touched files.
        if result.error.is_empty() && is_file_tool(&tc.name) {
            self.record_file_touch(&extract_path_from_input(&tc.input));
        }

        self.emit_tool_result_event(tx, tr, turn, tc, &result, false);
        (result, false)
    }

    fn append_denial_result(&self, tool_call_id: &str, reason: &str) -> ToolResult {
        self.session.append(crate::session::session::tool_result_entry(
            tool_call_id,
            "",
            reason,
            vec![],
        ));
        ToolResult::err(reason)
    }

    fn append_aborted_result(&self, tool_call_id: &str) -> ToolResult {
        self.session.append(
            crate::session::session::aborted_tool_result_entry(tool_call_id),
        );
        ToolResult::err("aborted by user")
    }

    fn emit_tool_result_event(
        &self,
        tx: &mpsc::Sender<AgentEvent>,
        tr: &Arc<Trace>,
        turn: i32,
        tc: &ToolCall,
        result: &ToolResult,
        aborted: bool,
    ) {
        tr.mark("tool.exec", &[
            ("turn".to_owned(), Value::from(turn)),
            ("tool".to_owned(), Value::from(tc.name.clone())),
            ("err".to_owned(), Value::from(!result.error.is_empty())),
            ("output_chars".to_owned(), Value::from(result.output.len())),
            ("aborted".to_owned(), Value::from(aborted)),
        ]);

        if !result.error.is_empty() {
            log::warn!("tool error: tool={} id={} error={}", tc.name, tc.id, result.error);
        } else {
            let preview = if result.output.len() > 500 {
                format!("{}...(truncated)", &result.output[..500])
            } else {
                result.output.clone()
            };
            log::debug!("tool result: tool={} id={} output_len={} output={}", tc.name, tc.id, result.output.len(), preview);
        }

        self.emit_event(tx, AgentEvent {
            event_type: AgentEventType::ToolResult,
            tool_call: Some(tc.clone()),
            result: Some(result.clone()),
            ..Default::default()
        });
    }

    /// Dispatches a batch of tool calls (parallel for safe batches, sequential
    /// for unsafe). Returns true if any call was aborted.
    async fn run_batch(
        self: &Arc<Self>,
        ctx: &tokio_util::sync::CancellationToken,
        tx: &mpsc::Sender<AgentEvent>,
        batch: &super::partition::Batch,
        turn: i32,
        tr: &Arc<Trace>,
    ) -> bool {
        if batch.calls.len() == 1 || !batch.concurrency_safe {
            // Sequential.
            for tc in &batch.calls {
                let (_, aborted) = self.dispatch_tool(ctx, tx, tc, turn, tr).await;
                if aborted {
                    return true;
                }
            }
            return false;
        }

        // Parallel.
        let cap = self.max_tool_concurrency();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(cap));
        let any_aborted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut handles = Vec::new();

        for tc in batch.calls.clone() {
            let runtime = Arc::clone(self);
            let tx = tx.clone();
            let sem = Arc::clone(&semaphore);
            let aborted_flag = Arc::clone(&any_aborted);
            let ctx = ctx.clone();
            let tr = Arc::clone(tr);

            let handle = tokio::spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore closed");
                let (_, aborted) = runtime.dispatch_tool(&ctx, &tx, &tc, turn, &tr).await;
                if aborted {
                    aborted_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            });
            handles.push(handle);
        }

        for h in handles {
            let _ = h.await;
        }

        any_aborted.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Collects all text from a Run and returns it.
    pub async fn run_sync(
        self: &Arc<Self>,
        ctx: tokio_util::sync::CancellationToken,
        user_msg: String,
        images: Vec<ImageContent>,
    ) -> anyhow::Result<String> {
        let mut rx = self.run(ctx, user_msg, images).await?;
        let mut response = String::new();
        while let Some(ev) = rx.recv().await {
            match ev.event_type {
                AgentEventType::TextDelta => response.push_str(&ev.text),
                AgentEventType::Error => {
                    return Err(ev.error.unwrap_or_else(|| anyhow::anyhow!("unknown error")));
                }
                _ => {}
            }
        }
        Ok(response)
    }
}

// ── Loop outcome ───────────────────────────────────────────────────────────────

enum LoopOutcome {
    Done(Option<crate::llm::Usage>),
    Continue,
    Aborted,
    Error(anyhow::Error),
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Returns true for tool names that take a `path` field.
pub fn is_file_tool(name: &str) -> bool {
    matches!(name, "read_file" | "write_file" | "edit_file")
}

/// Converts tool-result images to session ImageData.
pub fn convert_tool_result_images(imgs: &[ImageContent]) -> Vec<ImageData> {
    imgs.iter()
        .map(|img| ImageData {
            mime_type: img.mime_type.clone(),
            data: B64.encode(&img.data),
        })
        .collect()
}
