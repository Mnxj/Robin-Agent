use axum::{
    body::Body,
    http::{header, StatusCode},
    response::Response,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

use crate::gateway::{
    auth::bearer_auth_middleware,
    chat::chat_handler,
    logs::{logs_stream_handler, LogBuffer},
    memory::{delete_memory, get_memory, list_memory, save_memory, MemoryHandlerState},
    mcp::{reauth_handler, McpHandlerState},
    metrics::Metrics,
    settings::{
        bootstrap_status_handler, get_config_handler, list_tools_handler, save_config_handler,
        settings_page_handler, SettingsHandlerState,
    },
    skills::{delete_skill, get_skill, list_skills, upload_skill, SkillHandlerState},
    websocket::{ws_handler, WebSocketHandlerState},
};

/// Options for the gateway server.
pub struct ServerOptions {
    /// Bearer token for API authentication (empty = no auth).
    pub auth_token: String,
    /// WebSocket allowed origins (empty = localhost only).
    pub allowed_origins: Vec<String>,
    /// Optional Prometheus metrics state.
    pub metrics: Option<Arc<Metrics>>,
    /// Optional settings handler state.
    pub settings: Option<SettingsHandlerState>,
    /// Optional skills handler state.
    pub skills: Option<SkillHandlerState>,
    /// Optional memory handler state.
    pub memory: Option<MemoryHandlerState>,
    /// Optional MCP handler state.
    pub mcp: Option<McpHandlerState>,
    /// Optional log buffer for /logs and /logs/stream endpoints.
    /// LogBuffer is cheaply clonable (Arc inside).
    pub log_buffer: Option<LogBuffer>,
    /// Port passed to the chat UI so it can connect to the right WebSocket.
    pub chat_port: Option<u16>,
}

impl Default for ServerOptions {
    fn default() -> Self {
        ServerOptions {
            auth_token: String::new(),
            allowed_origins: vec![],
            metrics: None,
            settings: None,
            skills: None,
            memory: None,
            mcp: None,
            log_buffer: None,
            chat_port: None,
        }
    }
}

/// The Robin gateway HTTP + WebSocket server.
pub struct Server {
    router: Router,
    host: String,
    port: u16,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
}

impl Server {
    /// Creates a new gateway server.
    ///
    /// `ws_state` is the shared state for the WebSocket handler.
    pub fn new(
        host: String,
        port: u16,
        ws_state: Arc<WebSocketHandlerState>,
        opts: ServerOptions,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let mut router = build_routes(&opts, ws_state);

        // Add bearer auth middleware if configured.
        if !opts.auth_token.is_empty() {
            let token = opts.auth_token.clone();
            router = router.layer(axum::middleware::from_fn(move |req, next| {
                let token = token.clone();
                async move { bearer_auth_middleware(token)(req, next).await }
            }));
        }

        Server {
            router,
            host,
            port,
            shutdown_tx,
            shutdown_rx,
        }
    }

    /// Binds to the configured address and starts serving.
    ///
    /// Blocks until the server is shut down or encounters an error.
    pub async fn start(&mut self) -> anyhow::Result<()> {
        let addr = format!("{}:{}", self.host, self.port);
        let listener = TcpListener::bind(&addr).await?;
        info!("gateway listening on {}", addr);

        let router = self.router.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
            })
            .await?;

        Ok(())
    }

    /// Shuts down the server gracefully.
    ///
    /// Safe to call before `start` — in that case it is a no-op.
    pub fn shutdown(&self) {
        // Ignore send errors: receiver may already be dropped.
        let _ = self.shutdown_tx.send(true);
    }

    /// Returns a cloneable handle that can trigger shutdown from another task.
    pub fn shutdown_handle(&self) -> tokio::sync::watch::Sender<bool> {
        self.shutdown_tx.clone()
    }

    /// Returns a reference to the inner router (useful for testing).
    pub fn router(&self) -> &Router {
        &self.router
    }
}

// ─── Route builder ────────────────────────────────────────────────────────

fn build_routes(opts: &ServerOptions, ws_state: Arc<WebSocketHandlerState>) -> Router {
    let mut router = Router::new();

    // Health endpoint
    router = router.route("/health", get(health_handler));

    // Chat UI — redirect / → /chat
    if let Some(port) = opts.chat_port {
        router = router
            .route("/chat", get(move || chat_handler(port)))
            .route("/", get(move || async move {
                axum::response::Redirect::to("/chat")
            }));
    }

    // WebSocket endpoint
    router = router.route("/ws", get(ws_handler).with_state(ws_state));

    // Metrics
    if let Some(metrics) = &opts.metrics {
        let m = metrics.clone();
        router = router.route("/metrics", get(m.handler()));
    }

    // Settings pages and API
    if let Some(settings_state) = &opts.settings {
        let s = settings_state.clone();
        router = router
            .route("/settings", get(settings_page_handler))
            .route("/settings/", get(settings_page_handler))
            .route("/ui", get(settings_page_handler))
            .route("/jobs", get(settings_page_handler))
            .route(
                "/settings/api/config",
                get(get_config_handler)
                    .post(save_config_handler)
                    .with_state(s.clone()),
            )
            .route(
                "/settings/api/tools",
                get(list_tools_handler).with_state(s.clone()),
            )
            .route(
                "/settings/api/bootstrap",
                get(bootstrap_status_handler).with_state(s),
            );
    }

    // Skills API
    if let Some(skills_state) = &opts.skills {
        let s = skills_state.clone();
        router = router
            .route(
                "/settings/api/skills",
                get(list_skills).post(upload_skill).with_state(s.clone()),
            )
            .route(
                "/settings/api/skills/:name",
                get(get_skill).delete(delete_skill).with_state(s),
            );
    }

    // Memory API
    if let Some(memory_state) = &opts.memory {
        let m = memory_state.clone();
        router = router
            .route(
                "/settings/api/memory",
                get(list_memory).post(save_memory).with_state(m.clone()),
            )
            .route(
                "/settings/api/memory/:id",
                get(get_memory).delete(delete_memory).with_state(m),
            );
    }

    // MCP re-auth API
    if let Some(mcp_state) = &opts.mcp {
        let m = mcp_state.clone();
        router = router.route("/api/mcp/reauth/:id", post(reauth_handler).with_state(m));
    }

    // Logs endpoints
    if let Some(log_buf) = &opts.log_buffer {
        let buf = log_buf.clone();
        router = router
            .route("/logs", get(settings_page_handler))
            .route("/logs/stream", get(logs_stream_handler).with_state(buf));
    }

    router
}

// ─── Health handler ────────────────────────────────────────────────────────

async fn health_handler() -> Response {
    let ts = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let body = format!(r#"{{"status":"ok","timestamp":"{}"}}"#, ts);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

#[cfg(test)]
#[path = "server_test.rs"]
mod server_test;
