use std::time::Duration;

use serde_json::Value;

use super::tool::{Tool, ToolResult};

const TELEGRAM_DEFAULT_BASE_URL: &str = "https://api.telegram.org";
const TELEGRAM_TIMEOUT: Duration = Duration::from_secs(15);
const TELEGRAM_MAX_TEXT: usize = 4096;
const CHANNEL_TELEGRAM: &str = "telegram";

/// Configuration subset needed to drive the send_message tool.
#[derive(Debug, Clone, Default)]
pub struct SendMessageRegistration {
    pub telegram_enabled: bool,
    pub telegram_bot_token: String,
    pub telegram_default_chat_id: String,
}

/// Sends outbound messages to messaging channels. Currently supports Telegram.
pub struct SendMessageTool {
    /// Live config provider — when `Some`, takes precedence over static fields.
    pub config_fn: Option<Box<dyn Fn() -> SendMessageRegistration + Send + Sync>>,

    /// Static fallback used when `config_fn` is `None`.
    pub telegram_token: String,
    pub telegram_default_chat_id: String,
    /// Injected for tests; empty → use TELEGRAM_DEFAULT_BASE_URL.
    pub telegram_base_url: String,
    pub http_client: Option<reqwest::blocking::Client>,
}

impl Default for SendMessageTool {
    fn default() -> Self {
        Self {
            config_fn: None,
            telegram_token: String::new(),
            telegram_default_chat_id: String::new(),
            telegram_base_url: String::new(),
            http_client: None,
        }
    }
}

impl SendMessageTool {
    fn telegram_config(&self) -> (String, String) {
        if let Some(f) = &self.config_fn {
            let c = f();
            if !c.telegram_enabled {
                return (String::new(), String::new());
            }
            return (c.telegram_bot_token, c.telegram_default_chat_id);
        }
        (self.telegram_token.clone(), self.telegram_default_chat_id.clone())
    }

    fn send_telegram(&self, input: &SendMessageInput) -> anyhow::Result<ToolResult> {
        let (token, default_chat_id) = self.telegram_config();
        if token.trim().is_empty() {
            return Ok(ToolResult::err(
                "telegram channel is not configured — enable it and add a bot_token in Settings → Messaging",
            ));
        }
        if input.text.trim().is_empty() {
            return Ok(ToolResult::err("text is required"));
        }
        if input.text.len() > TELEGRAM_MAX_TEXT {
            return Ok(ToolResult::err(format!(
                "text exceeds Telegram's 4096-character limit (got {})",
                input.text.len()
            )));
        }
        let chat_id = if !input.chat_id.trim().is_empty() {
            input.chat_id.trim().to_owned()
        } else {
            default_chat_id.trim().to_owned()
        };
        if chat_id.is_empty() {
            return Ok(ToolResult::err("chat_id is required (no default_chat_id configured)"));
        }
        if !input.parse_mode.is_empty() {
            match input.parse_mode.as_str() {
                "Markdown" | "MarkdownV2" | "HTML" => {}
                _ => {
                    return Ok(ToolResult::err(format!(
                        "invalid parse_mode {:?} (valid: Markdown, MarkdownV2, HTML)",
                        input.parse_mode
                    )))
                }
            }
        }

        let mut payload = serde_json::json!({
            "chat_id": chat_id,
            "text": input.text
        });
        if !input.parse_mode.is_empty() {
            payload["parse_mode"] = Value::String(input.parse_mode.clone());
        }
        if input.disable_preview {
            payload["disable_web_page_preview"] = Value::Bool(true);
        }
        if input.disable_notify {
            payload["disable_notification"] = Value::Bool(true);
        }

        let base = if self.telegram_base_url.is_empty() {
            TELEGRAM_DEFAULT_BASE_URL
        } else {
            self.telegram_base_url.trim_end_matches('/')
        };
        let url = format!("{}/bot{}/sendMessage", base, token);

        let client = self.http_client.clone().unwrap_or_else(|| {
            reqwest::blocking::Client::builder()
                .timeout(TELEGRAM_TIMEOUT)
                .build()
                .expect("build http client")
        });

        let resp = match client.post(&url).json(&payload).send() {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::err(format!("send to Telegram: {}", e))),
        };

        let status = resp.status();
        let resp_body = match resp.text() {
            Ok(b) => b,
            Err(e) => return Ok(ToolResult::err(format!("read response: {}", e))),
        };

        #[derive(serde::Deserialize)]
        struct ApiResp {
            ok: bool,
            #[serde(default)]
            description: String,
            #[serde(default)]
            error_code: i64,
            result: Option<serde_json::Value>,
        }

        let api_resp: ApiResp = match serde_json::from_str(&resp_body) {
            Ok(r) => r,
            Err(_) => {
                return Ok(ToolResult::err(format!(
                    "Telegram returned non-JSON (HTTP {}): {}",
                    status.as_u16(),
                    truncate(&resp_body, 200)
                )))
            }
        };

        if !api_resp.ok {
            return Ok(ToolResult::err(format!(
                "Telegram API error {}: {}",
                api_resp.error_code, api_resp.description
            )));
        }

        let message_id = api_resp
            .result
            .as_ref()
            .and_then(|r| r.get("message_id"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let mut meta = serde_json::Map::new();
        meta.insert("channel".to_owned(), Value::String(CHANNEL_TELEGRAM.to_owned()));
        meta.insert("chat_id".to_owned(), Value::String(chat_id.clone()));
        meta.insert("message_id".to_owned(), Value::Number(message_id.into()));

        Ok(ToolResult {
            output: format!("Sent telegram → {} (message_id={})", chat_id, message_id),
            metadata: Some(meta),
            ..Default::default()
        })
    }
}

#[derive(Debug, Default)]
struct SendMessageInput {
    channel: String,
    text: String,
    chat_id: String,
    parse_mode: String,
    disable_preview: bool,
    disable_notify: bool,
}

impl SendMessageInput {
    fn from_value(v: &Value) -> Self {
        Self {
            channel: v.get("channel").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            text: v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            chat_id: v.get("chat_id").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            parse_mode: v.get("parse_mode").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            disable_preview: v.get("disable_link_preview").and_then(|x| x.as_bool()).unwrap_or(false),
            disable_notify: v.get("disable_notification").and_then(|x| x.as_bool()).unwrap_or(false),
        }
    }
}

impl Tool for SendMessageTool {
    fn name(&self) -> &str { "send_message" }

    fn description(&self) -> &str {
        r#"Send a message to a messaging channel. This is an outbound action — it actually delivers a message to a real person/channel and cannot be undone. Use sparingly and only when the user has asked for a message to be sent.

Currently supported channels:
- "telegram" (default): Telegram Bot API. "chat_id" can be a numeric user ID, "@channelname", or a negative group ID. Optional "parse_mode" enables Markdown/HTML formatting.

Required: "text" (max 4096 characters for Telegram). "chat_id" is optional if a default is configured for the channel; otherwise pass an explicit recipient."#
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "channel": {
                    "type": "string",
                    "enum": ["telegram"],
                    "description": "Messaging channel to send through. Default: telegram."
                },
                "text": {
                    "type": "string",
                    "description": "Message body (max 4096 characters for Telegram)."
                },
                "chat_id": {
                    "type": "string",
                    "description": "Recipient ID."
                },
                "parse_mode": {
                    "type": "string",
                    "enum": ["", "Markdown", "MarkdownV2", "HTML"],
                    "description": "Optional formatting mode (Telegram). Default is plain text."
                },
                "disable_link_preview": {
                    "type": "boolean",
                    "description": "Suppress link previews (Telegram). Default false."
                },
                "disable_notification": {
                    "type": "boolean",
                    "description": "Send silently, no notification sound (Telegram). Default false."
                }
            },
            "required": ["text"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let smi = SendMessageInput::from_value(&input);
        let channel = if smi.channel.trim().is_empty() {
            CHANNEL_TELEGRAM
        } else {
            smi.channel.trim()
        };
        match channel {
            "telegram" => self.send_telegram(&smi),
            other => Ok(ToolResult::err(format!(
                "channel {:?} is not supported (available: telegram)",
                other
            ))),
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_owned()
    } else {
        format!("{}...", &s[..n])
    }
}

/// Always registers the `send_message` tool so it appears in settings even
/// before any channel is configured.
pub fn register_send_message(
    reg: &super::tool::Registry,
    config_fn: Option<Box<dyn Fn() -> SendMessageRegistration + Send + Sync>>,
) {
    reg.register(SendMessageTool { config_fn, ..Default::default() });
}

#[path = "sendmessage_test.rs"]
#[cfg(test)]
mod sendmessage_test;