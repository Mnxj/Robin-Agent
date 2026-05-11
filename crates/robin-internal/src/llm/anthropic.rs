use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bytes::Bytes;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{info, error};

use super::provider::{
    ChatEvent, ChatRequest, Diagnostic, EventType, ImageContent, Message,
    ModelInfo, SystemPromptPart, ToolCall, ToolDef, Usage,
    LLMProvider, NonStreamingProvider, ReasoningMode,
    join_system_prompt_parts,
};

pub struct AnthropicProvider {
    client: Arc<reqwest::Client>,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: &str, base_url: &str) -> Self {
        AnthropicProvider {
            client: Arc::new(reqwest::ClientBuilder::new().build().unwrap_or_else(|_| reqwest::Client::new())),
            api_key: api_key.to_string(),
            base_url: if base_url.is_empty() {
                "https://api.anthropic.com".to_string()
            } else {
                base_url.trim_end_matches('/').to_string()
            },
        }
    }

    pub fn build_thinking_config(&self, model: &str, mode: ReasoningMode) -> Option<AnthropicThinkingConfig> {
        if mode == ReasoningMode::Off {
            return None;
        }
        if !anthropic_supports_thinking(model) {
            return None;
        }
        match mode {
            ReasoningMode::Low => Some(AnthropicThinkingConfig { budget_tokens: 1024 }),
            ReasoningMode::Medium => Some(AnthropicThinkingConfig { budget_tokens: 4096 }),
            ReasoningMode::High => Some(AnthropicThinkingConfig { budget_tokens: 16384 }),
            ReasoningMode::Off => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnthropicThinkingConfig {
    pub budget_tokens: i64,
}

fn anthropic_supports_thinking(model: &str) -> bool {
    let no_think = [
        "claude-3-haiku", "claude-3-5-haiku",
        "claude-3-sonnet", "claude-3-5-sonnet",
        "claude-3-opus",
    ];
    for prefix in &no_think {
        if model.starts_with(prefix) {
            return false;
        }
    }
    true
}

// ── Anthropic request/response types ─────────────────────────────────────────

#[derive(Serialize)]
pub(crate) struct AnthropicRequest {
    model: String,
    max_tokens: i64,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    system: Vec<AnthropicTextBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinkingParam>,
    stream: bool,
}

#[derive(Serialize, Clone)]
pub(crate) struct AnthropicMessage {
    pub(crate) role: String,
    pub(crate) content: Vec<AnthropicContent>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type")]
pub(crate) enum AnthropicContent {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "image")]
    Image {
        source: AnthropicImageSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<Vec<AnthropicToolResultContent>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
}

#[derive(Serialize, Clone)]
struct AnthropicImageSource {
    #[serde(rename = "type")]
    kind: String,
    media_type: String,
    data: String,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type")]
enum AnthropicToolResultContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: AnthropicImageSource },
}

#[derive(Serialize, Clone)]
pub(crate) struct CacheControl {
    #[serde(rename = "type")]
    pub(crate) kind: String,
}

impl CacheControl {
    fn ephemeral() -> Self { CacheControl { kind: "ephemeral".to_string() } }
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Serialize, Clone)]
pub(crate) struct AnthropicTextBlock {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_control: Option<CacheControl>,
}

#[derive(Serialize)]
struct AnthropicThinkingParam {
    #[serde(rename = "type")]
    kind: String,
    budget_tokens: i64,
}

// ── SSE response types ────────────────────────────────────────────────────────

#[derive(Deserialize, Debug, Default)]
struct SseMessageStart {
    message: Option<SseMessageStartInner>,
}

#[derive(Deserialize, Debug, Default)]
struct SseMessageStartInner {
    usage: Option<SseUsage>,
}

#[derive(Deserialize, Debug, Default)]
struct SseUsage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
}

#[derive(Deserialize, Debug, Default)]
struct SseContentBlockStart {
    content_block: Option<SseContentBlock>,
}

#[derive(Deserialize, Debug, Default)]
struct SseContentBlock {
    #[serde(rename = "type")]
    kind: Option<String>,
    id: Option<String>,
    name: Option<String>,
    input: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug, Default)]
struct SseContentBlockDelta {
    delta: Option<SseDelta>,
}

#[derive(Deserialize, Debug, Default)]
struct SseDelta {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
    partial_json: Option<String>,
    stop_reason: Option<String>,
    usage: Option<SseUsage>,
}

#[derive(Deserialize, Debug, Default)]
struct SseMessageDelta {
    usage: Option<SseUsage>,
}

// ── Request building ──────────────────────────────────────────────────────────

pub(crate) fn build_anthropic_messages_pub(in_msgs: &[Message], cache_last: bool) -> Vec<AnthropicMessage> {
    build_anthropic_messages(in_msgs, cache_last)
}

pub(crate) fn build_anthropic_system_pub(req: &ChatRequest) -> Vec<AnthropicTextBlock> {
    build_anthropic_system(req)
}

fn build_anthropic_messages(in_msgs: &[Message], cache_last: bool) -> Vec<AnthropicMessage> {
    let mut msgs: Vec<AnthropicMessage> = Vec::new();
    let mut i = 0;
    while i < in_msgs.len() {
        let m = &in_msgs[i];
        match m.role.as_str() {
            "user" => {
                if !m.tool_call_id.is_empty() {
                    // Collect a run of consecutive tool_result user messages.
                    let mut blocks: Vec<AnthropicContent> = Vec::new();
                    while i < in_msgs.len() {
                        let cur = &in_msgs[i];
                        if cur.role != "user" || cur.tool_call_id.is_empty() {
                            // un-consume; outer loop will re-process
                            break;
                        }
                        blocks.push(build_tool_result_block(cur));
                        i += 1;
                    }
                    msgs.push(AnthropicMessage { role: "user".to_string(), content: blocks });
                    continue;
                } else if !m.images.is_empty() {
                    let mut blocks: Vec<AnthropicContent> = Vec::new();
                    for img in &m.images {
                        let encoded = BASE64.encode(&img.data);
                        blocks.push(AnthropicContent::Image {
                            source: AnthropicImageSource {
                                kind: "base64".to_string(),
                                media_type: img.mime_type.clone(),
                                data: encoded,
                            },
                            cache_control: None,
                        });
                    }
                    if !m.content.is_empty() {
                        blocks.push(AnthropicContent::Text { text: m.content.clone(), cache_control: None });
                    }
                    msgs.push(AnthropicMessage { role: "user".to_string(), content: blocks });
                } else {
                    msgs.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: vec![AnthropicContent::Text { text: m.content.clone(), cache_control: None }],
                    });
                }
            }
            "assistant" => {
                let mut blocks: Vec<AnthropicContent> = Vec::new();
                if !m.content.is_empty() {
                    blocks.push(AnthropicContent::Text { text: m.content.clone(), cache_control: None });
                }
                for tc in &m.tool_calls {
                    let input = tc.input.clone();
                    blocks.push(AnthropicContent::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input,
                        cache_control: None,
                    });
                }
                if !blocks.is_empty() {
                    msgs.push(AnthropicMessage { role: "assistant".to_string(), content: blocks });
                }
            }
            _ => {}
        }
        i += 1;
    }

    // Cache marker on the last block of the last message.
    if cache_last && !msgs.is_empty() {
        if let Some(tail) = msgs.last_mut() {
            if let Some(last_block) = tail.content.last_mut() {
                set_cache_control(last_block);
            }
        }
    }

    msgs
}

fn set_cache_control(block: &mut AnthropicContent) {
    let cc = Some(CacheControl::ephemeral());
    match block {
        AnthropicContent::Text { cache_control, .. } => *cache_control = cc,
        AnthropicContent::Image { cache_control, .. } => *cache_control = cc,
        AnthropicContent::ToolUse { cache_control, .. } => *cache_control = cc,
        AnthropicContent::ToolResult { cache_control, .. } => *cache_control = cc,
    }
}

fn build_tool_result_block(m: &Message) -> AnthropicContent {
    if !m.images.is_empty() {
        let mut content: Vec<AnthropicToolResultContent> = Vec::new();
        for img in &m.images {
            let encoded = BASE64.encode(&img.data);
            content.push(AnthropicToolResultContent::Image {
                source: AnthropicImageSource {
                    kind: "base64".to_string(),
                    media_type: img.mime_type.clone(),
                    data: encoded,
                },
            });
        }
        if !m.content.is_empty() {
            content.push(AnthropicToolResultContent::Text { text: m.content.clone() });
        }
        AnthropicContent::ToolResult {
            tool_use_id: m.tool_call_id.clone(),
            content: Some(content),
            is_error: if m.is_error { Some(true) } else { None },
            cache_control: None,
        }
    } else {
        AnthropicContent::ToolResult {
            tool_use_id: m.tool_call_id.clone(),
            content: if m.content.is_empty() { None } else {
                Some(vec![AnthropicToolResultContent::Text { text: m.content.clone() }])
            },
            is_error: if m.is_error { Some(true) } else { None },
            cache_control: None,
        }
    }
}

fn build_anthropic_system(req: &ChatRequest) -> Vec<AnthropicTextBlock> {
    if !req.system_prompt_parts.is_empty() {
        let blocks: Vec<AnthropicTextBlock> = req.system_prompt_parts.iter()
            .filter(|p| !p.text.is_empty())
            .map(|p| AnthropicTextBlock {
                kind: "text".to_string(),
                text: p.text.clone(),
                cache_control: if p.cache { Some(CacheControl::ephemeral()) } else { None },
            })
            .collect();
        if !blocks.is_empty() {
            return blocks;
        }
        return vec![];
    }
    if !req.system_prompt.is_empty() {
        return vec![AnthropicTextBlock {
            kind: "text".to_string(),
            text: req.system_prompt.clone(),
            cache_control: None,
        }];
    }
    vec![]
}

fn build_request(req: &ChatRequest) -> AnthropicRequest {
    let msgs = build_anthropic_messages(&req.messages, req.cache_last_message);

    let tools: Vec<AnthropicTool> = req.tools.iter().map(|t| {
        AnthropicTool {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.parameters.clone(),
        }
    }).collect();

    let mut max_tokens = req.max_tokens as i64;
    if max_tokens == 0 { max_tokens = 4096; }
    let model = if req.model.is_empty() { "claude-sonnet-4-5-20250514".to_string() } else { req.model.clone() };

    let system = build_anthropic_system(req);

    // placeholder for thinking param (would be filled in below)
    let thinking_cfg = build_thinking_config_static(&model, req.reasoning);
    if let Some(ref cfg) = thinking_cfg {
        let required = cfg.budget_tokens + 4096;
        if max_tokens < required { max_tokens = required; }
    }

    AnthropicRequest {
        model,
        max_tokens,
        messages: msgs,
        tools,
        system,
        temperature: if req.temperature > 0.0 { Some(req.temperature) } else { None },
        thinking: thinking_cfg.map(|cfg| AnthropicThinkingParam {
            kind: "enabled".to_string(),
            budget_tokens: cfg.budget_tokens,
        }),
        stream: true,
    }
}

fn build_thinking_config_static(model: &str, mode: ReasoningMode) -> Option<AnthropicThinkingConfig> {
    if mode == ReasoningMode::Off { return None; }
    if !anthropic_supports_thinking(model) { return None; }
    match mode {
        ReasoningMode::Low => Some(AnthropicThinkingConfig { budget_tokens: 1024 }),
        ReasoningMode::Medium => Some(AnthropicThinkingConfig { budget_tokens: 4096 }),
        ReasoningMode::High => Some(AnthropicThinkingConfig { budget_tokens: 16384 }),
        ReasoningMode::Off => None,
    }
}

// ── SSE parsing ───────────────────────────────────────────────────────────────

async fn process_stream(
    response: reqwest::Response,
    tx: mpsc::Sender<ChatEvent>,
) {
    let mut stream = response.bytes_stream();
    let mut buf = String::new();

    let mut current_event_type = String::new();
    let mut pending_tools: Vec<PendingTC> = Vec::new();
    let mut current_block_type = String::new();
    let mut input_tokens: i64 = 0;
    let mut cache_creation_tokens: i64 = 0;
    let mut cache_read_tokens: i64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(ChatEvent {
                    event_type: EventType::Error,
                    error: Some(std::sync::Arc::new(anyhow::anyhow!("{}", e))),
                    ..Default::default()
                }).await;
                return;
            }
        };
        buf.push_str(&String::from_utf8_lossy(&chunk));

        // Process complete SSE events (separated by double newlines)
        loop {
            if let Some(idx) = find_double_newline(&buf) {
                let event_str = buf[..idx].to_string();
                buf = buf[idx..].trim_start_matches('\n').to_string();

                let (ev_type, ev_data) = parse_sse_event(&event_str);
                if !ev_type.is_empty() {
                    current_event_type = ev_type;
                }

                if ev_data.is_empty() || ev_data == "[DONE]" {
                    current_event_type.clear();
                    continue;
                }

                match current_event_type.as_str() {
                    "message_start" => {
                        if let Ok(e) = serde_json::from_str::<SseMessageStart>(&ev_data) {
                            if let Some(msg) = e.message {
                                if let Some(u) = msg.usage {
                                    input_tokens = u.input_tokens.unwrap_or(0);
                                    cache_creation_tokens = u.cache_creation_input_tokens.unwrap_or(0);
                                    cache_read_tokens = u.cache_read_input_tokens.unwrap_or(0);
                                }
                            }
                        }
                    }
                    "content_block_start" => {
                        if let Ok(e) = serde_json::from_str::<SseContentBlockStart>(&ev_data) {
                            if let Some(cb) = e.content_block {
                                match cb.kind.as_deref() {
                                    Some("text") => current_block_type = "text".to_string(),
                                    Some("tool_use") => {
                                        current_block_type = "tool_use".to_string();
                                        let mut start_input = String::new();
                                        if let Some(inp) = &cb.input {
                                            if let Ok(s) = serde_json::to_string(inp) {
                                                if s != "null" && s != "{}" {
                                                    start_input = s;
                                                }
                                            }
                                        }
                                        let tc_id = cb.id.unwrap_or_default();
                                        let tc_name = cb.name.unwrap_or_default();
                                        pending_tools.push(PendingTC {
                                            id: tc_id.clone(),
                                            name: tc_name.clone(),
                                            input_json: String::new(),
                                            start_input,
                                        });
                                        let _ = tx.send(ChatEvent {
                                            event_type: EventType::ToolCallStart,
                                            tool_call: Some(ToolCall {
                                                id: tc_id,
                                                name: tc_name,
                                                input: serde_json::Value::Null,
                                            }),
                                            ..Default::default()
                                        }).await;
                                    }
                                    _ => current_block_type.clear(),
                                }
                            }
                        }
                    }
                    "content_block_delta" => {
                        if let Ok(e) = serde_json::from_str::<SseContentBlockDelta>(&ev_data) {
                            if let Some(delta) = e.delta {
                                match delta.kind.as_deref() {
                                    Some("text_delta") => {
                                        if let Some(text) = delta.text {
                                            let _ = tx.send(ChatEvent {
                                                event_type: EventType::TextDelta,
                                                text,
                                                ..Default::default()
                                            }).await;
                                        }
                                    }
                                    Some("input_json_delta") => {
                                        if let Some(partial) = delta.partial_json {
                                            if let Some(pending) = pending_tools.last_mut() {
                                                pending.input_json.push_str(&partial);
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    "content_block_stop" => {
                        if current_block_type == "tool_use" {
                            if let Some(tc) = pending_tools.last() {
                                let inp = if !tc.input_json.is_empty() {
                                    tc.input_json.clone()
                                } else if !tc.start_input.is_empty() {
                                    tc.start_input.clone()
                                } else {
                                    "{}".to_string()
                                };
                                let input_val: serde_json::Value = serde_json::from_str(&inp)
                                    .unwrap_or(serde_json::json!({}));
                                let _ = tx.send(ChatEvent {
                                    event_type: EventType::ToolCallDone,
                                    tool_call: Some(ToolCall {
                                        id: tc.id.clone(),
                                        name: tc.name.clone(),
                                        input: input_val,
                                    }),
                                    ..Default::default()
                                }).await;
                            }
                        }
                        current_block_type.clear();
                    }
                    "message_delta" => {
                        if let Ok(e) = serde_json::from_str::<SseMessageDelta>(&ev_data) {
                            let output_tokens = e.usage.as_ref().and_then(|u| u.output_tokens).unwrap_or(0);
                            if output_tokens > 0 || cache_creation_tokens > 0 || cache_read_tokens > 0 {
                                let _ = tx.send(ChatEvent {
                                    event_type: EventType::Done,
                                    usage: Some(Usage {
                                        input_tokens: input_tokens as i32,
                                        output_tokens: output_tokens as i32,
                                        cache_creation_input_tokens: cache_creation_tokens as i32,
                                        cache_read_input_tokens: cache_read_tokens as i32,
                                    }),
                                    ..Default::default()
                                }).await;
                            }
                        }
                    }
                    "message_stop" => {}
                    "error" => {
                        error!("anthropic stream error event");
                    }
                    _ => {}
                }
                current_event_type.clear();
            } else {
                break;
            }
        }
    }
}

struct PendingTC {
    id: String,
    name: String,
    input_json: String,
    start_input: String,
}

fn find_double_newline(s: &str) -> Option<usize> {
    // Find \n\n or \r\n\r\n
    if let Some(idx) = s.find("\n\n") {
        return Some(idx + 2);
    }
    if let Some(idx) = s.find("\r\n\r\n") {
        return Some(idx + 4);
    }
    None
}

fn parse_sse_event(event: &str) -> (String, String) {
    let mut event_type = String::new();
    let mut data = String::new();
    for line in event.lines() {
        if let Some(t) = line.strip_prefix("event: ") {
            event_type = t.to_string();
        } else if let Some(d) = line.strip_prefix("data: ") {
            data = d.to_string();
        }
    }
    (event_type, data)
}

// ── LLMProvider implementation ────────────────────────────────────────────────

#[async_trait::async_trait]
impl LLMProvider for AnthropicProvider {
    async fn chat_stream(&self, req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        let url = format!("{}/v1/messages", self.base_url);
        let body = build_request(&req);

        let model = body.model.clone();
        let reasoning = req.reasoning;

        let response = self.client.post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header("anthropic-beta", "prompt-caching-2024-07-31")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("anthropic error {}: {}", status, text));
        }

        if reasoning != ReasoningMode::Off {
            if anthropic_supports_thinking(&model) {
                info!("anthropic thinking enabled model={}", model);
            } else {
                info!("reasoning ignored provider=anthropic model={} reason=model does not support thinking", model);
            }
        }

        let (tx, rx) = mpsc::channel(100);
        tokio::spawn(async move {
            process_stream(response, tx).await;
        });

        Ok(rx)
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo { id: "claude-sonnet-4-5-20250514".to_string(), name: "Claude Sonnet 4.5".to_string(), provider: "anthropic".to_string() },
            ModelInfo { id: "claude-opus-4-0-20250514".to_string(), name: "Claude Opus 4".to_string(), provider: "anthropic".to_string() },
            ModelInfo { id: "claude-haiku-3-5-20241022".to_string(), name: "Claude Haiku 3.5".to_string(), provider: "anthropic".to_string() },
        ]
    }

    fn normalize_tool_schema(&self, tools: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        (tools, vec![])
    }
}

#[async_trait::async_trait]
impl NonStreamingProvider for AnthropicProvider {
    async fn chat_non_streaming(&self, req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        let url = format!("{}/v1/messages", self.base_url);
        let mut body = build_request(&req);
        body.stream = false;

        let response = self.client.post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header("anthropic-beta", "prompt-caching-2024-07-31")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("anthropic error {}: {}", status, text));
        }

        let msg: serde_json::Value = response.json().await?;

        let (tx, rx) = mpsc::channel(16);
        tokio::spawn(async move {
            if let Some(content) = msg["content"].as_array() {
                for block in content {
                    match block["type"].as_str() {
                        Some("text") => {
                            if let Some(text) = block["text"].as_str() {
                                if !text.is_empty() {
                                    let _ = tx.send(ChatEvent {
                                        event_type: EventType::TextDelta,
                                        text: text.to_string(),
                                        ..Default::default()
                                    }).await;
                                }
                            }
                        }
                        Some("tool_use") => {
                            let id = block["id"].as_str().unwrap_or("").to_string();
                            let name = block["name"].as_str().unwrap_or("").to_string();
                            let _ = tx.send(ChatEvent {
                                event_type: EventType::ToolCallStart,
                                tool_call: Some(ToolCall { id: id.clone(), name: name.clone(), input: serde_json::Value::Null }),
                                ..Default::default()
                            }).await;
                            let input_json = serde_json::to_string(&block["input"])
                                .unwrap_or_else(|_| "{}".to_string());
                            let input_val: serde_json::Value = serde_json::from_str(&input_json)
                                .unwrap_or(serde_json::json!({}));
                            let _ = tx.send(ChatEvent {
                                event_type: EventType::ToolCallDone,
                                tool_call: Some(ToolCall { id, name, input: input_val }),
                                ..Default::default()
                            }).await;
                        }
                        _ => {}
                    }
                }
            }
            let usage = Usage {
                input_tokens: msg["usage"]["input_tokens"].as_i64().unwrap_or(0) as i32,
                output_tokens: msg["usage"]["output_tokens"].as_i64().unwrap_or(0) as i32,
                cache_creation_input_tokens: msg["usage"]["cache_creation_input_tokens"].as_i64().unwrap_or(0) as i32,
                cache_read_input_tokens: msg["usage"]["cache_read_input_tokens"].as_i64().unwrap_or(0) as i32,
            };
            let _ = tx.send(ChatEvent {
                event_type: EventType::Done,
                usage: Some(usage),
                ..Default::default()
            }).await;
        });

        Ok(rx)
    }
}