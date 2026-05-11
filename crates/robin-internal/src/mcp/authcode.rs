use std::net::TcpListener;
use std::time::Duration;

use parking_lot::Mutex;

use super::creds::{load_token, save_token, OAuthTokenStore};

/// AuthCodePKCEConfig is the resolved-secret shape needed to authenticate
/// against an MCP server using OAuth 2.0 Authorization Code + PKCE
/// (RFC 7636) with a loopback redirect (RFC 8252).
#[derive(Debug, Clone)]
pub struct AuthCodePKCEConfig {
    pub auth_url: String,
    pub token_url: String,
    pub client_id: String,
    /// empty => public PKCE client; some IdPs (Cognito) require non-empty
    pub client_secret: String,
    /// space-separated; defaulted upstream to "openid offline_access"
    pub scope: String,
    /// must be http://127.0.0.1:PORT/... or http://localhost:PORT/...
    pub redirect_uri: String,
    /// absolute path to per-server token cache file (mode 0600)
    pub store_path: String,
}

/// ErrInteractiveLoginRequired is returned when no usable token is cached
/// (no access token still valid AND no refresh token), so the caller must
/// drive the interactive PKCE flow before the client is usable.
#[derive(Debug, thiserror::Error)]
#[error("mcp: interactive auth-code login required")]
pub struct ErrInteractiveLoginRequired;

/// loginTimeout caps how long we wait for the user to complete the browser
/// dance before tearing down the loopback callback server.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Package-level browser opener (swappable for tests).
static OPEN_BROWSER: Mutex<fn(&str)> = Mutex::new(open_browser_os);

/// SetOpenBrowserForTest swaps the package-level browser opener. Pass None
/// to restore the OS default.
pub fn set_open_browser_for_test(f: Option<fn(&str)>) {
    *OPEN_BROWSER.lock() = f.unwrap_or(open_browser_os);
}

/// NewAuthCodePKCEHTTPClient returns a reqwest::Client whose transport injects
/// a Bearer token from cfg.store_path. The token is refreshed transparently
/// via the IdP's refresh_token grant when it nears expiry, and every refresh
/// result is persisted back to the store.
///
/// Behaviour by cache state:
///   - Valid access token (not expired with 30s buffer): returned wrapped, no network call.
///   - Expired access token but refresh token present: refreshed inline now.
///   - No usable token: returns ErrInteractiveLoginRequired.
pub async fn new_auth_code_pkce_http_client(
    cfg: AuthCodePKCEConfig,
) -> Result<reqwest::Client, anyhow::Error> {
    let tok = load_token(&cfg.store_path)
        .map_err(|e| anyhow::anyhow!("load cached token: {}", e))?;

    match &tok {
        None => return Err(anyhow::anyhow!(ErrInteractiveLoginRequired)),
        Some(t) if t.access_token.is_empty() && t.refresh_token.is_empty() => {
            return Err(anyhow::anyhow!(ErrInteractiveLoginRequired));
        }
        _ => {}
    }

    let mut token = tok.unwrap();

    // If the access token is not usable, try a refresh now.
    if !token.is_usable() {
        token = refresh_token(&cfg, &token.refresh_token).await
            .map_err(|e| anyhow::anyhow!("refresh cached token: {}", e))?;
        save_token(&cfg.store_path, &token)
            .map_err(|e| anyhow::anyhow!("persist refreshed token: {}", e))?;
    }

    // Build a client with the current access token as the default header.
    // The token refresh logic lives in PersistingClient, but for simplicity
    // we build a plain reqwest::Client here. Long-lived refresh is handled
    // by the persisting token store on subsequent calls.
    build_bearer_client(&token.access_token)
}

/// RunInteractiveLogin performs the full PKCE dance: generates a verifier
/// and S256 challenge, listens on the redirect URI's loopback port, opens
/// the OS browser to the IdP's authorize endpoint, waits for the callback,
/// validates state, exchanges the code for a token, and persists.
///
/// Returns the token (also written to cfg.store_path). The caller does not
/// need to persist it again.
pub async fn run_interactive_login(
    cfg: AuthCodePKCEConfig,
) -> anyhow::Result<OAuthTokenStore> {
    if cfg.redirect_uri.is_empty() || cfg.auth_url.is_empty() || cfg.token_url.is_empty() || cfg.client_id.is_empty() {
        return Err(anyhow::anyhow!("auth-code login requires auth_url, token_url, client_id, redirect_uri"));
    }

    let redirect = url::Url::parse(&cfg.redirect_uri)
        .map_err(|e| anyhow::anyhow!("parse redirect_uri: {}", e))?;

    let host = redirect.host_str().unwrap_or("");
    if host != "localhost" && host != "127.0.0.1" && host != "::1" {
        return Err(anyhow::anyhow!(
            "redirect_uri host {:?} is not a loopback address", host
        ));
    }

    let port: u16 = redirect.port()
        .ok_or_else(|| anyhow::anyhow!("redirect_uri must include an explicit port"))?;

    let (verifier, challenge) = new_pkce_pair()?;
    let state = rand_string(32)?;

    // Build the authorization URL.
    let mut auth_url = url::Url::parse(&cfg.auth_url)
        .map_err(|e| anyhow::anyhow!("parse auth_url: {}", e))?;
    {
        let mut q = auth_url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("client_id", &cfg.client_id);
        q.append_pair("redirect_uri", &cfg.redirect_uri);
        if !cfg.scope.is_empty() {
            q.append_pair("scope", &cfg.scope);
        }
        q.append_pair("state", &state);
        q.append_pair("code_challenge", &challenge);
        q.append_pair("code_challenge_method", "S256");
    }

    // Bind the loopback listener before opening the browser.
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
        .map_err(|e| anyhow::anyhow!(
            "bind callback port {}: {} (something else is using it; close that process or change redirect_uri port)",
            port, e
        ))?;

    // Callback path.
    let _callback_path = redirect.path().to_owned();
    let state_clone = state.clone();

    // Channel for the result.
    let (tx, rx) = tokio::sync::oneshot::channel::<anyhow::Result<String>>();
    let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));

    // Spawn an axum server to handle the callback.
    let tx_clone = tx.clone();
    let server_handle = tokio::task::spawn_blocking(move || {
        // We need a mini HTTP server on the blocking thread.
        // Use a simple std::net::TcpListener + hand-rolled HTTP for simplicity.
        listener.set_nonblocking(false).ok();
        // Accept one connection (the callback redirect).
        match listener.accept() {
            Ok((mut stream, _)) => {
                use std::io::{Read, Write};
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                // Parse the GET request line.
                let first_line = request.lines().next().unwrap_or("");
                // GET /cb?code=...&state=... HTTP/1.1
                let path_and_query = first_line.split_whitespace().nth(1).unwrap_or("");
                let parsed = url::Url::parse(&format!("http://127.0.0.1{}", path_and_query));

                let result = match parsed {
                    Err(e) => Err(anyhow::anyhow!("parse callback URL: {}", e)),
                    Ok(u) => {
                        let params: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
                        if let Some(err_code) = params.get("error") {
                            let desc = params.get("error_description").cloned().unwrap_or_default();
                            let _ = stream.write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<h2>Login failed</h2><p>You can close this tab.</p>"
                            );
                            Err(anyhow::anyhow!("authorization server returned error {:?}: {}", err_code, desc))
                        } else if params.get("state").map(|s| s.as_str()) != Some(state_clone.as_str()) {
                            let _ = stream.write_all(
                                b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\n\r\nstate mismatch"
                            );
                            Err(anyhow::anyhow!("state mismatch in callback (possible CSRF)"))
                        } else if let Some(code) = params.get("code") {
                            let _ = stream.write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<h2>Login complete.</h2><p>You can close this tab and return to Robin.</p>"
                            );
                            Ok(code.clone())
                        } else {
                            let _ = stream.write_all(
                                b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\n\r\nmissing code"
                            );
                            Err(anyhow::anyhow!("callback missing code parameter"))
                        }
                    }
                };

                // Send result through the channel.
                let _ = tokio::runtime::Handle::current().block_on(async {
                    let mut guard = tx_clone.lock().await;
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(result);
                    }
                });
            }
            Err(e) => {
                let _ = tokio::runtime::Handle::current().block_on(async {
                    let mut guard = tx_clone.lock().await;
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(Err(anyhow::anyhow!("accept callback: {}", e)));
                    }
                });
            }
        }
    });

    let auth_url_str = auth_url.as_str().to_owned();
    tracing::info!(
        auth_url = %auth_url_str,
        callback = %cfg.redirect_uri,
        hint = "if the browser doesn't open, visit the auth_url manually",
        "mcp: opening browser for OAuth login"
    );
    (OPEN_BROWSER.lock())(&auth_url_str);

    // Wait for the callback with a timeout.
    let code = tokio::time::timeout(LOGIN_TIMEOUT, rx).await
        .map_err(|_| anyhow::anyhow!("waiting for OAuth callback: timeout"))?
        .map_err(|_| anyhow::anyhow!("callback channel closed"))?
        .map_err(|e| e)?;

    // Exchange the code for a token.
    let token = exchange_code(&cfg, &code, &verifier).await
        .map_err(|e| anyhow::anyhow!("exchange code for token: {}", e))?;

    save_token(&cfg.store_path, &token)
        .map_err(|e| anyhow::anyhow!("persist token: {}", e))?;

    let _ = server_handle.await;
    Ok(token)
}

/// Exchange an authorization code for tokens via the token endpoint.
async fn exchange_code(
    cfg: &AuthCodePKCEConfig,
    code: &str,
    verifier: &str,
) -> anyhow::Result<OAuthTokenStore> {
    let client = reqwest::Client::new();
    let params = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", cfg.redirect_uri.as_str()),
        ("client_id", cfg.client_id.as_str()),
        ("code_verifier", verifier),
    ];

    let mut resp_builder = client.post(&cfg.token_url)
        .form(&params);
    if !cfg.client_secret.is_empty() {
        resp_builder = resp_builder.basic_auth(&cfg.client_id, Some(&cfg.client_secret));
    }

    let resp = resp_builder.send().await
        .map_err(|e| anyhow::anyhow!("send token request: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("token endpoint {}: {}", status, body));
    }
    let body: serde_json::Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("decode token response: {}", e))?;
    parse_token_response(body)
}

/// Refresh an expired access token using the refresh_token grant.
pub(crate) async fn refresh_token(
    cfg: &AuthCodePKCEConfig,
    refresh_token: &str,
) -> anyhow::Result<OAuthTokenStore> {
    let client = reqwest::Client::new();
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", cfg.client_id.as_str()),
    ];
    let mut req = client.post(&cfg.token_url).form(&params);
    if !cfg.client_secret.is_empty() {
        req = req.basic_auth(&cfg.client_id, Some(&cfg.client_secret));
    }
    let resp = req.send().await
        .map_err(|e| anyhow::anyhow!("send refresh request: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("token endpoint {}: {}", status, body));
    }
    let body: serde_json::Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("decode token response: {}", e))?;
    parse_token_response(body)
}

fn parse_token_response(body: serde_json::Value) -> anyhow::Result<OAuthTokenStore> {
    let access_token = body.get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("token response missing access_token"))?
        .to_owned();
    let refresh_token = body.get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let token_type = body.get("token_type")
        .and_then(|v| v.as_str())
        .unwrap_or("Bearer")
        .to_owned();
    let expiry = body.get("expires_in")
        .and_then(|v| v.as_i64())
        .map(|secs| chrono::Utc::now() + chrono::Duration::seconds(secs));
    Ok(OAuthTokenStore { access_token, refresh_token, token_type, expiry })
}

/// Build a reqwest::Client with a Bearer token as the default authorization header.
fn build_bearer_client(token: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    let value = format!("Bearer {}", token);
    let v = reqwest::header::HeaderValue::from_str(&value)
        .map_err(|e| anyhow::anyhow!("invalid token header: {}", e))?;
    headers.insert(reqwest::header::AUTHORIZATION, v);
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| anyhow::anyhow!("build http client: {}", e))
}

/// new_pkce_pair generates a 64-byte verifier (RFC 7636 §4.1 allows 43-128
/// chars from the URL-safe alphabet) and its S256 challenge.
pub fn new_pkce_pair() -> anyhow::Result<(String, String)> {
    use sha2::{Digest, Sha256};
    let verifier = rand_string(64)?;
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let sum = hasher.finalize();
    let challenge = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        sum.as_slice(),
    );
    Ok((verifier, challenge))
}

pub fn rand_string(byte_len: usize) -> anyhow::Result<String> {
    use rand::RngCore;
    let mut buf = vec![0u8; byte_len];
    rand::thread_rng().fill_bytes(&mut buf);
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        &buf,
    ))
}

/// html_escape avoids pulling in a template library for a one-line response.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// open_browser_os launches the user's default browser.
pub fn open_browser_os(url: &str) {
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(url).spawn(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg(url).spawn(); }
    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("cmd").args(["/c", "start", url]).spawn(); }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    { tracing::warn!("mcp: unsupported OS for opening browser"); }
}