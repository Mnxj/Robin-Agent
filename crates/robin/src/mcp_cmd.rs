use std::io::Write;

use robin_internal::config::config::{default_config_path, load};
use robin_internal::mcp::authcode::{run_interactive_login, AuthCodePKCEConfig};

pub fn mcp_cmd() -> clap::Command {
    clap::Command::new("mcp")
        .about("Manage MCP server connections")
        .subcommand(mcp_login_cmd())
}

fn mcp_login_cmd() -> clap::Command {
    clap::Command::new("login")
        .about("Run interactive OAuth login for an MCP server (auth-code + PKCE)")
        .long_about(
            "login performs the OAuth 2.0 Authorization Code + PKCE flow against\n\
             the configured MCP server's IdP. Robin opens your browser, listens on\n\
             the redirect URI's loopback port for the callback, exchanges the code\n\
             for a token, and writes the token to <data-dir>/mcp-tokens/<server-id>.json.",
        )
        .arg(clap::Arg::new("server-id").required(true).help("MCP server ID to authenticate"))
        .arg(
            clap::Arg::new("config")
                .short('c').long("config").value_name("FILE")
                .help("path to config file (default ~/.robin/robin.json5)"),
        )
}

pub fn handle_mcp(matches: &clap::ArgMatches) -> anyhow::Result<()> {
    match matches.subcommand() {
        Some(("login", sub)) => {
            let config_path = sub.get_one::<String>("config").map(|s| s.as_str()).unwrap_or("");
            let server_id = sub.get_one::<String>("server-id").expect("required");
            run_mcp_login(config_path, server_id, &mut std::io::stdout())
        }
        _ => { eprintln!("Usage: robin mcp <subcommand>"); Ok(()) }
    }
}

pub fn run_mcp_login(config_path: &str, server_id: &str, out: &mut impl Write) -> anyhow::Result<()> {
    let resolved_path = if config_path.is_empty() {
        default_config_path()
    } else {
        config_path.to_string()
    };

    let cfg = load(&resolved_path)
        .map_err(|e| anyhow::anyhow!("load config {}: {}", resolved_path, e))?;

    let resolved = cfg.resolve_mcp_servers()
        .map_err(|e| anyhow::anyhow!("resolve mcp servers: {}", e))?;

    let entry = resolved.iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| anyhow::anyhow!("mcp server {:?} not found in {}", server_id, resolved_path))?;

    if entry.transport != "http" || entry.http.is_none() {
        return Err(anyhow::anyhow!("mcp server {:?} is not an HTTP server", server_id));
    }

    let http = entry.http.as_ref().unwrap();
    let auth = &http.auth;

    if auth.kind != "oauth2_authorization_code" {
        return Err(anyhow::anyhow!(
            "mcp server {:?} uses auth.kind={:?}; login is only meaningful for oauth2_authorization_code",
            server_id,
            auth.kind,
        ));
    }

    let pkce_cfg = AuthCodePKCEConfig {
        auth_url: auth.auth_url.clone(),
        token_url: auth.token_url.clone(),
        client_id: auth.client_id.clone(),
        client_secret: auth.client_secret.clone(),
        scope: auth.scope.clone(),
        redirect_uri: auth.redirect_uri.clone(),
        store_path: auth.token_store_path.clone(),
    };

    let tok = tokio::runtime::Handle::current()
        .block_on(run_interactive_login(pkce_cfg))
        .map_err(|e| anyhow::anyhow!("login: {}", e))?;

    writeln!(out, "Logged in to {}.\nToken cached at {}.", server_id, auth.token_store_path)?;

    if tok.refresh_token.is_empty() {
        writeln!(out, "WARNING: IdP did not return a refresh token...")?;
    }

    if let Some(expiry) = tok.expiry {
        writeln!(out, "Access token expires at {}.", expiry.format("%Y-%m-%d %H:%M:%S UTC"))?;
    }

    writeln!(out, "Restart `robin start` to pick up the new token.")?;
    Ok(())
}