use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::info;

use super::provider::{
    ChatEvent, ChatRequest, Diagnostic, EventType, LLMProvider, Message,
    ModelInfo, ReasoningMode, SystemPromptPart, ToolCall, ToolDef, Usage,
};
use super::normalize::apply_strip_list;
use super::openai::concat_system_prompt_parts;

pub struct QwenProvider {
    client: Arc<reqwest::Client>,
    api_key: String,
    base_url: String,
}

impl QwenProvider {
    pub fn new(api_key: &str, base_url: &str) -> Self {
        let url = if base_url.is_empty() {
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1".to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        };
        QwenProvider {
            client: Arc::new(reqwest::ClientBuilder::new().build().unwrap_or_else(|_| reqwest::Client::new())),
            api_key: api_key.to_string(),
            base_url: url,
        }
    }

    pub fn build_enable_thinking(&self, model: &str, mode: ReasoningMode) -> Option<(bool, super::provider::Diagnostic)> {
        if mode == ReasoningMode::Off { return None; }
        if !qwen_supports_thinking(model) { return None; }
        Some((true, super::provider::Diagnostic {
            tool_name: String::new(),
            field: String::new(),
            action: "clamped".to_string(),
            reason: "qwen reasoning is boolean; granularity ignored".to_string(),
        }))
    }
}

fn qwen_supports_thinking(model: &str) -> bool {
    let prefixes = ["qwen-qwq", "qwen3"];
    prefixes.iter().any(|p| model.starts_with(p))
}

pub(crate) fn qwen_resolve_system_prompt(req: &ChatRequest) -> String {
    if !req.system_prompt_parts.is_empty() {
        return concat_system_prompt_parts(&req.system_prompt_parts);
    }
    req.system_prompt.clone()
}

// ── Request/response types (reuse OpenAI-compatible shapes) ──────────────────

#[derive(Serialize, Debug)]
struct QwenRequest {
    model: String,
    messages: Vec<QwenMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<QwenTool>,
    max_tokens: i32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize, Debug, Clone)]
struct QwenMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    tool_calls: Vec<QwenToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
struct QwenToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: QwenFunctionCall,
}

#[derive(Serialize, Debug, Clone)]
struct QwenFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Serialize, Debug)]
struct QwenTool {
    #[serde(rename = "type")]
    kind: String,
    function: QwenFunctionDef,
}

#[derive(Serialize, Debug)]
struct QwenFunctionDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

// ── SSE streaming (same format as OpenAI) ────────────────────────────────────

#[derive(Deserialize, Debug, Default)]
struct StreamChunk {
    choices: Option<Vec<StreamChoice>>,
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

const QWEN_UNSUPPORTED_FIELDS: &[&str] = &["$ref", "definitions"];

fn build_qwen_messages(req: &ChatRequest) -> Vec<QwenMessage> {
    let mut msgs: Vec<QwenMessage> = Vec::new();

    let sys = qwen_resolve_system_prompt(req);
    if !sys.is_empty() {
        msgs.push(QwenMessage {
            role: "system".to_string(),
            content: Some(serde_json::Value::String(sys)),
            tool_calls: vec![],
            tool_call_id: None,
        });
    }

    for m in &req.messages {
        match m.role.as_str() {
            "user" => {
                if !m.tool_call_id.is_empty() {
                    if !m.images.is_empty() {
                        let parts: Vec<serde_json::Value> = m.images.iter().map(|img| {
                            let encoded = BASE64.encode(&img.data);
                            serde_json::json!({
                                "type": "image_url",
                                "image_url": {"url": format!("data:{};base64,{}", img.mime_type, encoded), "detail": "auto"}
                            })
                        }).chain(if !m.content.is_empty() {
                            vec![serde_json::json!({"type": "text", "text": m.content})]
                        } else {
                            vec![]
                        }).collect();
                        msgs.push(QwenMessage {
                            role: "tool".to_string(),
                            content: Some(serde_json::Value::Array(parts)),
                            tool_calls: vec![],
                            tool_call_id: Some(m.tool_call_id.clone()),
                        });
                    } else {
                        msgs.push(QwenMessage {
                            role: "tool".to_string(),
                            content: Some(serde_json::Value::String(m.content.clone())),
                            tool_calls: vec![],
                            tool_call_id: Some(m.tool_call_id.clone()),
                        });
                    }
                } else if !m.images.is_empty() {
                    let mut parts: Vec<serde_json::Value> = m.images.iter().map(|img| {
                        let encoded = BASE64.encode(&img.data);
                        serde_json::json!({
                            "type": "image_url",
                            "image_url": {"url": format!("data:{};base64,{}", img.mime_type, encoded), "detail": "auto"}
                        })
                    }).collect();
                    if !m.content.is_empty() {
                        parts.push(serde_json::json!({"type": "text", "text": m.content}));
                    }
                    msgs.push(QwenMessage {
                        role: "user".to_string(),
                        content: Some(serde_json::Value::Array(parts)),
                        tool_calls: vec![],
                        tool_call_id: None,
                    });
                } else {
                    msgs.push(QwenMessage {
                        role: "user".to_string(),
                        content: Some(serde_json::Value::String(m.content.clone())),
                        tool_calls: vec![],
                        tool_call_id: None,
                    });
                }
            }
            "assistant" => {
                let tool_calls: Vec<QwenToolCall> = m.tool_calls.iter().map(|tc| QwenToolCall {
                    id: tc.id.clone(),
                    kind: "function".to_string(),
                    function: QwenFunctionCall {
                        name: tc.name.clone(),
                        arguments: tc.input.to_string(),
                    },
                }).collect();
                msgs.push(QwenMessage {
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

async fn process_qwen_stream(response: reqwest::Response, tx: mpsc::Sender<ChatEvent>) {
    use futures_util::StreamExt;

    let mut stream = response.bytes_stream();
    let mut buf = String::new();
    let mut tool_calls: std::collections::HashMap<usize, PendingTC> = Default::default();

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
            let idx = if let Some(i) = buf.find("\n\n") { i + 2 }
                else if let Some(i) = buf.find("\r\n\r\n") { i + 4 }
                else { break };

            let line = buf[..idx].to_string();
            buf = buf[idx..].to_string();

            for l in line.lines() {
                let data = if let Some(d) = l.strip_prefix("data: ") { d } else { continue };
                if data == "[DONE]" { break 'outer; }

                let chunk: StreamChunk = match serde_json::from_str(data) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

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
                            let pending = tool_calls.entry(idx).or_insert_with(PendingTC::default);
                            if let Some(id) = tc_delta.id { pending.id = id; }
                            if let Some(func) = tc_delta.function {
                                if let Some(name) = func.name {
                                    pending.name = name.clone();
                                    let _ = tx.send(ChatEvent {
                                        event_type: EventType::ToolCallStart,
                                        tool_call: Some(ToolCall { id: pending.id.clone(), name, input: serde_json::Value::Null }),
                                        ..Default::default()
                                    }).await;
                                }
                                if let Some(args) = func.arguments { pending.args_json.push_str(&args); }
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
                                    tool_call: Some(ToolCall { id: tc.id.clone(), name: tc.name.clone(), input: input_val }),
                                    ..Default::default()
                                }).await;
                            }
                        }
                    }
                }
            }
        }
    }

    let _ = tx.send(ChatEvent { event_type: EventType::Done, ..Default::default() }).await;
}

#[derive(Default)]
struct PendingTC { id: String, name: String, args_json: String }

#[async_trait::async_trait]
impl LLMProvider for QwenProvider {
    async fn chat_stream(&self, req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        let msgs = build_qwen_messages(&req);
        let tools: Vec<QwenTool> = req.tools.iter().map(|t| {
            QwenTool {
                kind: "function".to_string(),
                function: QwenFunctionDef { name: t.name.clone(), description: t.description.clone(), parameters: t.parameters.clone() },
            }
        }).collect();

        let model = if req.model.is_empty() { "qwen-plus".to_string() } else { req.model.clone() };
        let max_tokens = if req.max_tokens == 0 { 4096 } else { req.max_tokens };

        if req.reasoning != ReasoningMode::Off {
            if let Some((_enabled, diag)) = self.build_enable_thinking(&model, req.reasoning) {
                info!("qwen thinking requested model={} reason={}", model, diag.reason);
            } else {
                info!("reasoning ignored provider=qwen model={} reason=model does not support thinking", model);
            }
        }

        let body = QwenRequest {
            model,
            messages: msgs,
            tools,
            max_tokens,
            stream: true,
            temperature: if req.temperature > 0.0 { Some(req.temperature as f32) } else { None },
        };

        let url = format!("{}/chat/completions", self.base_url);
        let response = self.client.post(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("qwen error {}: {}", status, text));
        }

        let (tx, rx) = mpsc::channel(100);
        tokio::spawn(async move { process_qwen_stream(response, tx).await; });
        Ok(rx)
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo { id: "qwen-plus".to_string(), name: "Qwen Plus".to_string(), provider: "qwen".to_string() },
            ModelInfo { id: "qwen-turbo".to_string(), name: "Qwen Turbo".to_string(), provider: "qwen".to_string() },
            ModelInfo { id: "qwen-max".to_string(), name: "Qwen Max".to_string(), provider: "qwen".to_string() },
            ModelInfo { id: "qwen-coder-plus".to_string(), name: "Qwen Coder Plus".to_string(), provider: "qwen".to_string() },
            ModelInfo { id: "qwen-vl-plus".to_string(), name: "Qwen VL Plus".to_string(), provider: "qwen".to_string() },
        ]
    }

    fn normalize_tool_schema(&self, tools: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        apply_strip_list(tools, QWEN_UNSUPPORTED_FIELDS)
    }
}