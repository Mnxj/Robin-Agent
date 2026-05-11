use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::info;

use super::provider::{
    ChatEvent, ChatRequest, Diagnostic, EventType, LLMProvider, Message,
    ModelInfo, ReasoningMode, SystemPromptPart, ToolCall, ToolDef, Usage,
    join_system_prompt_parts,
};
use super::normalize::apply_strip_list;

pub struct GeminiProvider {
    client: Arc<reqwest::Client>,
    api_key: String,
    base_url: String,
}

impl GeminiProvider {
    pub fn new(api_key: &str, base_url: &str) -> Self {
        GeminiProvider {
            client: Arc::new(reqwest::ClientBuilder::new().build().unwrap_or_else(|_| reqwest::Client::new())),
            api_key: api_key.to_string(),
            base_url: if base_url.is_empty() {
                "https://generativelanguage.googleapis.com".to_string()
            } else {
                base_url.trim_end_matches('/').to_string()
            },
        }
    }

    pub fn build_thinking_budget(&self, model: &str, mode: ReasoningMode) -> Option<i32> {
        if mode == ReasoningMode::Off { return None; }
        if !gemini_supports_thinking(model) { return None; }
        match mode {
            ReasoningMode::Low => Some(1024),
            ReasoningMode::Medium => Some(4096),
            ReasoningMode::High => Some(16384),
            ReasoningMode::Off => None,
        }
    }
}

fn gemini_supports_thinking(model: &str) -> bool {
    let prefixes = ["gemini-2.0-flash-thinking", "gemini-2.5"];
    prefixes.iter().any(|p| model.starts_with(p))
}

pub(crate) fn gemini_resolve_system_prompt(req: &ChatRequest) -> String {
    if !req.system_prompt_parts.is_empty() {
        let non_empty: Vec<&str> = req.system_prompt_parts.iter()
            .filter(|p| !p.text.is_empty())
            .map(|p| p.text.as_str())
            .collect();
        return non_empty.join("\n");
    }
    req.system_prompt.clone()
}

// ── Gemini request/response types ─────────────────────────────────────────────

#[derive(Serialize, Debug)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiTool>>,
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(rename = "inlineData", skip_serializing_if = "Option::is_none")]
    inline_data: Option<GeminiInlineData>,
    #[serde(rename = "functionCall", skip_serializing_if = "Option::is_none")]
    function_call: Option<GeminiFunctionCall>,
    #[serde(rename = "functionResponse", skip_serializing_if = "Option::is_none")]
    function_response: Option<GeminiFunctionResponse>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiInlineData {
    data: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiFunctionCall {
    id: Option<String>,
    name: String,
    args: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiFunctionResponse {
    name: String,
    id: Option<String>,
    response: serde_json::Value,
}

#[derive(Serialize, Debug)]
struct GeminiTool {
    #[serde(rename = "functionDeclarations")]
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Serialize, Debug)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    #[serde(rename = "parametersJsonSchema")]
    parameters_json_schema: serde_json::Value,
}

#[derive(Serialize, Debug)]
struct GeminiGenerationConfig {
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(rename = "thinkingConfig", skip_serializing_if = "Option::is_none")]
    thinking_config: Option<GeminiThinkingConfig>,
}

#[derive(Serialize, Debug)]
struct GeminiThinkingConfig {
    #[serde(rename = "thinkingBudget")]
    thinking_budget: i32,
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Deserialize, Debug)]
struct GeminiCandidate {
    content: Option<GeminiContent>,
}

#[derive(Deserialize, Debug)]
struct GeminiUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<i32>,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<i32>,
}

// ── Request building ──────────────────────────────────────────────────────────

fn build_gemini_contents(in_msgs: &[Message]) -> (Vec<GeminiContent>, std::collections::HashMap<String, String>) {
    let mut tool_id_to_name: std::collections::HashMap<String, String> = Default::default();
    for m in in_msgs {
        for tc in &m.tool_calls {
            tool_id_to_name.insert(tc.id.clone(), tc.name.clone());
        }
    }

    let mut contents: Vec<GeminiContent> = Vec::new();
    for m in in_msgs {
        match m.role.as_str() {
            "user" => {
                if !m.tool_call_id.is_empty() {
                    let func_name = tool_id_to_name.get(&m.tool_call_id)
                        .cloned()
                        .unwrap_or_else(|| m.tool_call_id.clone());
                    let response: serde_json::Value = if m.is_error {
                        serde_json::json!({"error": m.content})
                    } else {
                        serde_json::json!({"output": m.content})
                    };
                    contents.push(GeminiContent {
                        role: "user".to_string(),
                        parts: vec![GeminiPart {
                            text: None,
                            inline_data: None,
                            function_call: None,
                            function_response: Some(GeminiFunctionResponse {
                                name: func_name,
                                id: Some(m.tool_call_id.clone()),
                                response,
                            }),
                        }],
                    });
                } else {
                    let mut parts: Vec<GeminiPart> = Vec::new();
                    for img in &m.images {
                        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
                        parts.push(GeminiPart {
                            text: None,
                            inline_data: Some(GeminiInlineData {
                                data: BASE64.encode(&img.data),
                                mime_type: img.mime_type.clone(),
                            }),
                            function_call: None,
                            function_response: None,
                        });
                    }
                    if !m.content.is_empty() {
                        parts.push(GeminiPart {
                            text: Some(m.content.clone()),
                            inline_data: None,
                            function_call: None,
                            function_response: None,
                        });
                    }
                    contents.push(GeminiContent { role: "user".to_string(), parts });
                }
            }
            "assistant" => {
                let mut parts: Vec<GeminiPart> = Vec::new();
                if !m.content.is_empty() {
                    parts.push(GeminiPart {
                        text: Some(m.content.clone()),
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                    });
                }
                for tc in &m.tool_calls {
                    let args: serde_json::Value = tc.input.clone();
                    parts.push(GeminiPart {
                        text: None,
                        inline_data: None,
                        function_call: Some(GeminiFunctionCall {
                            id: Some(tc.id.clone()),
                            name: tc.name.clone(),
                            args,
                        }),
                        function_response: None,
                    });
                }
                if !parts.is_empty() {
                    contents.push(GeminiContent { role: "model".to_string(), parts });
                }
            }
            _ => {}
        }
    }

    (contents, tool_id_to_name)
}

// ── Streaming ─────────────────────────────────────────────────────────────────

const GEMINI_UNSUPPORTED_FIELDS: &[&str] = &["anyOf", "oneOf", "not", "$ref", "format"];

#[async_trait::async_trait]
impl LLMProvider for GeminiProvider {
    async fn chat_stream(&self, req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        let (contents, _) = build_gemini_contents(&req.messages);

        let model = if req.model.is_empty() { "gemini-2.5-flash".to_string() } else { req.model.clone() };

        let tools: Option<Vec<GeminiTool>> = if req.tools.is_empty() {
            None
        } else {
            let decls: Vec<GeminiFunctionDeclaration> = req.tools.iter().map(|t| {
                GeminiFunctionDeclaration {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters_json_schema: t.parameters.clone(),
                }
            }).collect();
            Some(vec![GeminiTool { function_declarations: decls }])
        };

        let sys_prompt = gemini_resolve_system_prompt(&req);
        let system_instruction = if sys_prompt.is_empty() {
            None
        } else {
            Some(GeminiContent {
                role: "user".to_string(),
                parts: vec![GeminiPart {
                    text: Some(sys_prompt),
                    inline_data: None,
                    function_call: None,
                    function_response: None,
                }],
            })
        };

        let thinking_budget = self.build_thinking_budget(&model, req.reasoning);
        if req.reasoning != ReasoningMode::Off && thinking_budget.is_none() {
            info!("reasoning ignored provider=gemini model={} reason=model does not support thinking", model);
        }

        let gen_config = GeminiGenerationConfig {
            max_output_tokens: if req.max_tokens > 0 { Some(req.max_tokens) } else { None },
            temperature: if req.temperature > 0.0 { Some(req.temperature as f32) } else { None },
            thinking_config: thinking_budget.map(|b| GeminiThinkingConfig { thinking_budget: b }),
        };

        let body = GeminiRequest {
            contents,
            tools,
            generation_config: Some(gen_config),
            system_instruction,
        };

        let url = format!(
            "{}/v1beta/models/{}:streamGenerateContent?key={}&alt=sse",
            self.base_url, model, self.api_key
        );

        let response = self.client.post(&url)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("gemini error {}: {}", status, text));
        }

        let (tx, rx) = mpsc::channel(100);
        tokio::spawn(async move {
            process_gemini_stream(response, tx).await;
        });

        Ok(rx)
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo { id: "gemini-2.5-flash".to_string(), name: "Gemini 2.5 Flash".to_string(), provider: "gemini".to_string() },
            ModelInfo { id: "gemini-2.5-pro".to_string(), name: "Gemini 2.5 Pro".to_string(), provider: "gemini".to_string() },
            ModelInfo { id: "gemini-2.0-flash".to_string(), name: "Gemini 2.0 Flash".to_string(), provider: "gemini".to_string() },
        ]
    }

    fn normalize_tool_schema(&self, tools: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        apply_strip_list(tools, GEMINI_UNSUPPORTED_FIELDS)
    }
}

async fn process_gemini_stream(response: reqwest::Response, tx: mpsc::Sender<ChatEvent>) {
    use futures_util::StreamExt;

    let mut stream = response.bytes_stream();
    let mut buf = String::new();
    let mut sent_done = false;

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

        loop {
            let idx = if let Some(i) = buf.find("\n\n") { i + 2 }
                else if let Some(i) = buf.find("\r\n\r\n") { i + 4 }
                else { break };

            let event_str = buf[..idx].to_string();
            buf = buf[idx..].to_string();

            for line in event_str.lines() {
                let data = if let Some(d) = line.strip_prefix("data: ") { d } else { continue };
                if data == "[DONE]" { break; }

                let resp: GeminiResponse = match serde_json::from_str(data) {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                if let Some(meta) = &resp.usage_metadata {
                    let _ = tx.send(ChatEvent {
                        event_type: EventType::Done,
                        usage: Some(Usage {
                            input_tokens: meta.prompt_token_count.unwrap_or(0),
                            output_tokens: meta.candidates_token_count.unwrap_or(0),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }).await;
                    sent_done = true;
                }

                for cand in resp.candidates.unwrap_or_default() {
                    if let Some(content) = cand.content {
                        for part in content.parts {
                            if let Some(text) = part.text {
                                let _ = tx.send(ChatEvent {
                                    event_type: EventType::TextDelta,
                                    text,
                                    ..Default::default()
                                }).await;
                            }
                            if let Some(fc) = part.function_call {
                                let args_json = serde_json::to_value(&fc.args)
                                    .unwrap_or(serde_json::json!({}));
                                let id = fc.id.unwrap_or_else(|| fc.name.clone());
                                let _ = tx.send(ChatEvent {
                                    event_type: EventType::ToolCallStart,
                                    tool_call: Some(ToolCall { id: id.clone(), name: fc.name.clone(), input: serde_json::Value::Null }),
                                    ..Default::default()
                                }).await;
                                let _ = tx.send(ChatEvent {
                                    event_type: EventType::ToolCallDone,
                                    tool_call: Some(ToolCall { id, name: fc.name, input: args_json }),
                                    ..Default::default()
                                }).await;
                            }
                        }
                    }
                }
            }
        }
    }

    if !sent_done {
        let _ = tx.send(ChatEvent {
            event_type: EventType::Done,
            ..Default::default()
        }).await;
    }
}