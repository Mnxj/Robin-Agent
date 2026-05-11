use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use tokio::sync::mpsc;

use crate::llm::provider::{ChatRequest, EventType, LLMProvider, Message, SystemPromptPart};
use crate::session::SessionEntry;

use super::overflow::is_context_overflow;
use super::prompt::{build_prompt_parts, build_transcript, format_compact_summary};

/// ErrEmptySummary is returned when the LLM emits no usable summary text.
#[derive(Debug)]
pub struct ErrEmptySummary;

impl std::fmt::Display for ErrEmptySummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "compaction: empty summary returned")
    }
}

impl std::error::Error for ErrEmptySummary {}

/// Summarizer wraps an LLMProvider with the prompt and call shape used for
/// compaction.
pub struct Summarizer {
    pub provider: Arc<dyn LLMProvider>,
    pub model: String,
    /// per-call deadline; zero → 60 s
    pub timeout: Duration,
}

impl Summarizer {
    /// Summarize sends entries through the configured provider and returns the
    /// trimmed, formatted summary text.
    ///
    /// The call wraps three fallback stages:
    ///   1. Full transcript — preferred; preserves all detail.
    ///   2. Small-only — drops oversized messages. Triggered when stage 1
    ///      returns a context-overflow or stream error.
    ///   3. Placeholder — a static stub indicating compaction failed.
    pub async fn summarize(
        &self,
        entries: &[SessionEntry],
        additional_instructions: &str,
    ) -> anyhow::Result<String> {
        self.summarize_with_fallback(entries, additional_instructions).await
    }

    async fn summarize_with_fallback(
        &self,
        entries: &[SessionEntry],
        additional_instructions: &str,
    ) -> anyhow::Result<String> {
        // Stage 1: full transcript.
        let transcript = build_transcript(entries);
        match self.call_once(&transcript, additional_instructions).await {
            Ok(out) if !out.is_empty() => return Ok(out),
            Err(e) => {
                // Context cancellation/deadline must propagate — don't degrade to
                // a placeholder when the caller asked us to stop.
                let e_str = e.to_string();
                let is_deadline = e_str.contains("deadline exceeded")
                    || e_str.contains("DeadlineExceeded")
                    || e_str.contains("timed out")
                    || e_str.contains("operation canceled")
                    || e_str.contains("context canceled");
                if is_deadline {
                    return Err(e);
                }

                // Stage 2: drop oversized messages and retry. Only meaningful when
                // build_small_only_transcript actually elides something.
                if is_overflow_error(&e) || is_stream_error(&e) {
                    let (small, dropped) = build_small_only_transcript(entries);
                    if dropped > 0 {
                        let mut s = small;
                        s.push_str(&format!("\n[oversized message(s) elided: {}]\n", dropped));
                        if let Ok(out2) = self.call_once(&s, additional_instructions).await {
                            if !out2.is_empty() {
                                return Ok(out2);
                            }
                        }
                    }
                }
                // Fall through to Stage 3.
            }
            Ok(_) => {
                // Stage 1 returned empty string — fall through to Stage 3.
            }
        }

        // Stage 3: placeholder. Never returns an error.
        Ok(placeholder_summary(entries.len()))
    }

    /// callOnce performs a single summarizer invocation against a pre-built
    /// transcript. Returns the formatted summary text or an error.
    async fn call_once(
        &self,
        transcript: &str,
        additional_instructions: &str,
    ) -> anyhow::Result<String> {
        let (system_prompt, user_message) =
            build_prompt_parts(transcript, additional_instructions);

        let timeout = if self.timeout.is_zero() {
            Duration::from_secs(60)
        } else {
            self.timeout
        };

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: user_message,
                ..Default::default()
            }],
            system_prompt_parts: vec![SystemPromptPart {
                text: system_prompt,
                cache: true,
            }],
            max_tokens: 4096,
            ..Default::default()
        };

        // Run with per-call timeout — wraps both the ChatStream call AND the full
        // event-collection loop so providers that return Ok(rx) immediately but
        // then delay events (as DelayedProvider does in tests) are still bounded.
        let provider = Arc::clone(&self.provider);
        let collect_result = tokio::time::timeout(timeout, async move {
            let rx = provider.chat_stream(req).await
                .map_err(|e| anyhow!("compaction: chat stream: {}", e))?;

            let mut sb = String::new();
            let mut stream = rx;
            while let Some(ev) = stream.recv().await {
                match ev.event_type {
                    EventType::TextDelta => sb.push_str(&ev.text),
                    EventType::Error => {
                        if let Some(e) = ev.error {
                            return Err(anyhow!("compaction: stream error: {}", e));
                        }
                        return Err(anyhow!("compaction: stream error: unknown"));
                    }
                    _ => {}
                }
            }
            Ok::<String, anyhow::Error>(sb)
        })
        .await;

        let collected = match collect_result {
            Err(_elapsed) => {
                return Err(anyhow!("compaction: chat stream: deadline exceeded"));
            }
            Ok(Err(e)) => return Err(e),
            Ok(Ok(s)) => s,
        };

        let out = collected.trim().to_string();
        if out.is_empty() {
            return Err(anyhow!(ErrEmptySummary));
        }
        let out = format_compact_summary(&out);
        if out.is_empty() {
            return Err(anyhow!(ErrEmptySummary));
        }
        Ok(out)
    }
}

/// maxSmallEntryLen is the per-entry size threshold for stage 2.
const MAX_SMALL_ENTRY_LEN: usize = 10_000; // matches MAX_TRANSCRIPT_TOOL_RESULT_LEN

/// buildSmallOnlyTranscript renders entries while skipping any single-entry
/// payload larger than MAX_SMALL_ENTRY_LEN.
fn build_small_only_transcript(entries: &[SessionEntry]) -> (String, usize) {
    let mut dropped = 0usize;
    let kept: Vec<SessionEntry> = entries
        .iter()
        .filter(|e| {
            let size = e.data.get().len();
            if size > MAX_SMALL_ENTRY_LEN {
                dropped += 1;
                false
            } else {
                true
            }
        })
        .cloned()
        .collect();
    (build_transcript(&kept), dropped)
}

/// placeholder_summary is the stage-3 fallback. It must be a valid summary
/// the model can pick up from. The stable phrase "compaction failed and the
/// summary could not be generated" is detected by the circuit breaker.
pub fn placeholder_summary(entry_count: usize) -> String {
    format!(
        "Summary:\nConversation history ({} entries) — compaction failed and the summary could not be generated. \
The conversation continues; refer to the recent preserved turns and ask the user for any context you need.",
        entry_count
    )
}

/// isOverflowError reports whether err looks like a "your prompt is too big" signal.
fn is_overflow_error(err: &anyhow::Error) -> bool {
    is_context_overflow(err)
}

/// isStreamError reports whether err originated from the streaming layer.
fn is_stream_error(err: &anyhow::Error) -> bool {
    err.to_string().contains("stream error")
}

#[cfg(test)]
#[path = "summarizer_test.rs"]
mod summarizer_test;