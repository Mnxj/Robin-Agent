use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::llm::provider::{ChatEvent, ChatRequest, Diagnostic, EventType, LLMProvider, ModelInfo, ToolDef};
use crate::session::{assistant_message_entry, user_message_entry};

use super::Summarizer;

// ── fakeProvider ──────────────────────────────────────────────────────────────

/// fakeProvider is an LLMProvider stub that emits a fixed text response.
struct FakeProvider {
    text: String,
    err: Option<String>,
}

#[async_trait::async_trait]
impl LLMProvider for FakeProvider {
    async fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        if let Some(ref e) = self.err {
            return Err(anyhow::anyhow!("{}", e));
        }
        let text = self.text.clone();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = tx.send(ChatEvent {
                event_type: EventType::TextDelta,
                text,
                ..Default::default()
            }).await;
            let _ = tx.send(ChatEvent {
                event_type: EventType::Done,
                ..Default::default()
            }).await;
        });
        Ok(rx)
    }

    fn models(&self) -> Vec<ModelInfo> { vec![] }
    fn normalize_tool_schema(&self, tools: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        (tools, vec![])
    }
}

// ── flakyProvider ─────────────────────────────────────────────────────────────

/// flakyProvider returns ChatStream errors a configured number of times, then succeeds.
struct FlakyProvider {
    fails_remaining: std::sync::Mutex<i32>,
    success_text: String,
    failure_err: Option<String>,
}

#[async_trait::async_trait]
impl LLMProvider for FlakyProvider {
    async fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        let mut remaining = self.fails_remaining.lock().unwrap();
        if *remaining > 0 {
            *remaining -= 1;
            drop(remaining);
            let err_msg = self.failure_err.clone().unwrap_or_else(|| "input is too long".to_string());
            let (tx, rx) = mpsc::channel(2);
            tokio::spawn(async move {
                let _ = tx.send(ChatEvent {
                    event_type: EventType::Error,
                    error: Some(std::sync::Arc::new(anyhow::anyhow!("{}", err_msg))),
                    ..Default::default()
                }).await;
            });
            return Ok(rx);
        }
        drop(remaining);
        let text = self.success_text.clone();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = tx.send(ChatEvent {
                event_type: EventType::TextDelta,
                text,
                ..Default::default()
            }).await;
            let _ = tx.send(ChatEvent {
                event_type: EventType::Done,
                ..Default::default()
            }).await;
        });
        Ok(rx)
    }

    fn models(&self) -> Vec<ModelInfo> { vec![] }
    fn normalize_tool_schema(&self, tools: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        (tools, vec![])
    }
}

// ── delayedProvider ───────────────────────────────────────────────────────────

/// delayedProvider sleeps before responding.
pub struct DelayedProvider {
    pub text: String,
    pub delay: Duration,
}

#[async_trait::async_trait]
impl LLMProvider for DelayedProvider {
    async fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        let text = self.text.clone();
        let delay = self.delay;
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = tx.send(ChatEvent {
                event_type: EventType::TextDelta,
                text,
                ..Default::default()
            }).await;
            let _ = tx.send(ChatEvent {
                event_type: EventType::Done,
                ..Default::default()
            }).await;
        });
        Ok(rx)
    }

    fn models(&self) -> Vec<ModelInfo> { vec![] }
    fn normalize_tool_schema(&self, tools: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        (tools, vec![])
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_summarizer_returns_model_output() {
    let s = Summarizer {
        provider: Arc::new(FakeProvider { text: "we picked option B for X.".to_string(), err: None }),
        model: "qwen2.5:3b-instruct".to_string(),
        timeout: Duration::from_secs(5),
    };
    let entries = vec![user_message_entry("hello")];
    let got = s.summarize(&entries, "").await.unwrap();
    assert_eq!(got, "we picked option B for X.");
}

#[tokio::test]
async fn test_summarizer_trims_whitespace() {
    let s = Summarizer {
        provider: Arc::new(FakeProvider {
            text: "  \n  summary text  \n  ".to_string(),
            err: None,
        }),
        model: "qwen2.5:3b-instruct".to_string(),
        timeout: Duration::from_secs(5),
    };
    let entries = vec![user_message_entry("hi")];
    let got = s.summarize(&entries, "").await.unwrap();
    assert_eq!(got, "summary text");
}

#[tokio::test]
async fn test_summarizer_empty_response_falls_back_to_placeholder() {
    let s = Summarizer {
        provider: Arc::new(FakeProvider {
            text: "   \n  ".to_string(),
            err: None,
        }),
        model: "qwen2.5:3b-instruct".to_string(),
        timeout: Duration::from_secs(5),
    };
    let entries = vec![user_message_entry("hi")];
    let got = s.summarize(&entries, "").await.unwrap();
    assert!(got.contains("compaction failed"), "expected placeholder, got: {}", got);
}

#[tokio::test]
async fn test_summarizer_provider_error_falls_back_to_placeholder() {
    let s = Summarizer {
        provider: Arc::new(FakeProvider {
            text: String::new(),
            err: Some("ollama down".to_string()),
        }),
        model: "qwen2.5:3b-instruct".to_string(),
        timeout: Duration::from_secs(5),
    };
    let entries = vec![user_message_entry("hi")];
    let got = s.summarize(&entries, "").await.unwrap();
    assert!(got.contains("compaction failed"), "expected placeholder, got: {}", got);
}

#[tokio::test]
async fn test_summarize_fallback_full_stage_succeeds() {
    let s = Summarizer {
        provider: Arc::new(FakeProvider {
            text: "stage 1 summary".to_string(),
            err: None,
        }),
        model: "m".to_string(),
        timeout: Duration::from_secs(1),
    };
    let entries = vec![user_message_entry("hi")];
    let got = s.summarize(&entries, "").await.unwrap();
    assert!(got.contains("stage 1 summary"));
}

#[tokio::test]
async fn test_summarize_fallback_to_small_only_on_overflow() {
    let huge = "X".repeat(50_000);
    let entries = vec![
        user_message_entry("small 1"),
        assistant_message_entry(&huge),
        user_message_entry("small 2"),
    ];
    let s = Summarizer {
        provider: Arc::new(FlakyProvider {
            fails_remaining: std::sync::Mutex::new(1),
            success_text: "stage 2 summary".to_string(),
            failure_err: None, // defaults to "input is too long"
        }),
        model: "m".to_string(),
        timeout: Duration::from_secs(1),
    };
    let got = s.summarize(&entries, "").await.unwrap();
    assert!(
        got.contains("stage 2 summary"),
        "second-stage success must produce the summary; got: {}",
        got
    );
}

#[tokio::test]
async fn test_summarize_fallback_to_placeholder_when_all_stages_fail() {
    let entries = vec![user_message_entry("hi")];
    let s = Summarizer {
        provider: Arc::new(FlakyProvider {
            fails_remaining: std::sync::Mutex::new(99),
            success_text: String::new(),
            failure_err: None,
        }),
        model: "m".to_string(),
        timeout: Duration::from_secs(1),
    };
    let got = s.summarize(&entries, "").await.unwrap();
    assert!(
        got.contains("Conversation history"),
        "placeholder must be a usable summary stub; got: {}",
        got
    );
    assert!(
        got.contains("compaction failed"),
        "placeholder must indicate the failure; got: {}",
        got
    );
}

#[tokio::test]
async fn test_summarize_per_call_timeout_propagates() {
    // A per-call timeout (Summarizer.Timeout shorter than provider delay)
    // must surface to the caller as an error — NOT silently degrade to a placeholder.
    let s = Summarizer {
        provider: Arc::new(DelayedProvider {
            text: "never reached".to_string(),
            delay: Duration::from_millis(500),
        }),
        model: "m".to_string(),
        timeout: Duration::from_millis(50),
    };
    let entries = vec![user_message_entry("hi")];
    let result = s.summarize(&entries, "").await;
    assert!(
        result.is_err(),
        "per-call timeout must propagate, not be swallowed by placeholder"
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("deadline") || err.to_string().contains("timed out"),
        "error must indicate timeout; got: {}",
        err
    );
}