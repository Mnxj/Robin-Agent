use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

// ── Event types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    TextDelta,
    ToolCallStart,
    ToolCallDelta,
    ToolCallDone,
    Done,
    Error,
}

impl Default for EventType {
    fn default() -> Self { EventType::Done }
}

// ── Core message types ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ImageContent {
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,
    #[serde(skip)]
    pub images: Vec<ImageContent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_error: bool,
}

fn is_false(v: &bool) -> bool { !v }

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Clone, Debug, Default)]
pub struct Diagnostic {
    pub tool_name: String,
    pub field: String,
    pub action: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct SystemPromptPart {
    pub text: String,
    pub cache: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Copy)]
pub enum ReasoningMode {
    #[default]
    Off,
    Low,
    Medium,
    High,
}

impl ReasoningMode {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        parse_reasoning_mode(s)
    }
}

impl std::fmt::Display for ReasoningMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReasoningMode::Off => write!(f, "off"),
            ReasoningMode::Low => write!(f, "low"),
            ReasoningMode::Medium => write!(f, "medium"),
            ReasoningMode::High => write!(f, "high"),
        }
    }
}

pub fn parse_reasoning_mode(s: &str) -> anyhow::Result<ReasoningMode> {
    match s {
        "" | "off" => Ok(ReasoningMode::Off),
        "low" => Ok(ReasoningMode::Low),
        "medium" => Ok(ReasoningMode::Medium),
        "high" => Ok(ReasoningMode::High),
        other => anyhow::bail!("unknown reasoning mode {:?} (want off|low|medium|high)", other),
    }
}

#[derive(Clone, Debug, Default)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub max_tokens: i32,
    pub temperature: f64,
    pub system_prompt: String,
    pub system_prompt_parts: Vec<SystemPromptPart>,
    pub cache_last_message: bool,
    pub reasoning: ReasoningMode,
}

#[derive(Clone, Debug, Default)]
pub struct Usage {
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cache_creation_input_tokens: i32,
    pub cache_read_input_tokens: i32,
}

#[derive(Debug, Default, Clone)]
pub struct ChatEvent {
    pub event_type: EventType,
    pub text: String,
    pub tool_call: Option<ToolCall>,
    pub usage: Option<Usage>,
    pub error: Option<std::sync::Arc<anyhow::Error>>,
}

#[derive(Clone, Debug, Default)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
}

// ── Provider interface ─────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait LLMProvider: Send + Sync {
    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> anyhow::Result<mpsc::Receiver<ChatEvent>>;

    fn models(&self) -> Vec<ModelInfo>;

    fn normalize_tool_schema(
        &self,
        tools: Vec<ToolDef>,
    ) -> (Vec<ToolDef>, Vec<Diagnostic>);
}

#[async_trait::async_trait]
pub trait NonStreamingProvider: Send + Sync {
    async fn chat_non_streaming(
        &self,
        req: ChatRequest,
    ) -> anyhow::Result<mpsc::Receiver<ChatEvent>>;
}

// ── ProviderOptions ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct ProviderOptions {
    pub api_key: String,
    pub base_url: String,
    pub kind: String,
    pub ca_bundle: String,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub fn parse_provider_model(name: &str) -> (&str, &str) {
    if let Some(i) = name.find('/') {
        (&name[..i], &name[i + 1..])
    } else {
        ("", name)
    }
}

pub fn join_system_prompt_parts(parts: &[SystemPromptPart]) -> String {
    let non_empty: Vec<&str> = parts.iter()
        .filter(|p| !p.text.is_empty())
        .map(|p| p.text.as_str())
        .collect();
    non_empty.join("\n")
}

pub fn new_provider(
    provider_name: &str,
    opts: ProviderOptions,
) -> anyhow::Result<Box<dyn LLMProvider>> {
    let kind = if opts.kind.is_empty() {
        if !opts.base_url.is_empty() {
            "openai-compatible".to_string()
        } else {
            provider_name.to_string()
        }
    } else {
        opts.kind.clone()
    };

    match kind.as_str() {
        "anthropic" => Ok(Box::new(super::anthropic::AnthropicProvider::new(&opts.api_key, &opts.base_url))),
        "openai" => Ok(Box::new(super::openai::OpenAIProvider::new(&opts.api_key, &opts.base_url, "openai", &opts.ca_bundle))),
        "openai-compatible" => Ok(Box::new(super::openai::OpenAIProvider::new(&opts.api_key, &opts.base_url, "openai-compatible", &opts.ca_bundle))),
        "gemini" => Ok(Box::new(super::gemini::GeminiProvider::new(&opts.api_key, &opts.base_url))),
        "qwen" => Ok(Box::new(super::qwen::QwenProvider::new(&opts.api_key, &opts.base_url))),
        other => anyhow::bail!("unknown LLM provider kind: {:?}", other),
    }
}