use std::sync::Arc;

use serde_json::Value;

use crate::tools::{Tool, ToolResult};
use super::manager::ServerEntry;

/// ParallelSafeFn is the live-read callback an McpToolAdapter uses to query
/// its server's current parallel_safe flag. Implementations must be safe
/// for concurrent calls and should read from the live Config (which is
/// updated in-place by hot-reload, so values change between calls).
///
/// Pass None from tests or call sites that don't need hot-reload semantics —
/// adapters built with a None function report is_concurrency_safe == false.
pub type ParallelSafeFn = Arc<dyn Fn(&str) -> bool + Send + Sync + 'static>;

/// McpToolAdapter wraps a remote MCP tool as a Robin Tool. The adapter
/// is constructed by register_tools (one per remote tool per server) and
/// registered into a tools::Registry alongside core tools.
///
/// Holds Arc<ServerEntry> rather than a Client directly so that calls always
/// read the freshest client via entry.live() — picking up any in-process
/// Reconnect triggered by the Settings/Chat re-auth flow without re-registration.
///
/// The parallel_safe hint is read live from a closure on each
/// is_concurrency_safe call so that toggling mcp_servers[].parallel_safe via
/// the settings UI takes effect on the next agent run without restart.
pub struct McpToolAdapter {
    /// Name as Robin sees it (with prefix applied).
    full_name: String,
    /// Name as the MCP server knows it.
    remote_name: String,
    description: String,
    schema: Value,
    entry: Arc<ServerEntry>,
    /// nil-safe; None → is_concurrency_safe returns false
    parallel_safe: Option<ParallelSafeFn>,
}

impl McpToolAdapter {
    /// Package-internal constructor. register_tools is the only normal caller;
    /// tests may use it directly.
    pub fn new(
        full_name: impl Into<String>,
        remote_name: impl Into<String>,
        description: impl Into<String>,
        schema: Value,
        entry: Arc<ServerEntry>,
        parallel_safe: Option<ParallelSafeFn>,
    ) -> Self {
        Self {
            full_name: full_name.into(),
            remote_name: remote_name.into(),
            description: description.into(),
            schema,
            entry,
            parallel_safe,
        }
    }
}

impl Tool for McpToolAdapter {
    fn name(&self) -> &str {
        &self.full_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        self.schema.clone()
    }

    /// IsConcurrencySafe defers to the live config via the closure passed at
    /// construction time. Returns false when no closure was provided
    /// (preserves the conservative "MCP tools have unknown side effects"
    /// default for tests and call sites that don't wire hot-reload).
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        match &self.parallel_safe {
            None => false,
            Some(f) => f(&self.entry.id),
        }
    }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        // Unmarshal arguments. Defensive default: a null args map sends {} so
        // the envelope is well-formed even when the LLM produced no input.
        // If input is a string, parse it; if it's already a JSON object use it
        // directly; if it's null use {}.
        let args = if input.is_null() {
            Value::Object(Default::default())
        } else if input.is_object() {
            input.clone()
        } else if let Some(s) = input.as_str() {
            if s.is_empty() {
                Value::Object(Default::default())
            } else {
                match serde_json::from_str(s) {
                    Ok(v) => v,
                    Err(e) => return Ok(ToolResult::err(format!("invalid arguments JSON: {}", e))),
                }
            }
        } else {
            match serde_json::from_value::<serde_json::Map<String, Value>>(input.clone()) {
                Ok(m) => Value::Object(m),
                Err(e) => return Ok(ToolResult::err(format!("invalid arguments JSON: {}", e))),
            }
        };

        // Pre-flight circuit breaker.
        let n = self.entry.failure_count();
        if n >= super::manager::MAX_CONSECUTIVE_AUTH_FAILURES {
            let mut meta = serde_json::Map::new();
            meta.insert("auth_required".to_owned(), Value::String(self.entry.id.clone()));
            meta.insert("circuit_breaker".to_owned(), Value::Bool(true));
            return Ok(ToolResult {
                error: format!(
                    "MCP server {:?} has failed {} consecutive auth attempts including automatic reconnection — \
                    the server appears to be in a bad state that re-authentication alone is not fixing. \
                    Stop calling tools from this server in this conversation. \
                    Tell the user to investigate the server-side issue (the gateway may be misconfigured, \
                    the user may lack the required scopes, or the upstream service may be down) and try again later. \
                    Do NOT call any {}.* tools again until the user confirms the server is fixed.",
                    self.entry.id, n, self.entry.id,
                ),
                metadata: Some(meta),
                ..Default::default()
            });
        }

        let client_opt = self.entry.live();
        let client = match client_opt {
            None => {
                let mut meta = serde_json::Map::new();
                meta.insert("auth_required".to_owned(), Value::String(self.entry.id.clone()));
                return Ok(ToolResult {
                    error: format!("MCP server {:?} is not connected. Re-authenticate to reconnect.", self.entry.id),
                    metadata: Some(meta),
                    ..Default::default()
                });
            }
            Some(c) => c,
        };

        // Use a blocking runtime handle for async calls from a sync context.
        let result = {
            let rt = tokio::runtime::Handle::try_current();
            match rt {
                Ok(handle) => {
                    let remote_name = self.remote_name.clone();
                    let args_clone = args.clone();
                    let entry = self.entry.clone();
                    tokio::task::block_in_place(move || {
                        handle.block_on(async move {
                            // Try the call.
                            let res = client.call_tool(&remote_name, args_clone.clone()).await;
                            if let Err(ref e) = res {
                                if is_auth_failure(e) {
                                    // Attempt one reconnect+retry.
                                    if entry.reconnect().await.is_ok() {
                                        if let Some(retry_client) = entry.live() {
                                            if let Ok(r) = retry_client.call_tool(&remote_name, args_clone).await {
                                                return Ok(r);
                                            }
                                        }
                                    }
                                }
                            }
                            res
                        })
                    })
                }
                Err(_) => {
                    // No async runtime — return an error rather than panic.
                    return Ok(ToolResult::err("MCP tool call requires an async runtime"));
                }
            }
        };

        match result {
            Err(e) => {
                let mut tr = ToolResult::err(e.to_string());
                if is_auth_failure(&e) {
                    let mut meta = serde_json::Map::new();
                    meta.insert("auth_required".to_owned(), Value::String(self.entry.id.clone()));
                    tr.metadata = Some(meta);
                    tr.error = format!(
                        "MCP server {:?} rejected the call (auth expired). Re-authenticate to continue. Underlying error: {}",
                        self.entry.id, e
                    );
                    self.entry.record_failure();
                }
                Ok(tr)
            }
            Ok(call_res) => {
                if call_res.is_error {
                    let text = if call_res.text.is_empty() {
                        "tool returned isError without text".to_owned()
                    } else {
                        call_res.text.clone()
                    };
                    let mut tr = ToolResult::err(text.clone());
                    if is_auth_failure(&anyhow::anyhow!("{}", text)) {
                        let mut meta = serde_json::Map::new();
                        meta.insert("auth_required".to_owned(), Value::String(self.entry.id.clone()));
                        tr.metadata = Some(meta);
                        self.entry.record_failure();
                    } else {
                        // Tool ran and reported a non-auth error — the server is
                        // reachable. Reset the breaker.
                        self.entry.record_success();
                    }
                    Ok(tr)
                } else {
                    // Clean success — reset the breaker.
                    self.entry.record_success();
                    Ok(ToolResult::ok(call_res.text))
                }
            }
        }
    }
}

/// is_auth_failure reports whether err looks like a failure that
/// re-authentication or session reconnection would fix. Covers the common auth
/// signatures across providers (Cognito, Okta, Auth0, Azure AD, GitHub, Google),
/// the Streamable-HTTP session-rejection patterns, the OAuth refresh failure
/// modes, AND the SDK's session-terminal-state signals.
pub fn is_auth_failure(err: &anyhow::Error) -> bool {
    let s = err.to_string().to_lowercase();
    // Status codes
    if s.contains("401") || s.contains("403") { return true; }
    // Auth terms
    if s.contains("unauthorized") || s.contains("unauthenticated") { return true; }
    if s.contains("invalid_token") || s.contains("invalid token") { return true; }
    if s.contains("token expired") || s.contains("token has expired") { return true; }
    if s.contains("session expired") || s.contains("expired_token") { return true; }
    if s.contains("access denied") || s.contains("permission denied") { return true; }
    // MCP session-level rejections
    if s.contains("session not found") || s.contains("session_not_found") { return true; }
    if s.contains("session terminated") || s.contains("session is no longer valid") { return true; }
    if s.contains("no longer valid") { return true; }
    if s.contains("must re-authenticate") || s.contains("must reauthenticate") { return true; }
    if s.contains("please re-authenticate") || s.contains("re-authentication required") { return true; }
    // oauth2 refresh-failure shapes
    if s.contains("invalid_grant") { return true; }
    if s.contains("oauth2: cannot fetch token") { return true; }
    if s.contains("oauth2: token expired") { return true; }
    // MCP SDK session-terminal-state signals
    if s.contains("client is closing") { return true; }
    if s.contains("connection closed: calling") { return true; }
    // First-failure pattern: HTTP 400 on a tools/call inside SDK wrap
    if s.contains("sending \"") && s.contains(": bad request") { return true; }
    false
}
