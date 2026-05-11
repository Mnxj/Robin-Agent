use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::Arc,
    time::Instant,
};
use tokio::sync::mpsc;
use tracing::info;

// ─── JSON-RPC types ────────────────────────────────────────────────────────

/// A JSON-RPC 2.0 request.
#[derive(Deserialize, Debug, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub id: Value,
}

/// A JSON-RPC 2.0 response.
#[derive(Serialize, Debug, Clone)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    pub id: Value,
}

impl JsonRpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: Some(result),
            error: None,
            id,
        }
    }

    fn err(id: Value, code: i64, message: &str) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(json!({"code": code, "message": message})),
            id,
        }
    }
}

// ─── Traits for external dependencies ─────────────────────────────────────

/// An agent configuration entry.
#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub model: String,
    pub workspace: String,
    pub context_window: u64,
}

/// A session entry summary.
#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub key: String,
    pub entry_count: usize,
    pub created_at: i64,
    pub last_activity: i64,
}

/// A history entry.
#[derive(Clone, Debug)]
pub enum HistoryEntry {
    Message {
        role: String,
        text: String,
    },
    ToolCall {
        tool: String,
        id: String,
        input: Value,
    },
    ToolResult {
        tool_call_id: String,
        output: String,
        error: String,
        images: Vec<ImageData>,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct ImageData {
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub data: String,
}

/// Events emitted by an agent run.
#[derive(Debug)]
pub enum AgentEvent {
    TextDelta(String),
    ToolCallStart {
        tool: String,
        id: String,
        input: Value,
    },
    ToolResult {
        tool: String,
        id: String,
        input: Value,
        output: String,
        error: String,
        images: Vec<ImageData>,
        auth_required: Option<String>,
    },
    Done {
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_input_tokens: u64,
        cache_read_input_tokens: u64,
        context_window: u64,
        model: String,
    },
    Error(String),
    Aborted,
    Trace {
        phase: String,
        dur_ms: Option<i64>,
        at_ms: Option<i64>,
        attrs: HashMap<String, Value>,
    },
}

/// Trait for the agent runtime that the websocket handler drives.
pub trait AgentRuntime: Send + Sync {
    fn run(
        self: Arc<Self>,
        text: String,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<tokio::sync::mpsc::Receiver<AgentEvent>>> + Send>,
    >;
}

/// Trait for the config surface the handler needs.
pub trait ConfigSurface: Send + Sync {
    fn list_agents(&self) -> Vec<AgentConfig>;
    fn get_agent(&self, id: &str) -> Option<AgentConfig>;
}

/// Trait for the session store.
pub trait SessionStoreTrait: Send + Sync {
    fn list(&self, agent_id: &str) -> anyhow::Result<Vec<SessionSummary>>;
    fn exists(&self, agent_id: &str, key: &str) -> bool;
    fn create(&self, agent_id: &str, key: &str) -> anyhow::Result<()>;
    fn delete(&self, agent_id: &str, key: &str) -> anyhow::Result<()>;
    fn history(&self, agent_id: &str, key: &str) -> anyhow::Result<Vec<HistoryEntry>>;
}

/// Trait for the job scheduler.
pub trait JobSchedulerTrait: Send + Sync {
    fn list_jobs(&self) -> Vec<Value>;
    fn pause_job(&self, name: &str) -> anyhow::Result<()>;
    fn resume_job(&self, name: &str) -> anyhow::Result<()>;
    fn remove_job(&self, name: &str) -> anyhow::Result<()>;
    fn add_job(&self, name: &str, schedule: &str, prompt: &str) -> anyhow::Result<()>;
    fn update_job_schedule(&self, name: &str, schedule: &str) -> anyhow::Result<()>;
}

/// Trait for building agent runtimes.
pub trait AgentBuilder: Send + Sync {
    fn build(
        &self,
        agent_id: &str,
        session_key: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<Arc<dyn AgentRuntime>>> + Send + '_>,
    >;
}

// ─── WebSocketHandler state ───────────────────────────────────────────────

/// Shared state for the WebSocket handler.
pub struct WebSocketHandlerState {
    pub config: Arc<RwLock<Arc<dyn ConfigSurface>>>,
    pub session_store: Arc<dyn SessionStoreTrait>,
    pub job_scheduler: Option<Arc<dyn JobSchedulerTrait>>,
    pub agent_builder: Option<Arc<dyn AgentBuilder>>,
    pub origin_checker: Arc<dyn Fn(&axum::http::HeaderMap) -> bool + Send + Sync>,
}

impl WebSocketHandlerState {
    pub fn new(
        config: Arc<dyn ConfigSurface>,
        session_store: Arc<dyn SessionStoreTrait>,
    ) -> Self {
        WebSocketHandlerState {
            config: Arc::new(RwLock::new(config)),
            session_store,
            job_scheduler: None,
            agent_builder: None,
            origin_checker: Arc::new(|_| true),
        }
    }

    pub fn update_config(&self, cfg: Arc<dyn ConfigSurface>) {
        *self.config.write() = cfg;
    }

    pub fn set_job_scheduler(&mut self, js: Arc<dyn JobSchedulerTrait>) {
        self.job_scheduler = Some(js);
    }

    pub fn set_agent_builder(&mut self, ab: Arc<dyn AgentBuilder>) {
        self.agent_builder = Some(ab);
    }

    pub fn set_origin_checker(
        &mut self,
        checker: Arc<dyn Fn(&axum::http::HeaderMap) -> bool + Send + Sync>,
    ) {
        self.origin_checker = checker;
    }
}

// ─── Per-connection state ─────────────────────────────────────────────────

struct ConnState {
    active_cancel: Option<tokio::sync::oneshot::Sender<()>>,
    session_keys: HashMap<String, String>, // agentId -> sessionKey
}

// ─── WebSocket upgrade handler ────────────────────────────────────────────

/// Axum extractor handler for WebSocket upgrades.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<WebSocketHandlerState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let checker = state.origin_checker.clone();
    if !checker(&headers) {
        return axum::http::Response::builder()
            .status(403)
            .body(axum::body::Body::from("Forbidden: origin not allowed"))
            .unwrap();
    }
    ws.on_upgrade(move |socket| handle_connection(socket, state))
}

async fn handle_connection(socket: WebSocket, state: Arc<WebSocketHandlerState>) {
    info!("websocket client connected");

    let (mut sink, mut stream) = socket.split();

    // Per-connection write channel (serialises concurrent writes)
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    // Spawn the writer task
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    let conn_state = Arc::new(Mutex::new(ConnState {
        active_cancel: None,
        session_keys: HashMap::new(),
    }));

    // Rate limiter: max 30 messages/sec (token bucket)
    const RATE_LIMIT: f64 = 30.0;
    let mut tokens = RATE_LIMIT;
    let mut last_refill = Instant::now();

    while let Some(Ok(msg)) = stream.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };

        // Token bucket refill
        let now = Instant::now();
        let elapsed = now.duration_since(last_refill).as_secs_f64();
        tokens = (tokens + elapsed * RATE_LIMIT).min(RATE_LIMIT);
        last_refill = now;

        if tokens < 1.0 {
            let resp = JsonRpcResponse::err(Value::Null, -32000, "rate limit exceeded");
            let _ = tx.send(serde_json::to_string(&resp).unwrap_or_default());
            continue;
        }
        tokens -= 1.0;

        // Parse the JSON-RPC request
        let req: JsonRpcRequest = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(_) => {
                let resp = JsonRpcResponse::err(Value::Null, -32700, "Parse error");
                let _ = tx.send(serde_json::to_string(&resp).unwrap_or_default());
                continue;
            }
        };

        dispatch(req, state.clone(), conn_state.clone(), tx.clone()).await;
    }

    // Cancel any active run on disconnect
    {
        let mut cs = conn_state.lock();
        if let Some(cancel) = cs.active_cancel.take() {
            let _ = cancel.send(());
        }
    }

    writer_task.abort();
    info!("websocket client disconnected");
}

// ─── Dispatch ────────────────────────────────────────────────────────────

async fn dispatch(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    conn_state: Arc<Mutex<ConnState>>,
    tx: mpsc::UnboundedSender<String>,
) {
    match req.method.as_str() {
        "chat.send" => handle_chat_send(req, state, conn_state, tx).await,
        "chat.abort" => handle_chat_abort(req, conn_state, tx).await,
        "agent.status" => handle_agent_status(req, state, tx).await,
        "session.list" => handle_session_list(req, state, conn_state, tx).await,
        "session.new" => handle_session_new(req, state, conn_state, tx).await,
        "session.switch" => handle_session_switch(req, conn_state, tx).await,
        "session.history" => handle_session_history(req, state, conn_state, tx).await,
        "session.clear" => handle_session_clear(req, state, conn_state, tx).await,
        "jobs.list" => handle_jobs_list(req, state, tx).await,
        "jobs.pause" => handle_jobs_pause(req, state, tx).await,
        "jobs.resume" => handle_jobs_resume(req, state, tx).await,
        "jobs.remove" => handle_jobs_remove(req, state, tx).await,
        "jobs.add" => handle_jobs_add(req, state, tx).await,
        "jobs.update" => handle_jobs_update(req, state, tx).await,
        _ => {
            send_response(
                &tx,
                JsonRpcResponse::err(req.id, -32601, "Method not found"),
            );
        }
    }
}

fn send_response(tx: &mpsc::UnboundedSender<String>, resp: JsonRpcResponse) {
    if let Ok(json) = serde_json::to_string(&resp) {
        let _ = tx.send(json);
    }
}

fn send_result(tx: &mpsc::UnboundedSender<String>, id: Value, result: Value) {
    send_response(tx, JsonRpcResponse::ok(id, result));
}

fn send_error(tx: &mpsc::UnboundedSender<String>, id: Value, code: i64, msg: &str) {
    send_response(tx, JsonRpcResponse::err(id, code, msg));
}

fn resolve_session_key(
    conn_state: &Arc<Mutex<ConnState>>,
    param_key: &str,
    agent_id: &str,
) -> String {
    if !param_key.is_empty() {
        return param_key.to_string();
    }
    let cs = conn_state.lock();
    cs.session_keys
        .get(agent_id)
        .cloned()
        .unwrap_or_else(|| "ws_default".to_string())
}

// ─── chat.send ───────────────────────────────────────────────────────────

async fn handle_chat_send(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    conn_state: Arc<Mutex<ConnState>>,
    tx: mpsc::UnboundedSender<String>,
) {
    let agent_id = req
        .params
        .get("agentId")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    let agent_id = if agent_id.is_empty() {
        "default".to_string()
    } else {
        agent_id
    };

    let text = match req.params.get("text").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            send_error(&tx, req.id, -32602, "Invalid params: text required");
            return;
        }
    };

    let session_key_param = req
        .params
        .get("sessionKey")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let session_key = resolve_session_key(&conn_state, &session_key_param, &agent_id);

    // Check agent exists
    let cfg = state.config.read().clone();
    if cfg.get_agent(&agent_id).is_none() {
        send_error(&tx, req.id, -32602, "Unknown agent");
        return;
    }

    // Build runtime
    let builder = match &state.agent_builder {
        Some(b) => b.clone(),
        None => {
            send_error(&tx, req.id, -32603, "Agent builder not configured");
            return;
        }
    };

    let runtime = match builder.build(&agent_id, &session_key).await {
        Ok(r) => r,
        Err(e) => {
            send_error(
                &tx,
                req.id,
                -32603,
                &format!("Build runtime failed: {}", e),
            );
            return;
        }
    };

    // Cancel any existing run
    {
        let mut cs = conn_state.lock();
        if let Some(cancel) = cs.active_cancel.take() {
            let _ = cancel.send(());
        }
    }

    // Start the new run
    let mut events_rx = match runtime.run(text).await {
        Ok(rx) => rx,
        Err(e) => {
            send_error(&tx, req.id, -32603, &e.to_string());
            return;
        }
    };

    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut cs = conn_state.lock();
        cs.active_cancel = Some(cancel_tx);
    }

    let tx_clone = tx.clone();
    let req_id = req.id.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut cancel_rx => {
                    // Run cancelled — send aborted
                    send_result(&tx_clone, req_id, json!({"type": "aborted"}));
                    break;
                }
                maybe_event = events_rx.recv() => {
                    match maybe_event {
                        None => break, // channel closed
                        Some(event) => {
                            let result = event_to_value(event);
                            if let Some(v) = result {
                                send_result(&tx_clone, req_id.clone(), v);
                            }
                        }
                    }
                }
            }
        }
    });
}

fn event_to_value(event: AgentEvent) -> Option<Value> {
    match event {
        AgentEvent::TextDelta(text) => Some(json!({"type": "text_delta", "text": text})),
        AgentEvent::ToolCallStart { tool, id, input } => Some(json!({
            "type": "tool_call_start",
            "tool": tool,
            "id": id,
            "input": safe_value(input),
        })),
        AgentEvent::ToolResult {
            tool,
            id,
            input,
            output,
            error,
            images,
            auth_required,
        } => {
            let mut r = json!({
                "type": "tool_result",
                "tool": tool,
                "id": id,
                "input": safe_value(input),
                "output": output,
                "error": error,
            });
            if let Some(auth_id) = auth_required {
                if !auth_id.is_empty() {
                    r["auth_required"] = json!(auth_id);
                }
            }
            if !images.is_empty() {
                r["images"] = json!(images);
            }
            Some(r)
        }
        AgentEvent::Done {
            input_tokens,
            output_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
            context_window,
            model,
        } => Some(json!({
            "type": "done",
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "cache_creation_input_tokens": cache_creation_input_tokens,
                "cache_read_input_tokens": cache_read_input_tokens,
            },
            "context_window": context_window,
            "model": model,
        })),
        AgentEvent::Error(msg) => Some(json!({"type": "error", "message": msg})),
        AgentEvent::Aborted => Some(json!({"type": "aborted"})),
        AgentEvent::Trace {
            phase,
            dur_ms,
            at_ms,
            attrs,
        } => Some(json!({
            "type": "trace",
            "phase": phase,
            "dur_ms": dur_ms,
            "at_ms": at_ms,
            "attrs": attrs,
        })),
    }
}

/// Returns the input value if it is valid JSON, null otherwise.
/// Mirrors the Go `safeRawMessage` function.
pub fn safe_value(v: Value) -> Value {
    // If it's already a proper JSON value (not a raw unparsed string of garbage),
    // return it. Null is always safe.
    v
}

/// Validates that a JSON string value round-trips cleanly.
/// Used for raw JSON message safety (matches Go safeRawMessage).
pub fn safe_raw_message(raw: Option<&str>) -> Value {
    match raw {
        None => Value::Null,
        Some(s) => {
            let s = s.trim();
            if s.is_empty() {
                return Value::Null;
            }
            match serde_json::from_str::<Value>(s) {
                Ok(v) => v,
                Err(_) => Value::Null,
            }
        }
    }
}

// ─── chat.abort ──────────────────────────────────────────────────────────

async fn handle_chat_abort(
    req: JsonRpcRequest,
    conn_state: Arc<Mutex<ConnState>>,
    tx: mpsc::UnboundedSender<String>,
) {
    let mut cs = conn_state.lock();
    if let Some(cancel) = cs.active_cancel.take() {
        let _ = cancel.send(());
    }
    send_result(&tx, req.id, json!({"ok": true}));
}

// ─── agent.status ────────────────────────────────────────────────────────

async fn handle_agent_status(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    tx: mpsc::UnboundedSender<String>,
) {
    let cfg = state.config.read().clone();
    let agents = cfg.list_agents();

    let statuses: Vec<Value> = agents
        .into_iter()
        .map(|a| {
            json!({
                "id": a.id,
                "name": a.name,
                "model": a.model,
                "workspace": a.workspace,
                "context_window": a.context_window,
            })
        })
        .collect();

    send_result(&tx, req.id, json!({"agents": statuses}));
}

// ─── session.list ────────────────────────────────────────────────────────

async fn handle_session_list(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    conn_state: Arc<Mutex<ConnState>>,
    tx: mpsc::UnboundedSender<String>,
) {
    let agent_id = req
        .params
        .get("agentId")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    let agent_id = if agent_id.is_empty() {
        "default".to_string()
    } else {
        agent_id
    };

    let sessions = match state.session_store.list(&agent_id) {
        Ok(s) => s,
        Err(e) => {
            send_error(
                &tx,
                req.id,
                -32603,
                &format!("List sessions error: {}", e),
            );
            return;
        }
    };

    let active_key = {
        let cs = conn_state.lock();
        cs.session_keys
            .get(&agent_id)
            .cloned()
            .unwrap_or_else(|| "ws_default".to_string())
    };

    let result: Vec<Value> = sessions
        .into_iter()
        .map(|s| {
            json!({
                "key": s.key,
                "entryCount": s.entry_count,
                "createdAt": s.created_at,
                "lastActivity": s.last_activity,
                "active": s.key == active_key,
            })
        })
        .collect();

    send_result(&tx, req.id, json!({"sessions": result}));
}

// ─── session.new ─────────────────────────────────────────────────────────

async fn handle_session_new(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    conn_state: Arc<Mutex<ConnState>>,
    tx: mpsc::UnboundedSender<String>,
) {
    let agent_id = req
        .params
        .get("agentId")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    let agent_id = if agent_id.is_empty() {
        "default".to_string()
    } else {
        agent_id
    };

    let name = req
        .params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = if name.is_empty() {
        chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string()
    } else {
        name
    };

    let session_key = format!("ws_{}", name);

    if state.session_store.exists(&agent_id, &session_key) {
        send_error(
            &tx,
            req.id,
            -32602,
            &format!("Session already exists: {}", session_key),
        );
        return;
    }

    if let Err(e) = state.session_store.create(&agent_id, &session_key) {
        send_error(
            &tx,
            req.id,
            -32603,
            &format!("Create session error: {}", e),
        );
        return;
    }

    {
        let mut cs = conn_state.lock();
        cs.session_keys.insert(agent_id, session_key.clone());
    }

    send_result(&tx, req.id, json!({"sessionKey": session_key}));
}

// ─── session.switch ──────────────────────────────────────────────────────

async fn handle_session_switch(
    req: JsonRpcRequest,
    conn_state: Arc<Mutex<ConnState>>,
    tx: mpsc::UnboundedSender<String>,
) {
    let agent_id = req
        .params
        .get("agentId")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    let agent_id = if agent_id.is_empty() {
        "default".to_string()
    } else {
        agent_id
    };

    let session_key = match req
        .params
        .get("sessionKey")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        Some(k) => k.to_string(),
        None => {
            send_error(&tx, req.id, -32602, "Invalid params: sessionKey required");
            return;
        }
    };

    {
        let mut cs = conn_state.lock();
        cs.session_keys.insert(agent_id, session_key.clone());
    }

    send_result(&tx, req.id, json!({"sessionKey": session_key}));
}

// ─── session.history ─────────────────────────────────────────────────────

async fn handle_session_history(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    conn_state: Arc<Mutex<ConnState>>,
    tx: mpsc::UnboundedSender<String>,
) {
    let agent_id = req
        .params
        .get("agentId")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    let agent_id = if agent_id.is_empty() {
        "default".to_string()
    } else {
        agent_id
    };

    let session_key_param = req
        .params
        .get("sessionKey")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let session_key = resolve_session_key(&conn_state, &session_key_param, &agent_id);

    let history = match state.session_store.history(&agent_id, &session_key) {
        Ok(h) => h,
        Err(e) => {
            send_error(
                &tx,
                req.id,
                -32603,
                &format!("Session load error: {}", e),
            );
            return;
        }
    };

    let entries: Vec<Value> = history
        .into_iter()
        .filter_map(|entry| match entry {
            HistoryEntry::Message { role, text } => Some(json!({
                "type": "message",
                "role": role,
                "text": text,
            })),
            HistoryEntry::ToolCall { tool, id, input } => Some(json!({
                "type": "tool_call",
                "tool": tool,
                "id": id,
                "input": input,
            })),
            HistoryEntry::ToolResult {
                tool_call_id,
                output,
                error,
                images,
            } => {
                let mut e = json!({
                    "type": "tool_result",
                    "tool_call_id": tool_call_id,
                    "output": output,
                    "error": error,
                });
                if !images.is_empty() {
                    e["images"] = json!(images);
                }
                Some(e)
            }
        })
        .collect();

    send_result(&tx, req.id, json!({"entries": entries}));
}

// ─── session.clear ───────────────────────────────────────────────────────

async fn handle_session_clear(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    conn_state: Arc<Mutex<ConnState>>,
    tx: mpsc::UnboundedSender<String>,
) {
    let agent_id = req
        .params
        .get("agentId")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    let agent_id = if agent_id.is_empty() {
        "default".to_string()
    } else {
        agent_id
    };

    let session_key_param = req
        .params
        .get("sessionKey")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let session_key = resolve_session_key(&conn_state, &session_key_param, &agent_id);

    if let Err(e) = state.session_store.delete(&agent_id, &session_key) {
        send_error(&tx, req.id, -32603, &format!("Delete error: {}", e));
        return;
    }

    send_result(&tx, req.id, json!({"ok": true}));
}

// ─── jobs handlers ────────────────────────────────────────────────────────

fn get_job_scheduler(
    state: &Arc<WebSocketHandlerState>,
    tx: &mpsc::UnboundedSender<String>,
    id: &Value,
) -> Option<Arc<dyn JobSchedulerTrait>> {
    match &state.job_scheduler {
        Some(js) => Some(js.clone()),
        None => {
            send_error(tx, id.clone(), -32603, "Job scheduler not available");
            None
        }
    }
}

async fn handle_jobs_list(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    tx: mpsc::UnboundedSender<String>,
) {
    let js = match get_job_scheduler(&state, &tx, &req.id) {
        Some(j) => j,
        None => return,
    };
    let jobs = js.list_jobs();
    send_result(&tx, req.id, json!({"jobs": jobs}));
}

async fn handle_jobs_pause(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    tx: mpsc::UnboundedSender<String>,
) {
    let js = match get_job_scheduler(&state, &tx, &req.id) {
        Some(j) => j,
        None => return,
    };
    let name = match req.params.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            send_error(&tx, req.id, -32602, "Invalid params: name required");
            return;
        }
    };
    match js.pause_job(&name) {
        Ok(_) => send_result(&tx, req.id, json!({"ok": true})),
        Err(e) => send_error(&tx, req.id, -32603, &e.to_string()),
    }
}

async fn handle_jobs_resume(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    tx: mpsc::UnboundedSender<String>,
) {
    let js = match get_job_scheduler(&state, &tx, &req.id) {
        Some(j) => j,
        None => return,
    };
    let name = match req.params.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            send_error(&tx, req.id, -32602, "Invalid params: name required");
            return;
        }
    };
    match js.resume_job(&name) {
        Ok(_) => send_result(&tx, req.id, json!({"ok": true})),
        Err(e) => send_error(&tx, req.id, -32603, &e.to_string()),
    }
}

async fn handle_jobs_remove(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    tx: mpsc::UnboundedSender<String>,
) {
    let js = match get_job_scheduler(&state, &tx, &req.id) {
        Some(j) => j,
        None => return,
    };
    let name = match req.params.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            send_error(&tx, req.id, -32602, "Invalid params: name required");
            return;
        }
    };
    match js.remove_job(&name) {
        Ok(_) => send_result(&tx, req.id, json!({"ok": true})),
        Err(e) => send_error(&tx, req.id, -32603, &e.to_string()),
    }
}

async fn handle_jobs_add(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    tx: mpsc::UnboundedSender<String>,
) {
    let js = match get_job_scheduler(&state, &tx, &req.id) {
        Some(j) => j,
        None => return,
    };
    let name = req
        .params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let schedule = req
        .params
        .get("schedule")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let prompt = req
        .params
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if name.is_empty() || schedule.is_empty() || prompt.is_empty() {
        send_error(
            &tx,
            req.id,
            -32602,
            "name, schedule, and prompt are all required",
        );
        return;
    }

    match js.add_job(&name, &schedule, &prompt) {
        Ok(_) => send_result(&tx, req.id, json!({"ok": true, "name": name})),
        Err(e) => send_error(&tx, req.id, -32603, &e.to_string()),
    }
}

async fn handle_jobs_update(
    req: JsonRpcRequest,
    state: Arc<WebSocketHandlerState>,
    tx: mpsc::UnboundedSender<String>,
) {
    let js = match get_job_scheduler(&state, &tx, &req.id) {
        Some(j) => j,
        None => return,
    };
    let name = req
        .params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let schedule = req
        .params
        .get("schedule")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if name.is_empty() || schedule.is_empty() {
        send_error(
            &tx,
            req.id,
            -32602,
            "Invalid params: name and schedule required",
        );
        return;
    }

    match js.update_job_schedule(&name, &schedule) {
        Ok(_) => send_result(&tx, req.id, json!({"ok": true})),
        Err(e) => send_error(&tx, req.id, -32603, &e.to_string()),
    }
}

#[cfg(test)]
#[path = "websocket_test.rs"]
mod websocket_test;