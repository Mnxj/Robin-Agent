use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::info;

use super::provider::{
    ChatEvent, ChatRequest, Diagnostic, EventType, LLMProvider, Message,
    ModelInfo, ReasoningMode, SystemPromptPart, ToolCall, ToolDef, Usage,
    join_system_prompt_parts,
};
use super::normalize::apply_strip_list;

pub struct OpenAIProvider {
    client: Arc<reqwest::Client>,
    api_key: String,
    base_url: String,
    pub(crate) kind: String,
}

impl OpenAIProvider {
    pub fn new(api_key: &str, base_url: &str, kind: &str, ca_bundle: &str) -> Self {
        let resolved_base_url = if base_url.is_empty() {
            match kind {
                "openai" => "https://api.openai.com".to_string(),
                _ => String::new(),
            }
        } else {
            base_url.trim_end_matches('/').to_string()
        };
        let mut builder = reqwest::ClientBuilder::new();
        if !ca_bundle.is_empty() {
            if let Ok(bytes) = std::fs::read(ca_bundle) {
                let cert = if bytes.starts_with(b"-----") {
                    reqwest::Certificate::from_pem(&bytes).ok()
                } else {
                    reqwest::Certificate::from_der(&bytes).ok()
                };
                if let Some(c) = cert {
                    builder = builder.add_root_certificate(c);
                }
            }
        }
        let client = builder.build().unwrap_or_else(|_| reqwest::Client::new());
        OpenAIProvider {
            client: Arc::new(client),
            api_key: api_key.to_string(),
            base_url: resolved_base_url,
            kind: kind.to_string(),
        }
    }

    pub fn build_reasoning_effort(&self, model: &str, mode: ReasoningMode) -> Option<String> {
        if mode == ReasoningMode::Off { return None; }        if self.kind == "openai-compatible" { return None; }
        if !openai_supports_reasoning(model) { return None; }
        match mode {
            ReasoningMode::Low => Some("low".to_string()),
            ReasoningMode::Medium => Some("medium".to_string()),
            ReasoningMode::High => Some("high".to_string()),
            ReasoningMode::Off => None,
        }
    }
}

fn openai_supports_reasoning(model: &str) -> bool {
    let prefixes = ["o1-", "o3-", "o4-", "gpt-5"];
    prefixes.iter().any(|p| model.starts_with(p))
}

pub(crate) fn concat_system_prompt_parts(parts: &[SystemPromptPart]) -> String {
    if parts.is_empty() { return String::new(); }
    let non_empty: Vec<&str> = parts.iter()
        .filter(|p| !p.text.is_empty())
        .map(|p| p.text.as_str())
        .collect();
    non_empty.join("\n")
}

// ── OpenAI request/response types ────────────────────────────────────────────

#[derive(Serialize, Debug)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAITool>,
    max_completion_tokens: i32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize, Debug, Clone)]
struct OpenAIMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    tool_calls: Vec<OpenAIToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
struct OpenAIToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: OpenAIFunctionCall,
}

#[derive(Serialize, Debug, Clone)]
struct OpenAIFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Serialize, Debug)]
struct OpenAITool {
    #[serde(rename = "type")]
    kind: String,
    function: OpenAIFunctionDef,
}

#[derive(Serialize, Debug)]
struct OpenAIFunctionDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

// ── SSE response types ────────────────────────────────────────────────────────

#[derive(Deserialize, Debug, Default)]
struct StreamChunk {
    choices: Option<Vec<StreamChoice>>,
    usage: Option<StreamUsage>,
}

#[derive(Deserialize, Debug, Default)]
struct StreamChoice {
    delta: Option<StreamDelta>,
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct StreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Deserialize, Debug, Default)]
struct StreamToolCallDelta {
    index: Option<usize>,
    id: Option<String>,
    function: Option<StreamFunctionDelta>,
}

#[derive(Deserialize, Debug, Default)]
struct StreamFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct StreamUsage {
    prompt_tokens: Option<i32>,
    completion_tokens: Option<i32>,
    total_tokens: Option<i32>,
}

// ── Request building ──────────────────────────────────────────────────────────

fn build_openai_messages(req: &ChatRequest) -> Vec<OpenAIMessage> {
    let mut msgs: Vec<OpenAIMessage> = Vec::new();

    let sys_prompt = if !req.system_prompt_parts.is_empty() {
        concat_system_prompt_parts(&req.system_prompt_parts)
    } else {
        req.system_prompt.clone()
    };
    if !sys_prompt.is_empty() {
        msgs.push(OpenAIMessage {
            role: "system".to_string(),
            content: Some(serde_json::Value::String(sys_prompt)),
            tool_calls: vec![],
            tool_call_id: None,
        });
    }

    for m in &req.messages {
        match m.role.as_str() {
            "user" => {
                if !m.tool_call_id.is_empty() {
                    let tool_text = if m.content.is_empty() && !m.images.is_empty() {
                        "(image attached in following message)".to_string()
                    } else {
                        m.content.clone()
                    };
                    msgs.push(OpenAIMessage {
                        role: "tool".to_string(),
                        content: Some(serde_json::Value::String(tool_text)),
                        tool_calls: vec![],
                        tool_call_id: Some(m.tool_call_id.clone()),
                    });
                    if !m.images.is_empty() {
                        let parts = build_image_parts(&m.images);
                        let mut all_parts = parts;
                        all_parts.push(serde_json::json!({
                            "type": "text",
                            "text": "(Image returned by the previous tool call.)"
                        }));
                        msgs.push(OpenAIMessage {
                            role: "user".to_string(),
                            content: Some(serde_json::Value::Array(all_parts)),
                            tool_calls: vec![],
                            tool_call_id: None,
                        });
                    }
                } else if !m.images.is_empty() {
                    let mut parts = build_image_parts(&m.images);
                    if !m.content.is_empty() {
                        parts.push(serde_json::json!({"type": "text", "text": m.content}));
                    }
                    msgs.push(OpenAIMessage {
                        role: "user".to_string(),
                        content: Some(serde_json::Value::Array(parts)),
                        tool_calls: vec![],
                        tool_call_id: None,
                    });
                } else {
                    msgs.push(OpenAIMessage {
                        role: "user".to_string(),
                        content: Some(serde_json::Value::String(m.content.clone())),
                        tool_calls: vec![],
                        tool_call_id: None,
                    });
                }
            }
            "assistant" => {
                let tool_calls: Vec<OpenAIToolCall> = m.tool_calls.iter().map(|tc| OpenAIToolCall {
                    id: tc.id.clone(),
                    kind: "function".to_string(),
                    function: OpenAIFunctionCall {
                        name: tc.name.clone(),
                        arguments: tc.input.to_string(),
                    },
                }).collect();
                msgs.push(OpenAIMessage {
                    role: "assistant".to_string(),
                    content: if m.content.is_empty() { None } else { Some(serde_json::Value::String(m.content.clone())) },
                    tool_calls,
                    tool_call_id: None,
                });
            }
            _ => {}
        }
    }
    msgs
}

fn build_image_parts(images: &[super::provider::ImageContent]) -> Vec<serde_json::Value> {
    images.iter().map(|img| {
        let encoded = BASE64.encode(&img.data);
        let data_uri = format!("data:{};base64,{}", img.mime_type, encoded);
        serde_json::json!({
            "type": "image_url",
            "image_url": {"url": data_uri, "detail": "auto"}
        })
    }).collect()
}

// ── SSE streaming ─────────────────────────────────────────────────────────────

async fn process_openai_stream(
    response: reqwest::Response,
    tx: mpsc::Sender<ChatEvent>,
) {
    use futures_util::StreamExt;

    let mut stream = response.bytes_stream();
    let mut buf = String::new();
    let mut tool_calls: std::collections::HashMap<usize, PendingTC> = Default::default();
    let mut last_usage: Option<Usage> = None;

    'outer: while let Some(chunk) = stream.next().await {
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

        loop {
            // Find end of SSE event (double newline)
            let idx = if let Some(i) = buf.find("\n\n") {
                i + 2
            } else if let Some(i) = buf.find("\r\n\r\n") {
                i + 4
            } else {
                break;
            };

            let line = buf[..idx].to_string();
            buf = buf[idx..].to_string();

            for l in line.lines() {
                let data = if let Some(d) = l.strip_prefix("data: ") { d } else { continue };
                if data == "[DONE]" { break 'outer; }

                let chunk: StreamChunk = match serde_json::from_str(data) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                if let Some(usage) = chunk.usage {
                    if usage.total_tokens.unwrap_or(0) > 0 {
                        last_usage = Some(Usage {
                            input_tokens: usage.prompt_tokens.unwrap_or(0),
                            output_tokens: usage.completion_tokens.unwrap_or(0),
                            ..Default::default()
                        });
                    }
                }

                for choice in chunk.choices.unwrap_or_default() {
                    if let Some(delta) = choice.delta {
                        if let Some(content) = delta.content {
                            if !content.is_empty() {
                                let _ = tx.send(ChatEvent {
                                    event_type: EventType::TextDelta,
                                    text: content,
                                    ..Default::default()
                                }).await;
                            }
                        }
                        for tc_delta in delta.tool_calls.unwrap_or_default() {
                            let idx = tc_delta.index.unwrap_or(0);
                            let pending = tool_calls.entry(idx).or_insert_with(|| PendingTC::default());
                            if let Some(id) = tc_delta.id {
                                pending.id = id;
                            }
                            if let Some(func) = tc_delta.function {
                                if let Some(name) = func.name {
                                    pending.name = name.clone();
                                    let _ = tx.send(ChatEvent {
                                        event_type: EventType::ToolCallStart,
                                        tool_call: Some(ToolCall {
                                            id: pending.id.clone(),
                                            name,
                                            input: serde_json::Value::Null,
                                        }),
                                        ..Default::default()
                                    }).await;
                                }
                                if let Some(args) = func.arguments {
                                    pending.args_json.push_str(&args);
                                }
                            }
                        }
                    }

                    let finish = choice.finish_reason.as_deref();
                    if finish == Some("tool_calls") || finish == Some("stop") {
                        for tc in tool_calls.values() {
                            if !tc.name.is_empty() {
                                let input_val: serde_json::Value = serde_json::from_str(&tc.args_json)
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
                    }
                }
            }
        }
    }

    let _ = tx.send(ChatEvent {
        event_type: EventType::Done,
        usage: last_usage,
        ..Default::default()
    }).await;
}

#[derive(Default)]
struct PendingTC {
    id: String,
    name: String,
    args_json: String,
}

// ── openaiUnsupportedFields ───────────────────────────────────────────────────

const OPENAI_UNSUPPORTED_FIELDS: &[&str] = &["$ref", "definitions"];

// ── LLMProvider implementation ────────────────────────────────────────────────

#[async_trait::async_trait]
impl LLMProvider for OpenAIProvider {
    async fn chat_stream(&self, req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        let msgs = build_openai_messages(&req);
        let tools: Vec<OpenAITool> = req.tools.iter().map(|t| {
            OpenAITool {
                kind: "function".to_string(),
                function: OpenAIFunctionDef {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            }
        }).collect();

        let model = if req.model.is_empty() { "gpt-4o".to_string() } else { req.model.clone() };
        let max_tokens = if req.max_tokens == 0 { 4096 } else { req.max_tokens };

        let reasoning_effort = self.build_reasoning_effort(&model, req.reasoning);
        if req.reasoning != ReasoningMode::Off && reasoning_effort.is_none() {
            let reason = if self.kind == "openai-compatible" {
                "endpoint may not support reasoning_effort"
            } else {
                "model does not support reasoning_effort"
            };
            info!("reasoning ignored provider=openai kind={} model={} reason={}", self.kind, model, reason);
        }

        let body = OpenAIRequest {
            model: model.clone(),
            messages: msgs,
            tools,
            max_completion_tokens: max_tokens,
            stream: true,
            stream_options: Some(StreamOptions { include_usage: true }),
            temperature: if req.temperature > 0.0 { Some(req.temperature as f32) } else { None },
            reasoning_effort,
        };

        let url = format!("{}/v1/chat/completions", self.base_url);
        let response = self.client.post(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("openai error {}: {}", status, text));
        }

        let (tx, rx) = mpsc::channel(100);
        tokio::spawn(async move {
            process_openai_stream(response, tx).await;
        });

        Ok(rx)
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo { id: "gpt-4o".to_string(), name: "GPT-4o".to_string(), provider: "openai".to_string() },
            ModelInfo { id: "gpt-4o-mini".to_string(), name: "GPT-4o Mini".to_string(), provider: "openai".to_string() },
            ModelInfo { id: "gpt-4-turbo".to_string(), name: "GPT-4 Turbo".to_string(), provider: "openai".to_string() },
        ]
    }

    fn normalize_tool_schema(&self, tools: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        apply_strip_list(tools, OPENAI_UNSUPPORTED_FIELDS)
    }
}