//! Package llmtest provides shared test helpers for LLMProvider stubs.
//!
//! Two pieces:
//!   - Base: an embeddable struct that supplies default no-op
//!     implementations of every LLMProvider method except chat_stream.
//!   - Stub: a fully-configurable LLMProvider for the common case
//!     (canned text response, optional delay, observable requests).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::llm::provider::{
    ChatEvent, ChatRequest, Diagnostic, EventType, LLMProvider, ModelInfo, ToolCall, ToolDef,
};

/// Base provides default implementations of every LLMProvider method
/// except chat_stream. Embed this in test stubs to avoid having to update
/// every stub when the interface widens.
pub struct Base;

impl Base {
    pub fn models(&self) -> Vec<ModelInfo> { vec![] }
    pub fn normalize_tool_schema(&self, tools: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        (tools, vec![])
    }
}

/// Stub is a configurable LLMProvider for tests.
pub struct Stub {
    /// Text is the canned text response emitted on every chat_stream call.
    pub text: String,
    /// Delay sleeps before emitting the response.
    pub delay: Option<Duration>,
    /// chat_err, if Some, is returned synchronously from chat_stream.
    pub chat_err: Option<anyhow::Error>,
    /// chat_hook, if Some, observes every chat_stream request.
    pub chat_hook: Option<Arc<dyn Fn(ChatRequest) + Send + Sync>>,
    /// requests collects all requests observed by chat_hook.
    pub requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl Default for Stub {
    fn default() -> Self {
        Stub {
            text: String::new(),
            delay: None,
            chat_err: None,
            chat_hook: None,
            requests: Arc::new(Mutex::new(vec![])),
        }
    }
}

#[async_trait::async_trait]
impl LLMProvider for Stub {
    async fn chat_stream(&self, req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        if let Some(hook) = &self.chat_hook {
            hook(req.clone());
        }
        {
            let mut reqs = self.requests.lock().unwrap();
            reqs.push(req.clone());
        }
        if let Some(err_msg) = &self.chat_err {
            return Err(anyhow::anyhow!("{}", err_msg));
        }
        let text = self.text.clone();
        let delay = self.delay;
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
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