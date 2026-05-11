use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use super::client::{connect_http, Client};
use super::stdio::connect_stdio;
use super::types::{HttpAuthConfig, ManagerServerConfig};

/// MAX_CONSECUTIVE_AUTH_FAILURES is the per-server circuit-breaker threshold.
/// After this many consecutive tool calls fail with auth-shaped errors that
/// even an automatic Reconnect+retry couldn't fix, the adapter short-circuits
/// subsequent calls without touching the network.
///
/// The breaker resets on (a) a successful tool call (record_success) and
/// (b) a user-initiated reconnect via Manager::reconnect_server.
pub const MAX_CONSECUTIVE_AUTH_FAILURES: usize = 3;

/// ServerEntry is a connected MCP server known to the Manager. The live
/// Client is held under an RwLock so it can be swapped atomically by
/// reconnect without invalidating any adapter that's currently mid-call.
/// Adapters hold Arc<ServerEntry> (not Arc<Client>) and call entry.live()
/// per call so the next call after a successful reconnect picks up the new client.
pub struct ServerEntry {
    pub id: String,
    pub tool_prefix: String,
    /// Mirrors ManagerServerConfig.parallel_safe at construction time.
    pub parallel_safe: bool,

    pub(crate) client: RwLock<Option<Arc<Client>>>,
    /// Retained for reconnect to re-run connect_one.
    pub(crate) cfg: ManagerServerConfig,

    pub(crate) consecutive_failures: Mutex<usize>,
}

impl ServerEntry {
    fn new(cfg: ManagerServerConfig, client: Client) -> Self {
        Self {
            id: cfg.id.clone(),
            tool_prefix: cfg.tool_prefix.clone(),
            parallel_safe: cfg.parallel_safe,
            client: RwLock::new(Some(Arc::new(client))),
            cfg,
            consecutive_failures: Mutex::new(0),
        }
    }

    /// record_success clears the consecutive-failure counter. Called by the
    /// adapter after a successful tool call.
    pub fn record_success(&self) {
        *self.consecutive_failures.lock() = 0;
    }

    /// record_failure increments the consecutive-failure counter and returns
    /// the new value.
    pub fn record_failure(&self) -> usize {
        let mut guard = self.consecutive_failures.lock();
        *guard += 1;
        *guard
    }

    /// failure_count returns the current consecutive-failure count.
    pub fn failure_count(&self) -> usize {
        *self.consecutive_failures.lock()
    }

    /// reset_failures clears the breaker. Called by Manager::reconnect_server
    /// (the user-initiated Re-authenticate path).
    pub(crate) fn reset_failures(&self) {
        *self.consecutive_failures.lock() = 0;
    }

    /// live returns the current Client. Adapters call this on every tool
    /// invocation so a reconnect-driven swap is observed on the next call.
    pub fn live(&self) -> Option<Arc<Client>> {
        self.client.read().clone()
    }

    /// reconnect closes the existing client and opens a new one against the
    /// same config. Any in-flight tool call that already grabbed the old
    /// client via live() finishes against the old session; subsequent calls
    /// observe the new one.
    pub async fn reconnect(&self) -> anyhow::Result<()> {
        let new_client = connect_one(&self.cfg).await?;
        let old = {
            let mut guard = self.client.write();
            let old = guard.take();
            *guard = Some(Arc::new(new_client));
            old
        };
        // Close old client in background (fire-and-forget).
        if let Some(old_client) = old {
            let id = self.id.clone();
            tokio::spawn(async move {
                if let Err(e) = old_client.close() {
                    tracing::debug!("mcp: close old client after reconnect id={} error={}", id, e);
                }
            });
        }
        Ok(())
    }

    /// get_client is the legacy accessor. Kept for external callers that still
    /// expect it; reads through live().
    pub fn get_client(&self) -> Option<Arc<Client>> {
        self.live()
    }
}

/// Manager owns a Client per enabled MCP server. Servers that fail to connect
/// at startup are logged and skipped — Manager construction still succeeds so
/// the rest of the gateway can start.
pub struct Manager {
    pub(crate) servers: Vec<Arc<ServerEntry>>,
}

impl Manager {
    /// new_manager opens a session against each ManagerServerConfig in cfgs.
    /// Dispatches on cfg.transport: "http" uses connect_http with an
    /// auth-aware client, "stdio" uses connect_stdio with a spawned subprocess.
    /// An unknown transport (or a per-server connect failure) is logged and
    /// the entry skipped — Manager construction never fails.
    pub async fn new(cfgs: Vec<ManagerServerConfig>) -> anyhow::Result<Self> {
        let mut servers = Vec::new();
        for cfg in cfgs {
            match connect_one(&cfg).await {
                Err(e) => {
                    tracing::warn!(
                        "mcp: failed to connect to server, skipping id={} transport={} error={}",
                        cfg.id, cfg.transport, e
                    );
                }
                Ok(client) => {
                    let id = cfg.id.clone();
                    let transport = cfg.transport.clone();
                    servers.push(Arc::new(ServerEntry::new(cfg, client)));
                    tracing::info!("mcp: connected to server id={} transport={}", id, transport);
                }
            }
        }
        Ok(Self { servers })
    }

    /// reconnect_server finds the entry with the given id and runs reconnect
    /// on it. Returns an error if the server isn't known. Also resets the
    /// per-server consecutive-failure breaker.
    pub async fn reconnect_server(&self, id: &str) -> anyhow::Result<()> {
        for s in &self.servers {
            if s.id == id {
                s.reconnect().await?;
                s.reset_failures();
                return Ok(());
            }
        }
        Err(anyhow::anyhow!("mcp: server {:?} not found", id))
    }

    /// servers returns the connected server entries.
    pub fn servers(&self) -> &[Arc<ServerEntry>] {
        &self.servers
    }

    /// close terminates every server session. Errors are aggregated into a
    /// single returned error but close always attempts every server.
    pub fn close(&self) -> anyhow::Result<()> {
        let mut errors: Vec<String> = Vec::new();
        for s in &self.servers {
            if let Some(client) = s.live() {
                if let Err(e) = client.close() {
                    errors.push(format!("close {}: {}", s.id, e));
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow::anyhow!("{}", errors.join("\n")))
        }
    }
}

/// connect_one dispatches on cfg.transport and returns a connected Client.
pub(crate) async fn connect_one(cfg: &ManagerServerConfig) -> anyhow::Result<Client> {
    match cfg.transport.as_str() {
        "http" | "" => {
            let http_cfg = cfg.http.as_ref()
                .ok_or_else(|| anyhow::anyhow!("http transport requires HTTP block"))?;
            let http_client = build_http_client(&http_cfg.auth).await
                .map_err(|e| anyhow::anyhow!("build http client: {}", e))?;
            connect_http(&http_cfg.url, http_client).await
        }
        "stdio" => {
            let stdio_cfg = cfg.stdio.as_ref()
                .ok_or_else(|| anyhow::anyhow!("stdio transport requires Stdio block"))?;
            connect_stdio(
                &cfg.id,
                &stdio_cfg.command,
                &stdio_cfg.args,
                &stdio_cfg.env,
            )
        }
        other => Err(anyhow::anyhow!("unknown transport {:?}", other)),
    }
}

/// build_http_client constructs a reqwest::Client with the appropriate
/// authentication transport for the given auth config.
pub(crate) async fn build_http_client(auth: &HttpAuthConfig) -> anyhow::Result<reqwest::Client> {
    match auth.kind.as_str() {
        "oauth2_client_credentials" => {
            use super::oauth::{ClientCredentialsConfig, new_client_credentials_http_client};
            new_client_credentials_http_client(ClientCredentialsConfig {
                token_url: auth.token_url.clone(),
                client_id: auth.client_id.clone(),
                client_secret: auth.client_secret.clone(),
                scope: auth.scope.clone(),
            }).await
        }
        "oauth2_authorization_code" => {
            use super::authcode::{AuthCodePKCEConfig, new_auth_code_pkce_http_client, ErrInteractiveLoginRequired};
            let result = new_auth_code_pkce_http_client(AuthCodePKCEConfig {
                auth_url: auth.auth_url.clone(),
                token_url: auth.token_url.clone(),
                client_id: auth.client_id.clone(),
                client_secret: auth.client_secret.clone(),
                scope: auth.scope.clone(),
                redirect_uri: auth.redirect_uri.clone(),
                store_path: auth.token_store_path.clone(),
            }).await;
            match result {
                Err(e) if e.downcast_ref::<ErrInteractiveLoginRequired>().is_some() => {
                    Err(anyhow::anyhow!(
                        "no cached token at {} — run `robin mcp login <id>` first",
                        auth.token_store_path
                    ))
                }
                other => other,
            }
        }
        "bearer" => {
            Ok(super::bearer::new_bearer_http_client(auth.bearer_token.clone()))
        }
        "none" | "" => {
            Ok(reqwest::Client::new())
        }
        other => Err(anyhow::anyhow!("unsupported http auth kind {:?}", other)),
    }
}