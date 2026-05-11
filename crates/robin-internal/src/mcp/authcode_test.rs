#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::super::authcode::{
        new_pkce_pair, rand_string, ErrInteractiveLoginRequired,
        new_auth_code_pkce_http_client, run_interactive_login, set_open_browser_for_test,
        AuthCodePKCEConfig,
    };
    use super::super::creds::{load_token, save_token, OAuthTokenStore};

    fn free_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    fn compute_challenge_for_test(verifier: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let sum = hasher.finalize();
        base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            sum.as_slice(),
        )
    }

    #[test]
    fn test_new_pkce_pair_challenge_matches_verifier() {
        let (verifier, challenge) = new_pkce_pair().unwrap();
        assert!(!verifier.is_empty());
        assert!(!challenge.is_empty());
        // Verifier is 64 random bytes → 86 base64url chars (no padding).
        assert_eq!(verifier.len(), 86, "verifier should be 86 chars");
        // Challenge is sha256(verifier) → 32 bytes → 43 base64url chars.
        assert_eq!(challenge.len(), 43, "challenge should be 43 chars");
        // Sanity: re-derive challenge from verifier and compare.
        let want = compute_challenge_for_test(&verifier);
        assert_eq!(want, challenge);
    }

    #[tokio::test]
    async fn test_new_auth_code_pkce_http_client_no_token_returns_sentinel() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("tok.json").to_str().unwrap().to_owned();

        let result = new_auth_code_pkce_http_client(AuthCodePKCEConfig {
            auth_url: "https://example.invalid/authorize".to_owned(),
            token_url: "https://example.invalid/token".to_owned(),
            client_id: "cid".to_owned(),
            client_secret: String::new(),
            scope: String::new(),
            redirect_uri: String::new(),
            store_path,
        }).await;

        assert!(result.is_err());
        // Should be ErrInteractiveLoginRequired or an error wrapping it.
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("interactive") || err_str.contains("login required"),
            "expected interactive login required error, got: {}", err_str
        );
    }

    #[tokio::test]
    async fn test_new_auth_code_pkce_http_client_uses_cached_token_without_refresh() {
        use axum::{Router, routing::get, extract::Request, response::IntoResponse};

        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("tok.json").to_str().unwrap().to_owned();

        // Save a valid (not expired) token.
        let tok = OAuthTokenStore {
            access_token: "still-good".to_owned(),
            refresh_token: String::new(),
            token_type: "Bearer".to_owned(),
            expiry: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        };
        save_token(&store_path, &tok).unwrap();

        // A token server that fails if called.
        let token_server = wiremock::MockServer::start().await;
        // No mocks → any request returns 501, which would be an error.

        let (resource_url, rx, _handle) = {
            let (tx, rx) = tokio::sync::oneshot::channel::<String>();
            let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));
            let app = Router::new().route("/x", get(move |req: Request| {
                let tx = tx.clone();
                async move {
                    let auth = req.headers().get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("").to_owned();
                    let mut g = tx.lock().await;
                    if let Some(s) = g.take() { let _ = s.send(auth); }
                    axum::http::StatusCode::OK
                }
            }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
            (format!("http://{}", addr), rx, handle)
        };

        let client = new_auth_code_pkce_http_client(AuthCodePKCEConfig {
            auth_url: "http://example.invalid/authorize".to_owned(),
            token_url: token_server.uri(),
            client_id: "cid".to_owned(),
            client_secret: String::new(),
            scope: String::new(),
            redirect_uri: String::new(),
            store_path,
        }).await.expect("should build client with cached valid token");

        let _resp = client.get(format!("{}/x", resource_url)).send().await.unwrap();
        let auth = tokio::time::timeout(std::time::Duration::from_secs(3), rx).await
            .expect("timeout").expect("channel");
        assert_eq!(auth, "Bearer still-good");
    }

    #[tokio::test]
    async fn test_run_interactive_login_rejects_non_loopback_redirect() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("tok.json").to_str().unwrap().to_owned();

        let result = run_interactive_login(AuthCodePKCEConfig {
            auth_url: "https://idp.example/authorize".to_owned(),
            token_url: "https://idp.example/token".to_owned(),
            client_id: "cid".to_owned(),
            client_secret: String::new(),
            scope: String::new(),
            redirect_uri: "https://evil.example/callback".to_owned(),
            store_path,
        }).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().to_lowercase().contains("loopback"));
    }

    #[tokio::test]
    async fn test_run_interactive_login_completes_pkce_dance() {
        use axum::{Router, routing::{get, post}, extract::Request, response::IntoResponse};

        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("tok.json").to_str().unwrap().to_owned();
        let port = free_port();
        let redirect_uri = format!("http://127.0.0.1:{}/cb", port);

        // Token endpoint: exchange code for token.
        let (token_url, _token_handle) = {
            let app = Router::new().route("/token", post(move |req: Request| async move {
                let body_bytes = axum::body::to_bytes(req.into_body(), usize::MAX).await.unwrap_or_default();
                let params: std::collections::HashMap<_, _> = url::form_urlencoded::parse(&body_bytes)
                    .into_owned().collect();
                assert_eq!(params.get("grant_type").map(|s| s.as_str()), Some("authorization_code"));
                assert_eq!(params.get("code").map(|s| s.as_str()), Some("the-code"));
                assert!(params.get("code_verifier").map(|s| !s.is_empty()).unwrap_or(false));

                let resp = serde_json::json!({
                    "access_token": "minted",
                    "refresh_token": "ref",
                    "token_type": "Bearer",
                    "expires_in": 3600,
                });
                (axum::http::StatusCode::OK,
                 [(axum::http::header::CONTENT_TYPE, "application/json")],
                 axum::Json(resp)).into_response()
            }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
            (format!("http://{}", addr), handle)
        };

        // Authorize endpoint: redirects back to the callback with code+state.
        let redirect_uri_clone = redirect_uri.clone();
        let (authorize_url, _auth_handle) = {
            let app = Router::new().route("/authorize", get(move |req: Request| {
                let redir = redirect_uri_clone.clone();
                async move {
                    let query: std::collections::HashMap<_, _> = req.uri().query()
                        .map(|q| url::form_urlencoded::parse(q.as_bytes()).into_owned().collect())
                        .unwrap_or_default();
                    let state = query.get("state").cloned().unwrap_or_default();
                    assert!(!state.is_empty());
                    assert!(!query.get("code_challenge").cloned().unwrap_or_default().is_empty());
                    assert_eq!(query.get("code_challenge_method").map(|s| s.as_str()), Some("S256"));
                    let target = format!("{}?code=the-code&state={}", redir, url::form_urlencoded::byte_serialize(state.as_bytes()).collect::<String>());
                    axum::response::Redirect::to(&target).into_response()
                }
            }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
            (format!("http://{}", addr), handle)
        };

        // "Browser": instead of launching xdg-open, GET the authorize URL from a goroutine.
        set_open_browser_for_test(Some(|url: &str| {
            let url = url.to_owned();
            std::thread::spawn(move || {
                // Follow the redirect chain.
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all().build().unwrap();
                rt.block_on(async move {
                    let _ = reqwest::Client::builder()
                        .redirect(reqwest::redirect::Policy::limited(10))
                        .build().unwrap()
                        .get(&url).send().await;
                });
            });
        }));

        let tok = run_interactive_login(AuthCodePKCEConfig {
            auth_url: format!("{}/authorize", authorize_url),
            token_url: format!("{}/token", token_url),
            client_id: "cid".to_owned(),
            client_secret: String::new(),
            scope: "openid offline_access".to_owned(),
            redirect_uri,
            store_path: store_path.clone(),
        }).await.expect("interactive login should complete");

        set_open_browser_for_test(None);

        assert_eq!(tok.access_token, "minted");

        let loaded = load_token(&store_path).unwrap().unwrap();
        assert_eq!(loaded.access_token, "minted");
    }
}