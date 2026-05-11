use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use super::ssrf::validate_url_not_internal;
use super::tool::{Tool, ToolResult};

const MAX_FETCH_SIZE: usize = 5 * 1024 * 1024; // 5MB
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT_LENGTH: usize = 50_000;

/// Fetches a URL and returns its content as text (HTML is stripped to plain text).
pub struct WebFetchTool;

impl Tool for WebFetchTool {
    fn name(&self) -> &str { "web_fetch" }

    fn description(&self) -> &str {
        "Fetch the content of a web page at the given URL. Returns the page content converted to readable markdown text. Use this to read documentation, articles, API responses, or any web content."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch (must start with http:// or https://)"
                },
                "headers": {
                    "type": "object",
                    "description": "Optional HTTP headers to include in the request",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["url"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { true }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let url = match input.get("url").and_then(|v| v.as_str()) {
            Some(u) if !u.is_empty() => u.to_owned(),
            _ => return Ok(ToolResult::err("url is required")),
        };

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Ok(ToolResult::err("url must start with http:// or https://"));
        }

        if let Err(e) = validate_url_not_internal(&url) {
            return Ok(ToolResult::err(e.to_string()));
        }

        let extra_headers: HashMap<String, String> = input
            .get("headers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Use blocking reqwest for simplicity (avoid async fn in trait)
        let client = match reqwest::blocking::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .user_agent("Robin/1.0 (AI Agent Gateway)")
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 {
                    attempt.error("too many redirects (max 10)")
                } else {
                    // validate_url_not_internal is sync, call it here
                    if let Err(_e) = validate_url_not_internal(attempt.url().as_str()) {
                        attempt.error("redirect blocked: internal address")
                    } else {
                        attempt.follow()
                    }
                }
            }))
            .build()
        {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::err(format!("build client failed: {}", e))),
        };

        let mut req = client
            .get(&url)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,text/plain;q=0.8,application/json;q=0.7,*/*;q=0.5");
        for (k, v) in &extra_headers {
            req = req.header(k, v);
        }

        let resp = match req.send() {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::err(format!("fetch failed: {}", e))),
        };

        let status = resp.status();
        if status.as_u16() >= 400 {
            return Ok(ToolResult::err(format!("HTTP {}: {}", status.as_u16(), status)));
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();

        let body_bytes = match resp.bytes() {
            Ok(b) => b,
            Err(e) => return Ok(ToolResult::err(format!("read body failed: {}", e))),
        };

        // Limit body size
        let body_bytes = if body_bytes.len() > MAX_FETCH_SIZE {
            body_bytes.slice(..MAX_FETCH_SIZE)
        } else {
            body_bytes
        };

        let body_len = body_bytes.len();
        let content_str = String::from_utf8_lossy(&body_bytes).into_owned();

        // Strip HTML tags for readability
        let mut content = if content_type.contains("text/html") || content_type.contains("application/xhtml") {
            strip_html(&content_str)
        } else {
            content_str
        };

        // Truncate very long content
        if content.len() > MAX_OUTPUT_LENGTH {
            content.truncate(MAX_OUTPUT_LENGTH);
            content.push_str("\n\n[Content truncated — exceeded maximum length]");
        }

        let mut meta = serde_json::Map::new();
        meta.insert("url".to_owned(), Value::String(url));
        meta.insert("status".to_owned(), Value::Number(status.as_u16().into()));
        meta.insert("content_type".to_owned(), Value::String(content_type));
        meta.insert("length".to_owned(), Value::Number(body_len.into()));

        Ok(ToolResult { output: content, metadata: Some(meta), ..Default::default() })
    }
}

/// Simple HTML tag stripper and entity decoder for readability.
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out = out.replace("&amp;", "&");
    out = out.replace("&lt;", "<");
    out = out.replace("&gt;", ">");
    out = out.replace("&quot;", "\"");
    out = out.replace("&#x27;", "'");
    out = out.replace("&nbsp;", " ");
    out
}