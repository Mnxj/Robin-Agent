use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;

use super::client::{CallResult, Client, ToolInfo};

/// StdioSession wraps a spawned subprocess and communicates via newline-
/// delimited JSON-RPC on stdin/stdout, mirroring the MCP stdio transport.
struct StdioSession {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
    next_id: AtomicU64,
}

impl StdioSession {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn send_recv(&self, method: &str, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let id = self.next_id();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');

        {
            let mut stdin = self.stdin.lock();
            stdin.write_all(line.as_bytes())?;
            stdin.flush()?;
        }

        // Read lines until we get a response with our id.
        loop {
            let mut buf = String::new();
            {
                let mut stdout = self.stdout.lock();
                let n = stdout.read_line(&mut buf)?;
                if n == 0 {
                    return Err(anyhow::anyhow!("stdio rpc {}: subprocess closed stdout (EOF)", method));
                }
            }
            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }
            let envelope: serde_json::Value = serde_json::from_str(trimmed)
                .map_err(|e| anyhow::anyhow!("stdio rpc {}: decode: {}", method, e))?;

            // Check if this is our response (has an id field that matches).
            if let Some(resp_id) = envelope.get("id") {
                let matched = resp_id.as_u64().map(|n| n == id)
                    .or_else(|| resp_id.as_i64().map(|n| n as u64 == id))
                    .unwrap_or(false);
                if matched {
                    if let Some(err) = envelope.get("error") {
                        let msg = err.get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown error");
                        return Err(anyhow::anyhow!("stdio rpc {}: {}", method, msg));
                    }
                    return Ok(envelope.get("result").cloned().unwrap_or(serde_json::Value::Null));
                }
            }
            // Notification or unmatched response — skip.
        }
    }

    fn send_notification(&self, method: &str, params: serde_json::Value) -> anyhow::Result<()> {
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&notif)?;
        line.push('\n');
        let mut stdin = self.stdin.lock();
        stdin.write_all(line.as_bytes())?;
        stdin.flush()?;
        Ok(())
    }

    fn list_tools_inner(&self) -> anyhow::Result<Vec<ToolInfo>> {
        let result = self.send_recv("tools/list", serde_json::json!({}))?;
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

    fn call_tool_inner(&self, name: &str, args: serde_json::Value) -> anyhow::Result<CallResult> {
        let params = serde_json::json!({ "name": name, "arguments": args });
        let result = self.send_recv("tools/call", params)
            .map_err(|e| anyhow::anyhow!("stdio tools/call {}: {}", name, e))?;
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
        Ok(CallResult { is_error, text: text_buf, raw: result })
    }

    fn close(&self) {
        let mut child = self.child.lock();
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// StdioClient wraps a StdioSession and exposes the same interface as the
/// HTTP-based Client, so Manager and adapters can treat them uniformly.
pub struct StdioClient {
    session: StdioSession,
}

impl StdioClient {
    pub fn list_tools(&self) -> anyhow::Result<Vec<ToolInfo>> {
        self.session.list_tools_inner()
    }

    pub fn call_tool(&self, name: &str, args: serde_json::Value) -> anyhow::Result<CallResult> {
        self.session.call_tool_inner(name, args)
    }

    pub fn close(&self) {
        self.session.close();
    }
}

/// mergedEnv returns parent ++ overrides, deduplicated by KEY with later
/// entries winning. Preserves the parent slice's order for keys that aren't
/// overridden. New keys from overrides are appended at the end.
pub fn merged_env(parent: Vec<String>, overrides: &HashMap<String, String>) -> Vec<String> {
    if overrides.is_empty() {
        return parent;
    }
    let mut idx: HashMap<String, usize> = HashMap::with_capacity(parent.len());
    let mut out: Vec<String> = Vec::with_capacity(parent.len() + overrides.len());
    for kv in parent {
        let key = kv.split('=').next().unwrap_or("").to_owned();
        idx.insert(key, out.len());
        out.push(kv);
    }
    for (k, v) in overrides {
        let entry = format!("{}={}", k, v);
        if let Some(&i) = idx.get(k) {
            out[i] = entry;
        } else {
            idx.insert(k.clone(), out.len());
            out.push(entry);
        }
    }
    out
}

/// forward_stderr reads lines from stderr and logs them at debug level.
fn forward_stderr(stderr: std::process::ChildStderr, id: String) {
    let reader = BufReader::new(stderr);
    for line in reader.lines() {
        match line {
            Ok(l) => tracing::debug!("mcp stdio stderr id={} line={}", id, l),
            Err(_) => break,
        }
    }
}

/// ConnectStdio spawns a subprocess and opens an MCP session against it over
/// stdin/stdout (newline-delimited JSON-RPC). Stderr is captured and forwarded
/// to tracing at Debug level.
///
/// id is used purely for log labelling. command is passed to Command::new and
/// looked up via PATH. env is merged onto std::env::vars().
pub fn connect_stdio(
    id: &str,
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
) -> anyhow::Result<Client> {
    if command.is_empty() {
        return Err(anyhow::anyhow!("mcp stdio {}: empty command", id));
    }

    let parent_env: Vec<String> = std::env::vars()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();
    let merged = merged_env(parent_env, env);

    let mut cmd = Command::new(command);
    cmd.args(args);
    // Clear inherited env and set the merged env explicitly.
    cmd.env_clear();
    for kv in &merged {
        if let Some(eq) = kv.find('=') {
            cmd.env(&kv[..eq], &kv[eq + 1..]);
        }
    }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn()
        .map_err(|e| anyhow::anyhow!("mcp stdio {}: spawn {}: {}", id, command, e))?;

    let stdin = child.stdin.take()
        .ok_or_else(|| anyhow::anyhow!("mcp stdio {}: no stdin", id))?;
    let stdout = child.stdout.take()
        .ok_or_else(|| anyhow::anyhow!("mcp stdio {}: no stdout", id))?;
    let stderr = child.stderr.take()
        .ok_or_else(|| anyhow::anyhow!("mcp stdio {}: no stderr", id))?;

    let id_owned = id.to_owned();
    std::thread::spawn(move || forward_stderr(stderr, id_owned));

    let session = StdioSession {
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        stdout: Mutex::new(BufReader::new(stdout)),
        next_id: AtomicU64::new(1),
    };

    // Perform the MCP initialize handshake.
    let init_result = session.send_recv("initialize", serde_json::json!({
        "protocolVersion": "2025-06-18",
        "capabilities": {},
        "clientInfo": { "name": "robin", "version": "0.0.0-stdio" }
    })).map_err(|e| anyhow::anyhow!("mcp stdio connect {} ({}): {}", id, command, e))?;

    // Validate the response looks like a real MCP initialize result (must have
    // protocolVersion). If the process echoes back our request or otherwise
    // returns something non-conformant, reject it here.
    if init_result.get("protocolVersion").is_none() {
        session.close();
        return Err(anyhow::anyhow!(
            "mcp stdio connect {} ({}): initialize response missing protocolVersion",
            id, command
        ));
    }
    tracing::debug!("mcp stdio {} initialized: {:?}", id, init_result);

    // Send notifications/initialized (fire-and-forget).
    let _ = session.send_notification("notifications/initialized", serde_json::json!({}));

    let stdio_client = StdioClient { session };
    Ok(Client::from_stdio(stdio_client))
}