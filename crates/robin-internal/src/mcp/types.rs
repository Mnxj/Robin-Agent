/// ManagerServerConfig is the resolved-secret, transport-discriminated shape
/// Manager consumes. The caller (typically Config::resolve_mcp_servers) picks
/// the transport, resolves secrets from their config-or-env source, and
/// populates the matching transport-specific block (HTTP or Stdio).
#[derive(Debug, Clone)]
pub struct ManagerServerConfig {
    pub id: String,
    pub tool_prefix: String,
    /// "http" | "stdio"
    pub transport: String,
    /// populated when transport == "http"
    pub http: Option<HttpServerConfig>,
    /// populated when transport == "stdio"
    pub stdio: Option<StdioServerConfig>,
    /// ParallelSafe is vestigial here as of the live-read refactor.
    /// IsConcurrencySafe now reads the live config via the parallel_safe_fn
    /// closure passed to register_tools. Still set by resolve_mcp_servers and
    /// mirrored into ServerEntry for API stability.
    pub parallel_safe: bool,
}

/// HttpServerConfig describes an HTTP-transport MCP server, including which
/// auth scheme to use against it.
#[derive(Debug, Clone)]
pub struct HttpServerConfig {
    pub url: String,
    pub auth: HttpAuthConfig,
}

/// HttpAuthConfig discriminates on kind. Only the fields relevant to the
/// chosen kind need be populated; Manager dispatches on kind to build the
/// right reqwest::Client.
#[derive(Debug, Clone, Default)]
pub struct HttpAuthConfig {
    /// "oauth2_client_credentials" | "oauth2_authorization_code" | "bearer" | "none"
    pub kind: String,

    // oauth2_client_credentials, oauth2_authorization_code
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub scope: String,

    // oauth2_authorization_code
    /// IdP authorize endpoint
    pub auth_url: String,
    /// must be loopback per RFC 8252
    pub redirect_uri: String,
    /// absolute path to per-server token cache file
    pub token_store_path: String,

    // bearer
    pub bearer_token: String,
}

/// StdioServerConfig describes a stdio-transport MCP server. The configured
/// env map is merged onto std::env::vars() at spawn time so the child inherits
/// PATH and other parent env vars unless explicitly overridden.
#[derive(Debug, Clone)]
pub struct StdioServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
}
