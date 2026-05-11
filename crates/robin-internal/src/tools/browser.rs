use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::Value;

use crate::llm::ImageContent;
use super::ssrf::validate_url_not_internal;
use super::tool::{Tool, ToolResult};

const BROWSER_TIMEOUT: Duration = Duration::from_secs(120);
const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(600); // 10 minutes
const SESSION_MAX_COUNT: usize = 5;

/// Holds state for a persistent browser session.
/// In this Rust stub, we don't actually launch Chrome — we track the session
/// metadata and return errors for actions that require a real browser.
pub struct BrowserSession {
    pub last_used: Instant,
}

/// Provides headless browser automation via Chrome DevTools Protocol.
///
/// Two execution modes:
/// - Ephemeral (no "session" field): each call uses a fresh browser context.
/// - Persistent ("session" field): the named browser persists across calls.
///
/// NOTE: The actual browser automation (chromedp equivalent) is a TODO stub.
/// The session management, validation, and error-path logic is fully implemented.
pub struct BrowserTool {
    sessions: Mutex<HashMap<String, BrowserSession>>,
}

impl BrowserTool {
    pub fn new() -> Self {
        Self { sessions: Mutex::new(HashMap::new()) }
    }

    fn close_session(&self, name: &str) -> bool {
        self.sessions.lock().remove(name).is_some()
    }

    fn get_or_create_session(&self, name: &str) -> anyhow::Result<()> {
        let mut guard = self.sessions.lock();
        if guard.contains_key(name) {
            guard.get_mut(name).unwrap().last_used = Instant::now();
            return Ok(());
        }
        if guard.len() >= SESSION_MAX_COUNT {
            anyhow::bail!(
                "session limit reached ({}). Close an existing session before opening {:?}",
                SESSION_MAX_COUNT,
                name
            );
        }
        guard.insert(name.to_owned(), BrowserSession { last_used: Instant::now() });
        Ok(())
    }

    fn reap_idle_sessions(&self) {
        let now = Instant::now();
        self.sessions.lock().retain(|_, sess| {
            now.duration_since(sess.last_used) < SESSION_IDLE_TIMEOUT
        });
    }

    /// Exposed for tests.
    pub fn navigate_pub(&self, input: BrowserInputForTest) -> anyhow::Result<ToolResult> {
        self.navigate(&BrowserInput::from(input))
    }

    fn navigate(&self, input: &BrowserInput) -> anyhow::Result<ToolResult> {
        if input.url.is_empty() {
            return Ok(ToolResult::err("url is required for navigate action"));
        }
        if !input.url.starts_with("http://") && !input.url.starts_with("https://") {
            return Ok(ToolResult::err("url must start with http:// or https://"));
        }
        // TODO: actual browser navigation
        Ok(ToolResult {
            output: format!(
                "Navigated to {}\nPage title: (TODO: browser automation not implemented)",
                input.url
            ),
            ..Default::default()
        })
    }

    fn click(&self, input: &BrowserInput) -> anyhow::Result<ToolResult> {
        if input.selector.is_empty() {
            return Ok(ToolResult::err("selector is required for click action"));
        }
        // TODO: actual browser click
        Ok(ToolResult::ok(format!("Clicked element: {}", input.selector)))
    }

    fn type_text(&self, input: &BrowserInput) -> anyhow::Result<ToolResult> {
        if input.selector.is_empty() {
            return Ok(ToolResult::err("selector is required for type action"));
        }
        if input.text.is_empty() {
            return Ok(ToolResult::err("text is required for type action"));
        }
        // TODO: actual browser type
        Ok(ToolResult::ok(format!("Typed text into element: {}", input.selector)))
    }

    fn get_text(&self, input: &BrowserInput) -> anyhow::Result<ToolResult> {
        let selector = if input.selector.is_empty() { "body" } else { &input.selector };
        // TODO: actual browser get_text
        Ok(ToolResult::ok(format!("(TODO: get_text for selector: {})", selector)))
    }

    fn screenshot(&self, input: &BrowserInput) -> anyhow::Result<ToolResult> {
        // TODO: actual browser screenshot
        Ok(ToolResult::ok("(TODO: screenshot not implemented — browser automation stub)"))
    }

    fn evaluate(&self, input: &BrowserInput) -> anyhow::Result<ToolResult> {
        if input.script.is_empty() {
            return Ok(ToolResult::err("script is required for evaluate action"));
        }
        // TODO: actual browser evaluate
        Ok(ToolResult::ok(format!("(TODO: evaluate script: {})", input.script)))
    }
}

impl Default for BrowserTool {
    fn default() -> Self { Self::new() }
}

/// Browser input parsed from the JSON tool call.
/// Exposed as pub for tests (browser_test.rs accesses navigate/click directly).
#[derive(Debug, Default)]
pub struct BrowserInputForTest {
    pub action: String,
    pub session: String,
    pub url: String,
    pub selector: String,
    pub text: String,
    pub script: String,
    pub timeout: u64,
    pub wait_for: String,
    pub wait_ms: u64,
}

#[derive(Debug, Default)]
struct BrowserInput {
    action: String,
    session: String,
    url: String,
    selector: String,
    text: String,
    script: String,
    timeout: u64,
    wait_for: String,
    wait_ms: u64,
}

impl From<BrowserInputForTest> for BrowserInput {
    fn from(v: BrowserInputForTest) -> Self {
        Self {
            action: v.action,
            session: v.session,
            url: v.url,
            selector: v.selector,
            text: v.text,
            script: v.script,
            timeout: v.timeout,
            wait_for: v.wait_for,
            wait_ms: v.wait_ms,
        }
    }
}

impl BrowserInput {
    fn from_value(v: &Value) -> Self {
        Self {
            action: v.get("action").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            session: v.get("session").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            url: v.get("url").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            selector: v.get("selector").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            text: v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            script: v.get("script").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            timeout: v.get("timeout").and_then(|x| x.as_u64()).unwrap_or(0),
            wait_for: v.get("wait_for").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            wait_ms: v.get("wait_ms").and_then(|x| x.as_u64()).unwrap_or(0),
        }
    }
}

impl Tool for BrowserTool {
    fn name(&self) -> &str { "browser" }

    fn description(&self) -> &str {
        r#"Control a headless Chrome browser.

CRITICAL — RULE OF SESSIONS:
Each tool call uses an isolated, fresh browser by default. State (current URL, cookies, scroll, SPA hydration) does NOT carry over between calls.

If you intend to call this tool MORE THAN ONCE for the same page or flow, you MUST pass "session": "<any-label>" on EVERY call. Pick a label like "hormuz" or "github-login" and reuse it. Otherwise the second call starts on about:blank and your screenshot is blank, your get_text is empty, your click finds nothing.

ACTIONS:
- "navigate": Go to a URL. Requires "url".
- "click": Click an element. Requires "selector". Optional "url" to navigate first.
- "type": Type text into an input field. Requires "selector" and "text". Optional "url".
- "get_text": Get the inner HTML of an element or the full page. Optional "selector" (defaults to body). Optional "url".
- "screenshot": Take a screenshot of the current page. Optional "url". Returns the image.
- "evaluate": Execute JavaScript in the page. Requires "script". Optional "url".
- "close": Close a persistent session. Requires "session"."#
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "click", "type", "get_text", "screenshot", "evaluate", "close"],
                    "description": "The browser action to perform"
                },
                "session": {
                    "type": "string",
                    "description": "Optional session name for cross-call persistence"
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector for the target element"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type (required for type action)"
                },
                "script": {
                    "type": "string",
                    "description": "JavaScript code to evaluate (required for evaluate action)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 120)"
                },
                "wait_for": {
                    "type": "string",
                    "description": "Optional CSS selector to wait visible after navigation"
                },
                "wait_ms": {
                    "type": "integer",
                    "description": "Optional extra settle time in milliseconds after network-idle (default: 1000)"
                }
            },
            "required": ["action"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let bi = BrowserInput::from_value(&input);

        if bi.action.is_empty() {
            return Ok(ToolResult::err("action is required"));
        }

        // Handle "close" before any browser launch
        if bi.action == "close" {
            if bi.session.is_empty() {
                return Ok(ToolResult::err("session is required for close action"));
            }
            if self.close_session(&bi.session) {
                return Ok(ToolResult::ok(format!("Closed session {:?}", bi.session)));
            }
            return Ok(ToolResult::ok(format!("No active session {:?}", bi.session)));
        }

        // Validate URL early
        if !bi.url.is_empty() {
            if !bi.url.starts_with("http://") && !bi.url.starts_with("https://") {
                return Ok(ToolResult::err("url must start with http:// or https://"));
            }
            if let Err(e) = validate_url_not_internal(&bi.url) {
                return Ok(ToolResult::err(format!("navigate failed: {}", e)));
            }
        }

        if bi.action == "navigate" && bi.url.is_empty() {
            return Ok(ToolResult::err("url is required for navigate action"));
        }

        match bi.action.as_str() {
            "click" if bi.selector.is_empty() => {
                return Ok(ToolResult::err("selector is required for click action"))
            }
            "type" if bi.selector.is_empty() => {
                return Ok(ToolResult::err("selector is required for type action"))
            }
            "type" if bi.text.is_empty() => {
                return Ok(ToolResult::err("text is required for type action"))
            }
            "evaluate" if bi.script.is_empty() => {
                return Ok(ToolResult::err("script is required for evaluate action"))
            }
            _ => {}
        }

        // About:blank trap detection
        if bi.url.is_empty() && bi.session.is_empty() {
            match bi.action.as_str() {
                "screenshot" | "get_text" | "evaluate" | "click" | "type" => {
                    return Ok(ToolResult::err(format!(
                        "{} called without 'url' or 'session'. Each browser call uses a fresh browser starting on about:blank, so state does NOT carry over between calls. Either: (a) pass 'url' to navigate first, or (b) pass the same 'session: \"<name>\"' on every call in a multi-step flow (navigate, then act, then close).",
                        bi.action
                    )));
                }
                _ => {}
            }
        }

        // Reap idle sessions periodically
        self.reap_idle_sessions();

        // Acquire or create session
        if !bi.session.is_empty() {
            if let Err(e) = self.get_or_create_session(&bi.session) {
                return Ok(ToolResult::err(e.to_string()));
            }
        }

        // Dispatch action
        match bi.action.as_str() {
            "navigate" => self.navigate(&bi),
            "click" => self.click(&bi),
            "type" => self.type_text(&bi),
            "get_text" => self.get_text(&bi),
            "screenshot" => self.screenshot(&bi),
            "evaluate" => self.evaluate(&bi),
            other => Ok(ToolResult::err(format!(
                "unknown action: {:?} (valid: navigate, click, type, get_text, screenshot, evaluate, close)",
                other
            ))),
        }
    }
}

/// Global registry for graceful shutdown.
static BROWSER_REGISTRY: std::sync::LazyLock<Mutex<Vec<Arc<BrowserTool>>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

/// Creates a `BrowserTool` and registers it for global shutdown.
pub fn new_browser_tool() -> Arc<BrowserTool> {
    let t = Arc::new(BrowserTool::new());
    BROWSER_REGISTRY.lock().push(Arc::clone(&t));
    t
}

/// Closes all live browser sessions across every registered BrowserTool.
pub fn shutdown_browsers() {
    let tools = std::mem::take(&mut *BROWSER_REGISTRY.lock());
    for t in &tools {
        t.sessions.lock().clear();
    }
}

#[path = "browser_test.rs"]
#[cfg(test)]
mod browser_test;