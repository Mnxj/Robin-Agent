use std::sync::atomic::{AtomicU64, Ordering};
use parking_lot::Mutex;

/// ToolInfo is the minimal projection of an MCP tool definition that the
/// harness needs.
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// CallResult is the result of an MCP tools/call.
#[derive(Debug, Clone)]
pub struct CallResult {
    pub is_error: bool,
    /// concatenated text-content blocks
    pub text: String,
    /// full result JSON for debugging
    pub raw: serde_json::Value,
}

// ── HTTP transport internals ─────────────────────────────────────────────────

/// MCP JSON-RPC request envelope.
#[derive(Debug, serde::Serialize)]
struct JsonRpcRequest<'a, P: serde::Serialize> {
    jsonrpc: &'a str,
    id: u64,
    method: &'a str,
    params: P,
}

/// MCP JSON-RPC response envelope.
#[derive(Debug, serde::Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    id: Option<serde_json::Value>,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, serde::Deserialize)]
struct JsonRpcError {
    #[allow(dead_code)]
    code: i64,
    message: String,
}

/// Inner state for an HTTP-transport session.
struct HttpSession {
    url: String,
    http: reqwest::Client,
    session_id: Mutex<Option<String>>,
    next_id: AtomicU64,
}

impl HttpSession {
    async fn rpc<P: serde::Serialize>(
        &self,
        method: &str,
        params: P,
    ) -> anyhow::Result<serde_json::Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method,
            params,
        };

        let mut req = self.http.post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");

        if let Some(sid) = self.session_id.lock().as_deref() {
            req = req.header("Mcp-Session-Id", sid);
        }

        let resp = req.json(&body).send().await
            .map_err(|e| anyhow::anyhow!("mcp rpc {}: send: {}", method, e))?;

        // Capture any session ID the server assigns.
        if let Some(sid) = resp.headers().get("Mcp-Session-Id") {
            if let Ok(s) = sid.to_str() {
                *self.session_id.lock() = Some(s.to_owned());
            }
        }

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "mcp rpc {}: {}: {}",
                method,
                status,
                text.trim()
            ));
        }

        let envelope: JsonRpcResponse = resp.json().await
            .map_err(|e| anyhow::anyhow!("mcp rpc {}: decode response: {}", method, e))?;

        if let Some(err) = envelope.error {
            return Err(anyhow::anyhow!("mcp rpc {}: {}", method, err.message));
        }

        Ok(envelope.result.unwrap_or(serde_json::Value::Null))
    }

    async fn list_tools(&self) -> anyhow::Result<Vec<ToolInfo>> {
        let result = self.rpc("tools/list", serde_json::json!({})).await
            .map_err(|e| anyhow::anyhow!("mcp tools/list: {}", e))?;
        parse_tool_list(result)
    }

    async fn call_tool(&self, name: &str, args: serde_json::Value) -> anyhow::Result<CallResult> {
        let params = serde_json::json!({ "name": name, "arguments": args });
        let result = self.rpc("tools/call", params).await
            .map_err(|e| anyhow::anyhow!("mcp tools/call {}: {}", name, e))?;
        Ok(parse_call_result(result))
    }
}

// ── Shared parsing helpers ────────────────────────────────────────────────────

fn parse_tool_list(result: serde_json::Value) -> anyhow::Result<Vec<ToolInfo>> {
    let tools = result.get("tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::with_capacity(tools.len());
    for t in tools {
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("").to_owned();
        let description = t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_owned();
        let input_schema = t.get("inputSchema").cloned()
            .unwrap_or(serde_json::json!({"type": "object"}));
        out.push(ToolInfo { name, description, input_schema });
    }
    Ok(out)
}

fn parse_call_result(result: serde_json::Value) -> CallResult {
    let is_error = result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut text_buf = String::new();
    if let Some(content) = result.get("content").and_then(|v| v.as_array()) {
        for block in content {
            if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    text_buf.push_str(t);
                }
            }
        }
    }
    CallResult { is_error, text: text_buf, raw: result }
}

// ── Client (transport-polymorphic) ───────────────────────────────────────────

/// Transport-internal representation. Kept private; callers only see Client.
enum Transport {
    Http(HttpSession),
    Stdio(super::stdio::StdioClient),
}

/// Client is a thin wrapper over an MCP session that supports both the
/// Streamable-HTTP transport and the stdio (subprocess) transport.
///
/// Construct via `connect_http` or `connect_stdio`.
pub struct Client {
    transport: Transport,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").finish()
    }
}

impl Client {
    /// Build a Client from a StdioClient (called by stdio.rs).
    pub(crate) fn from_stdio(inner: super::stdio::StdioClient) -> Self {
        Self { transport: Transport::Stdio(inner) }
    }

    /// Close the MCP session / terminate the subprocess.
    pub fn close(&self) -> anyhow::Result<()> {
        match &self.transport {
            Transport::Http(_) => Ok(()),
            Transport::Stdio(s) => { s.close(); Ok(()) }
        }
    }

    /// ListTools returns the tools exposed by the server.
    pub async fn list_tools(&self) -> anyhow::Result<Vec<ToolInfo>> {
        match &self.transport {
            Transport::Http(h) => h.list_tools().await,
            Transport::Stdio(s) => {
                // Run the blocking stdio call on the blocking thread pool.
                // We borrow the reference by using a raw pointer to avoid
                // lifetime issues with tokio::task::spawn_blocking.
                s.list_tools()
            }
        }
    }

    /// CallTool invokes a tool by name with the supplied JSON arguments.
    pub async fn call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> anyhow::Result<CallResult> {
        match &self.transport {
            Transport::Http(h) => h.call_tool(name, args).await,
            Transport::Stdio(s) => s.call_tool(name, args),
        }
    }
}

/// ConnectHTTP opens an MCP session against server_url over the Streamable
/// HTTP transport, using the supplied reqwest::Client (which is expected to
/// inject auth). The returned Client must be close()d when done.
///
/// The initialize / initialized handshake is performed here. On failure the
/// function returns an error; no Client is returned.
pub async fn connect_http(
    server_url: &str,
    http_client: reqwest::Client,
) -> anyhow::Result<Client> {
    let session = HttpSession {
        url: server_url.to_owned(),
        http: http_client,
        session_id: Mutex::new(None),
        next_id: AtomicU64::new(1),
    };

    // Send initialize.
    let init_params = serde_json::json!({
        "protocolVersion": "2025-06-18",
        "capabilities": {},
        "clientInfo": {
            "name": "robin",
            "version": "0.0.0-stage1-harness"
        }
    });
    session.rpc("initialize", init_params).await
        .map_err(|e| anyhow::anyhow!("mcp connect: {}", e))?;

    // Send notifications/initialized (fire-and-forget; ignore errors).
    let notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    });
    let mut req = session.http.post(&session.url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json");
    if let Some(sid) = session.session_id.lock().as_deref() {
        req = req.header("Mcp-Session-Id", sid);
    }
    let _ = req.json(&notif).send().await;

    Ok(Client { transport: Transport::Http(session) })
}