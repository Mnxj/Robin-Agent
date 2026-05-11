use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
};
use parking_lot::Mutex;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::Arc,
    time::Duration,
};
use tracing::{info, warn};

/// Trait for MCP manager operations needed by the handler.
pub trait McpManagerTrait: Send + Sync {
    fn reconnect_server(
        &self,
        id: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>;
}

/// Configuration provider trait.
pub trait ConfigProvider: Send + Sync {
    fn resolved_mcp_servers(&self) -> anyhow::Result<Vec<McpServerConfig>>;
}

/// MCP server configuration needed for re-auth.
pub struct McpServerConfig {
    pub id: String,
    pub transport: String,
    pub http: Option<McpHttpConfig>,
}

/// HTTP transport configuration.
pub struct McpHttpConfig {
    pub auth: McpAuthConfig,
}

/// Authentication configuration.
pub struct McpAuthConfig {
    pub kind: String,
    pub auth_url: String,
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub scope: String,
    pub redirect_uri: String,
    pub token_store_path: String,
}

/// State for the MCP handlers.
#[derive(Clone)]
pub struct McpHandlerState {
    pub manager: Option<Arc<dyn McpManagerTrait>>,
    pub config: Option<Arc<dyn ConfigProvider>>,
    pub in_flight: Arc<Mutex<HashMap<String, bool>>>,
}

impl McpHandlerState {
    pub fn new(
        manager: Option<Arc<dyn McpManagerTrait>>,
        config: Option<Arc<dyn ConfigProvider>>,
    ) -> Self {
        McpHandlerState {
            manager,
            config,
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    let body = json!({"ok": false, "error": msg});
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn json_ok(status: StatusCode, body: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

/// Tries to claim an in-flight slot for the given server id.
/// Returns true if successful (not already in flight).
fn try_claim(in_flight: &Mutex<HashMap<String, bool>>, id: &str) -> bool {
    let mut map = in_flight.lock();
    if *map.get(id).unwrap_or(&false) {
        return false;
    }
    map.insert(id.to_string(), true);
    true
}

fn release(in_flight: &Mutex<HashMap<String, bool>>, id: &str) {
    let mut map = in_flight.lock();
    map.remove(id);
}

/// Handler: POST /api/mcp/reauth/:id
///
/// Runs the OAuth Authorization Code + PKCE flow for the named server,
/// persists the new token, and reconnects the MCP client in-process.
pub async fn reauth_handler(
    State(state): State<McpHandlerState>,
    Path(id): Path<String>,
) -> Response {
    if state.manager.is_none() || state.config.is_none() {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "MCP manager not configured",
        );
    }

    let manager = state.manager.as_ref().unwrap().clone();
    let config_provider = state.config.as_ref().unwrap().clone();

    if id.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "missing server id in URL");
    }

    if !try_claim(&state.in_flight, &id) {
        return json_error(
            StatusCode::CONFLICT,
            "another re-authentication is already running for this server",
        );
    }

    // Resolve the MCP server config
    let resolved = match config_provider.resolved_mcp_servers() {
        Ok(r) => r,
        Err(e) => {
            release(&state.in_flight, &id);
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("resolve mcp servers: {}", e),
            );
        }
    };

    let entry = resolved.into_iter().find(|s| s.id == id);
    let entry = match entry {
        Some(e) => e,
        None => {
            release(&state.in_flight, &id);
            return json_error(
                StatusCode::BAD_REQUEST,
                &format!(
                    "mcp server {:?} not found, disabled, or its secret env var isn't set",
                    id
                ),
            );
        }
    };

    if entry.transport != "http" || entry.http.is_none() {
        release(&state.in_flight, &id);
        return json_error(
            StatusCode::BAD_REQUEST,
            &format!(
                "mcp server {:?} is not an HTTP server (transport={:?})",
                id, entry.transport
            ),
        );
    }

    let http = entry.http.unwrap();
    let auth = &http.auth;

    if auth.kind != "oauth2_authorization_code" {
        release(&state.in_flight, &id);
        return json_error(
            StatusCode::BAD_REQUEST,
            &format!(
                "mcp server {:?} uses auth.kind={:?}; re-auth only applies to oauth2_authorization_code",
                id, auth.kind
            ),
        );
    }

    // Run the PKCE login flow (5-minute timeout to allow user interaction)
    // These fields would be passed to run_interactive_login() once implemented.
    let _auth_url = auth.auth_url.clone();
    let _token_url = auth.token_url.clone();
    let _client_id = auth.client_id.clone();
    let _client_secret = auth.client_secret.clone();
    let _scope = auth.scope.clone();
    let _redirect_uri = auth.redirect_uri.clone();
    let _store_path = auth.token_store_path.clone();
    let id_for_log = id.clone();

    // NOTE: In a full implementation, this would call into the MCP login flow.
    // Here we stub the PKCE interactive login and reconnect steps.
    // The actual implementation would use tokio::time::timeout with 5 minutes.
    let login_result: anyhow::Result<String> = Err(anyhow::anyhow!(
        "interactive login not implemented in Rust translation; implement mcp::run_interactive_login"
    ));

    let expiry = match login_result {
        Ok(exp) => exp,
        Err(e) => {
            warn!("mcp reauth: interactive login failed, id={}, error={}", id_for_log, e);
            release(&state.in_flight, &id);
            return json_error(
                StatusCode::BAD_REQUEST,
                &format!("login failed: {}", e),
            );
        }
    };

    info!("mcp reauth: token refreshed, id={}, expiry={}", id_for_log, expiry);

    // Reconnect in-process (30-second timeout)
    let reconnect_result = tokio::time::timeout(
        Duration::from_secs(30),
        manager.reconnect_server(&id),
    )
    .await;

    release(&state.in_flight, &id);

    match reconnect_result {
        Ok(Ok(())) => {
            info!("mcp reauth: reconnected, id={}", id_for_log);
            json_ok(StatusCode::OK, json!({"ok": true, "expiry": expiry}))
        }
        Ok(Err(e)) => {
            warn!(
                "mcp reauth: reconnect failed after successful login, id={}, error={}",
                id_for_log, e
            );
            json_ok(
                StatusCode::OK,
                json!({
                    "ok": true,
                    "expiry": expiry,
                    "warning": format!(
                        "token refreshed but in-process reconnect failed: {}. Restart Robin to pick up the new token.",
                        e
                    )
                }),
            )
        }
        Err(_timeout) => {
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "reconnect timed out after 30 seconds",
            )
        }
    }
}