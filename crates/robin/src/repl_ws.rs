use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

// ──────────────────────────────────────────────────────────────────────────────
// JSON-RPC types
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    pub id: i64,
}

#[derive(Deserialize, Clone)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<RpcError>,
    pub id: i64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rpc error {}: {}", self.code, self.message)
    }
}
impl std::error::Error for RpcError {}

// ──────────────────────────────────────────────────────────────────────────────
// Gateway URL helpers
// ──────────────────────────────────────────────────────────────────────────────

pub fn gateway_base_url(host: &str, port: u16) -> String {
    let h = if host.is_empty() { "127.0.0.1" } else { host };
    let p = if port == 0 { 18789 } else { port };
    format!("http://{h}:{p}")
}

pub fn http_to_ws(base_url: &str) -> anyhow::Result<String> {
    let u = url::Url::parse(base_url)?;
    let ws_scheme = match u.scheme() {
        "http" => "ws",
        "https" => "wss",
        s => anyhow::bail!("unsupported gateway scheme {:?}", s),
    };
    let after_scheme = base_url
        .find("://")
        .map(|i| &base_url[i + 3..])
        .unwrap_or(base_url);
    let authority = match after_scheme.find('/') {
        Some(i) => &after_scheme[..i],
        None => after_scheme,
    };
    Ok(format!("{ws_scheme}://{authority}/ws"))
}

pub fn probe_gateway(base_url: &str, auth_token: &str, timeout: Duration) -> bool {
    let client = match reqwest::blocking::Client::builder().timeout(timeout).build() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let url = format!("{base_url}/health");
    let mut req = client.get(&url);
    if !auth_token.is_empty() {
        req = req.header("Authorization", format!("Bearer {auth_token}"));
    }
    req.send().map(|r| r.status().is_success()).unwrap_or(false)
}

// ──────────────────────────────────────────────────────────────────────────────
// GatewayClient
// ──────────────────────────────────────────────────────────────────────────────

type WsSink = futures_util::stream::SplitSink<
    WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

pub struct GatewayClient {
    id_counter: AtomicI64,
    pending: Arc<Mutex<HashMap<i64, mpsc::Sender<JsonRpcResponse>>>>,
    sink: Arc<tokio::sync::Mutex<WsSink>>,
    closed: Arc<AtomicBool>,
}

impl GatewayClient {
    /// Connect to the gateway WebSocket, spawn the read-loop task, return Self.
    pub async fn dial(base_url: &str, auth_token: &str) -> anyhow::Result<Self> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let ws_url = http_to_ws(base_url)?;
        let mut request = ws_url.as_str().into_client_request()?;
        if !auth_token.is_empty() {
            request.headers_mut().insert(
                "Authorization",
                format!("Bearer {auth_token}").parse()?,
            );
        }
        let (ws_stream, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| anyhow::anyhow!("dial {ws_url}: {e}"))?;

        let (sink, stream) = ws_stream.split();

        let pending: Arc<Mutex<HashMap<i64, mpsc::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));

        // Spawn the read loop.
        let pending_clone = Arc::clone(&pending);
        let closed_clone = Arc::clone(&closed);
        tokio::spawn(async move {
            read_loop(stream, pending_clone, closed_clone).await;
        });

        Ok(GatewayClient {
            id_counter: AtomicI64::new(0),
            pending,
            sink: Arc::new(tokio::sync::Mutex::new(sink)),
            closed,
        })
    }

    fn next_id(&self) -> i64 {
        self.id_counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Register a new request id and return a receiver for its response.
    fn register(&self) -> anyhow::Result<(i64, mpsc::Receiver<JsonRpcResponse>)> {
        if self.closed.load(Ordering::SeqCst) {
            anyhow::bail!("gateway connection is closed");
        }
        let id = self.next_id();
        // Buffer 64 responses per in-flight request (mirrors Go channel buffer).
        let (tx, rx) = mpsc::channel(64);
        self.pending.lock().unwrap().insert(id, tx);
        Ok((id, rx))
    }

    /// Remove a request id from pending.
    fn release(&self, id: i64) {
        self.pending.lock().unwrap().remove(&id);
    }

    /// Serialize and send a JSON-RPC request over the WebSocket.
    async fn send(&self, req: &JsonRpcRequest) -> anyhow::Result<()> {
        let text = serde_json::to_string(req)?;
        let mut sink = self.sink.lock().await;
        sink.send(Message::Text(text)).await
            .map_err(|e| anyhow::anyhow!("ws send: {e}"))
    }

    /// Single-response RPC call.
    pub async fn call(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let (id, mut rx) = self.register()?;
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id,
        };
        // Ensure we always deregister, even on error.
        let result = async {
            self.send(&req).await?;
            let resp = rx
                .recv()
                .await
                .ok_or_else(|| anyhow::anyhow!("gateway connection closed waiting for response"))?;
            if let Some(err) = resp.error {
                return Err(anyhow::anyhow!("{err}"));
            }
            Ok(resp.result.unwrap_or(serde_json::Value::Null))
        }
        .await;
        self.release(id);
        result
    }

    /// Close the WebSocket connection.
    pub async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let mut sink = self.sink.lock().await;
        let _ = sink.close().await;
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Read loop (runs in a spawned task)
// ──────────────────────────────────────────────────────────────────────────────

async fn read_loop(
    mut stream: futures_util::stream::SplitStream<
        WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    >,
    pending: Arc<Mutex<HashMap<i64, mpsc::Sender<JsonRpcResponse>>>>,
    closed: Arc<AtomicBool>,
) {
    loop {
        match stream.next().await {
            None => {
                // Stream ended cleanly.
                break;
            }
            Some(Err(e)) => {
                eprintln!("\x1b[31m[gateway] read error: {e}\x1b[0m");
                break;
            }
            Some(Ok(msg)) => {
                let text = match msg {
                    Message::Text(t) => t,
                    Message::Binary(b) => match String::from_utf8(b) {
                        Ok(s) => s,
                        Err(_) => continue,
                    },
                    Message::Close(_) => break,
                    // Ping/Pong handled by tungstenite automatically.
                    _ => continue,
                };
                let resp: JsonRpcResponse = match serde_json::from_str(&text) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("\x1b[31m[gateway] parse error: {e}\x1b[0m");
                        continue;
                    }
                };
                let sender = pending.lock().unwrap().get(&resp.id).cloned();
                if let Some(tx) = sender {
                    // If the receiver was dropped (caller timed out / cancelled),
                    // just ignore the send error.
                    let _ = tx.send(resp).await;
                }
            }
        }
    }

    // Mark closed and drain all pending senders so callers unblock.
    closed.store(true, Ordering::SeqCst);
    let senders: Vec<_> = pending.lock().unwrap().drain().collect();
    drop(senders); // dropping Senders closes channels → receivers see None
}

// ──────────────────────────────────────────────────────────────────────────────
// Streaming chat turn
// ──────────────────────────────────────────────────────────────────────────────

pub async fn stream_chat_turn(
    gc: &GatewayClient,
    agent_id: &str,
    session_key: &str,
    text: &str,
) -> anyhow::Result<()> {
    let (id, mut rx) = gc.register()?;

    // Build params.
    let mut params = serde_json::json!({ "agentId": agent_id, "text": text });
    if !session_key.is_empty() {
        params["sessionKey"] = serde_json::Value::String(session_key.to_string());
    }

    gc.send(&JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "chat.send".to_string(),
        params: Some(params),
        id,
    })
    .await?;

    // Set up Ctrl-C abort.
    let (turn_cancel_tx, turn_cancel_rx) = tokio::sync::oneshot::channel::<()>();
    let gc_pending = Arc::clone(&gc.pending);
    let gc_sink = Arc::clone(&gc.sink);
    let gc_closed = Arc::clone(&gc.closed);

    // Spawn a task that waits for Ctrl-C and then calls chat.abort.
    let abort_task = tokio::spawn(async move {
        #[cfg(unix)]
        let mut sigint = {
            use tokio::signal::unix::{signal, SignalKind};
            signal(SignalKind::interrupt()).unwrap()
        };
        #[cfg(not(unix))]
        let mut sigint = tokio::signal::ctrl_c();

        tokio::select! {
            _ = async {
                #[cfg(unix)]
                sigint.recv().await;
                #[cfg(not(unix))]
                sigint.await.ok();
            } => {
                // Send chat.abort on best-effort basis.
                let abort_id = {
                    // We don't have access to id_counter here; use a fixed sentinel.
                    // The abort call is fire-and-forget; response goes to a one-shot channel.
                    let (tx, _rx) = mpsc::channel::<JsonRpcResponse>(1);
                    let abort_id: i64 = -1;
                    gc_pending.lock().unwrap().insert(abort_id, tx);
                    abort_id
                };
                if !gc_closed.load(Ordering::SeqCst) {
                    let req = JsonRpcRequest {
                        jsonrpc: "2.0".to_string(),
                        method: "chat.abort".to_string(),
                        params: None,
                        id: abort_id,
                    };
                    if let Ok(text) = serde_json::to_string(&req) {
                        let mut sink = gc_sink.lock().await;
                        let _ = sink.send(Message::Text(text)).await;
                    }
                }
                gc_pending.lock().unwrap().remove(&abort_id);
            }
            _ = turn_cancel_rx => {
                // Turn completed normally — nothing to do.
            }
        }
    });

    let mut response_text = String::new();
    let result = loop {
        match rx.recv().await {
            None => {
                break Err(anyhow::anyhow!("gateway connection closed mid-turn"));
            }
            Some(resp) => {
                if let Some(err) = resp.error {
                    break Err(anyhow::anyhow!("{err}"));
                }
                let raw = resp.result.unwrap_or(serde_json::Value::Null);
                match render_turn_event(&raw, &mut response_text) {
                    Err(e) => break Err(e),
                    Ok(true) => break Ok(()),
                    Ok(false) => {} // keep looping
                }
            }
        }
    };

    // Signal the abort task to stop and clean up.
    let _ = turn_cancel_tx.send(());
    abort_task.abort();
    gc.release(id);

    result
}

// ──────────────────────────────────────────────────────────────────────────────
// Event rendering
// ──────────────────────────────────────────────────────────────────────────────

pub fn render_turn_event(
    raw: &serde_json::Value,
    response_text: &mut String,
) -> anyhow::Result<bool> {
    let ev_type = raw["type"].as_str().unwrap_or("");
    match ev_type {
        "text_delta" => {
            response_text.push_str(raw["text"].as_str().unwrap_or(""));
        }
        "tool_call_start" => {
            print!("\n\x1b[36m[tool: {}]\x1b[0m\n", raw["tool"].as_str().unwrap_or(""));
        }
        "tool_result" => {
            let err = raw["error"].as_str().unwrap_or("");
            let out = raw["output"].as_str().unwrap_or("");
            if !err.is_empty() {
                print!("\x1b[31m  error: {err}\x1b[0m\n");
            } else if !out.is_empty() {
                print!("\x1b[90m  {}\x1b[0m\n", out.replace('\n', "\n  "));
            }
        }
        "compaction.start" => {
            print!("\x1b[90m🧹 Compacting…\x1b[0m\n");
        }
        "compaction.done" => {
            let turns = raw["turnsCompacted"].as_i64().unwrap_or(0);
            let ms = raw["durationMs"].as_i64().unwrap_or(0);
            print!("\x1b[90m🧹 Compacted {turns} turns in {ms}ms\x1b[0m\n");
        }
        "compaction.skipped" => {
            let reason = raw["reason"].as_str().unwrap_or("");
            if reason == "reactive" {
                let skipped = raw["skipped"].as_str().unwrap_or("");
                print!(
                    "\x1b[33m⚠ Compaction skipped during reactive retry: {skipped}\x1b[0m\n"
                );
            }
        }
        "error" => {
            print!("\n\x1b[31mError: {}\x1b[0m\n", raw["message"].as_str().unwrap_or(""));
            flush_markdown(response_text);
            return Ok(true);
        }
        "aborted" => {
            print!("\n\x1b[33m[aborted]\x1b[0m\n");
            flush_markdown(response_text);
            return Ok(true);
        }
        "done" => {
            flush_markdown(response_text);
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

// ──────────────────────────────────────────────────────────────────────────────
// Markdown rendering
// ──────────────────────────────────────────────────────────────────────────────

/// Render accumulated markdown to the terminal, then clear the buffer.
/// Uses pulldown-cmark to parse and emit plain text; if anything goes wrong,
/// falls back to printing the raw string.
pub fn flush_markdown(buf: &mut String) {
    if buf.is_empty() {
        return;
    }
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    use std::fmt::Write as _;

    let mut out = String::with_capacity(buf.len());
    let parser = Parser::new_ext(buf, Options::all());

    // Simple terminal-friendly rendering: emit text, annotate code blocks, skip HTML.
    let mut in_code_block = false;
    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
                out.push_str("\x1b[90m");
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                out.push_str("\x1b[0m");
            }
            Event::Start(Tag::Heading { level, .. }) => {
                // Bold heading prefix.
                let hashes = "#".repeat(level as usize);
                let _ = write!(out, "\n\x1b[1m{hashes} ");
            }
            Event::End(TagEnd::Heading(_)) => {
                out.push_str("\x1b[0m\n");
            }
            Event::Start(Tag::Strong) => out.push_str("\x1b[1m"),
            Event::End(TagEnd::Strong) => out.push_str("\x1b[0m"),
            Event::Start(Tag::Emphasis) => out.push_str("\x1b[3m"),
            Event::End(TagEnd::Emphasis) => out.push_str("\x1b[0m"),
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => out.push('\n'),
            Event::Start(Tag::Item) => out.push_str("  • "),
            Event::End(TagEnd::Item) => out.push('\n'),
            Event::Text(t) => out.push_str(&t),
            Event::Code(t) => {
                out.push_str("\x1b[90m`");
                out.push_str(&t);
                out.push_str("`\x1b[0m");
            }
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::Rule => out.push_str("────────────────────────────────────────\n"),
            _ => {
                if in_code_block {
                    // raw text inside code blocks already handled via Event::Text
                }
            }
        }
    }

    print!("{out}");
    buf.clear();
}

// ──────────────────────────────────────────────────────────────────────────────
// Session / compact helpers
// ──────────────────────────────────────────────────────────────────────────────

async fn handle_sessions_list(gc: &GatewayClient, agent_id: &str, current_key: &str) {
    match gc
        .call(
            "session.list",
            Some(serde_json::json!({ "agentId": agent_id })),
        )
        .await
    {
        Err(e) => eprintln!("\x1b[31mError listing sessions: {e}\x1b[0m"),
        Ok(raw) => {
            let sessions = raw["sessions"].as_array().cloned().unwrap_or_default();
            if sessions.is_empty() {
                println!("No sessions found.");
                return;
            }
            println!("Sessions:");
            for s in &sessions {
                let key = s["key"].as_str().unwrap_or("");
                let count = s["entryCount"].as_i64().unwrap_or(0);
                let ts = s["lastActivity"].as_i64().unwrap_or(0);
                let last = if ts > 0 {
                    // Format unix timestamp as "YYYY-MM-DD HH:MM" (matching Go layout).
                    use std::time::{Duration as StdDuration, UNIX_EPOCH};
                    let dt = UNIX_EPOCH + StdDuration::from_secs(ts as u64);
                    // chrono is available in workspace deps.
                    let naive = chrono::DateTime::<chrono::Utc>::from(dt);
                    naive.format("%Y-%m-%d %H:%M").to_string()
                } else {
                    "-".to_string()
                };
                let marker = if s["active"].as_bool().unwrap_or(false) || key == current_key {
                    "* "
                } else {
                    "  "
                };
                println!("  {marker}{key:<20}  {count} entries  {last}");
            }
        }
    }
}

pub async fn handle_session_new(
    gc: &GatewayClient,
    agent_id: &str,
    name: &str,
) -> anyhow::Result<String> {
    let mut params = serde_json::json!({ "agentId": agent_id });
    if !name.is_empty() {
        params["name"] = serde_json::Value::String(name.to_string());
    }
    let raw = gc.call("session.new", Some(params)).await?;
    raw["sessionKey"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("parse response: missing sessionKey"))
}

pub async fn handle_session_switch(
    gc: &GatewayClient,
    agent_id: &str,
    session_key: &str,
) -> anyhow::Result<()> {
    gc.call(
        "session.switch",
        Some(serde_json::json!({
            "agentId": agent_id,
            "sessionKey": session_key,
        })),
    )
    .await?;
    Ok(())
}

async fn handle_compact(
    gc: &GatewayClient,
    agent_id: &str,
    session_key: &str,
    instructions: &str,
) {
    let mut params = serde_json::json!({ "agentId": agent_id });
    if !session_key.is_empty() {
        params["sessionKey"] = session_key.into();
    }
    if !instructions.is_empty() {
        params["instructions"] = instructions.into();
    }

    println!("\x1b[90m🧹 Compacting…\x1b[0m");
    match gc.call("chat.compact", Some(params)).await {
        Err(e) => eprintln!("\x1b[31mCompaction failed: {e}\x1b[0m"),
        Ok(raw) => {
            if !raw["compacted"].as_bool().unwrap_or(false) {
                let skip = raw["skipped"].as_str().unwrap_or("");
                match skip {
                    "too_short" => println!("\x1b[90mSession too short to compact.\x1b[0m"),
                    "summarizer_error" => {
                        println!("\x1b[33mCompaction skipped: summarizer error.\x1b[0m")
                    }
                    "empty_summary" => {
                        println!("\x1b[33mCompaction skipped: model returned no summary.\x1b[0m")
                    }
                    "timeout" => {
                        println!("\x1b[33mCompaction skipped: timed out.\x1b[0m")
                    }
                    "cancelled" => {
                        println!("\x1b[33mCompaction cancelled.\x1b[0m")
                    }
                    other if !other.is_empty() => {
                        println!("\x1b[33mCompaction skipped: {other}\x1b[0m")
                    }
                    _ => {}
                }
                return;
            }
            let turns = raw["turnsCompacted"].as_i64().unwrap_or(0);
            let ms = raw["durationMs"].as_i64().unwrap_or(0);
            println!("\x1b[90m🧹 Compacted {turns} turns in {ms}ms\x1b[0m");
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// REPL entry point
// ──────────────────────────────────────────────────────────────────────────────

pub async fn run_chat_via_gateway(
    agent_id: &str,
    model_str: &str,
    base_url: &str,
    auth_token: &str,
) -> anyhow::Result<()> {
    let gc = GatewayClient::dial(base_url, auth_token).await?;
    println!(
        "Robin chat — agent {:?} via gateway {} (model: {})",
        agent_id, base_url, model_str
    );
    println!("Connected to running gateway; sessions are shared with the web chat at /chat.");
    println!("Type /quit to exit, /sessions to list sessions, /new to create a new session.");
    println!();

    let mut current_session_key = String::new();

    loop {
        print!("> ");
        use std::io::Write as _;
        let _ = std::io::stdout().flush();

        let input = read_line_stdin()?;
        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }

        match input.as_str() {
            "/quit" | "/exit" => {
                println!("Goodbye!");
                gc.close().await;
                return Ok(());
            }
            "/sessions" => handle_sessions_list(&gc, agent_id, &current_session_key).await,
            s if s.starts_with("/new") => {
                let name = s.trim_start_matches("/new").trim().to_string();
                match handle_session_new(&gc, agent_id, &name).await {
                    Ok(key) => {
                        println!("Switched to new session {:?}", key);
                        current_session_key = key;
                    }
                    Err(e) => eprintln!("\x1b[31mError creating session: {e}\x1b[0m"),
                }
            }
            s if s.starts_with("/switch ") => {
                let name = s.trim_start_matches("/switch ").trim();
                if name.is_empty() {
                    println!("Usage: /switch <session-key>");
                    continue;
                }
                match handle_session_switch(&gc, agent_id, name).await {
                    Ok(()) => {
                        current_session_key = name.to_string();
                        println!("Switched to session {:?}", name);
                    }
                    Err(e) => eprintln!("\x1b[31mError switching session: {e}\x1b[0m"),
                }
            }
            s if s.starts_with("/compact") => {
                let instructions = s.trim_start_matches("/compact").trim().to_string();
                handle_compact(&gc, agent_id, &current_session_key, &instructions).await;
            }
            _ => {
                if let Err(e) =
                    stream_chat_turn(&gc, agent_id, &current_session_key, &input).await
                {
                    eprintln!("\x1b[31mError: {e}\x1b[0m");
                }
            }
        }
    }
}

fn read_line_stdin() -> anyhow::Result<String> {
    use std::io::Read;
    let mut buf = [0u8; 1];
    let mut out = Vec::new();
    loop {
        let n = std::io::stdin().read(&mut buf)?;
        if n == 0 {
            break;
        }
        if buf[0] == b'\n' {
            break;
        }
        out.push(buf[0]);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}