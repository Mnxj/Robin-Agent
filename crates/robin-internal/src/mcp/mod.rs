/// MCP (Model Context Protocol) client integration for Robin.
///
/// Provides:
/// - HTTP-based MCP client (Streamable-HTTP transport)
/// - stdio-based MCP client (spawns subprocess, JSON-RPC over stdin/stdout)
/// - OAuth2 authentication (client credentials flow, authorization code + PKCE)
/// - Bearer token authentication
/// - Token store (persist tokens to disk)
/// - Manager that registers/manages multiple MCP server connections
/// - Adapter that converts MCP tools to the internal Tool interface

pub mod types;
pub mod client;
pub mod stdio;
pub mod bearer;
pub mod oauth;
pub mod authcode;
pub mod creds;
pub mod adapter;
pub mod manager;
pub mod register;

// Test modules (compiled only in test builds).
#[cfg(test)]
#[path = "adapter_test.rs"]
mod adapter_test;

#[cfg(test)]
#[path = "authcode_test.rs"]
mod authcode_test;

#[cfg(test)]
#[path = "bearer_test.rs"]
mod bearer_test;

#[cfg(test)]
#[path = "client_test.rs"]
mod client_test;

#[cfg(test)]
#[path = "creds_test.rs"]
mod creds_test;

#[cfg(test)]
#[path = "manager_test.rs"]
mod manager_test;

#[cfg(test)]
#[path = "oauth_test.rs"]
mod oauth_test;

#[cfg(test)]
#[path = "reauth_test.rs"]
mod reauth_test;

#[cfg(test)]
#[path = "register_test.rs"]
mod register_test;

#[cfg(test)]
#[path = "stdio_test.rs"]
mod stdio_test;

// Re-exports for convenience.
pub use types::{ManagerServerConfig, HttpServerConfig, HttpAuthConfig, StdioServerConfig};
pub use client::{Client, ToolInfo, CallResult, connect_http};
pub use stdio::connect_stdio;
pub use bearer::new_bearer_http_client;
pub use oauth::{ClientCredentialsConfig, new_client_credentials_http_client};
pub use authcode::{AuthCodePKCEConfig, ErrInteractiveLoginRequired, run_interactive_login,
                   new_auth_code_pkce_http_client};
pub use creds::{OAuthTokenStore, load_token, save_token, load_env_file, require_keys};
pub use adapter::{McpToolAdapter, ParallelSafeFn, is_auth_failure};
pub use manager::{Manager, ServerEntry, MAX_CONSECUTIVE_AUTH_FAILURES};
pub use register::register_tools;