#[cfg(test)]
mod tests {
    use super::super::oauth::{ClientCredentialsConfig, new_client_credentials_http_client};

    /// Spin up a minimal token endpoint that accepts client_credentials grants
    /// and returns a fixed access token. Returns the base URL.
    async fn start_token_server(
        expected_client_id: &'static str,
        expected_client_secret: &'static str,
        expected_scope: &'static str,
        access_token: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use axum::{Router, routing::post, extract::Request};
        use axum::response::IntoResponse;

        let app = Router::new().route("/token", post(move |req: Request| async move {
            let body_bytes = axum::body::to_bytes(req.into_body(), usize::MAX).await.unwrap_or_default();
            let body = String::from_utf8_lossy(&body_bytes).into_owned();
            let params: std::collections::HashMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
                .into_owned()
                .collect();

            let grant_type = params.get("grant_type").cloned().unwrap_or_default();
            assert_eq!(grant_type, "client_credentials");

            if !expected_scope.is_empty() {
                let scope = params.get("scope").cloned().unwrap_or_default();
                assert_eq!(scope, expected_scope);
            }

            let resp = serde_json::json!({
                "access_token": access_token,
                "token_type": "Bearer",
                "expires_in": 3600,
            });
            (
                axum::http::StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                axum::Json(resp),
            ).into_response()
        }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{}", addr), handle)
    }

    async fn start_resource_server() -> (String, tokio::sync::oneshot::Receiver<String>, tokio::task::JoinHandle<()>) {
        use axum::{Router, routing::get, extract::Request};
        use axum::response::IntoResponse;

        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));

        let app = Router::new().route("/resource", get(move |req: Request| {
            let tx = tx.clone();
            async move {
                let auth = req.headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_owned();
                let mut guard = tx.lock().await;
                if let Some(sender) = guard.take() {
                    let _ = sender.send(auth);
                }
                axum::http::StatusCode::OK
            }
        }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{}", addr), rx, handle)
    }

    #[tokio::test]
    async fn test_new_client_credentials_http_client_injects_bearer() {
        let (token_url, _token_handle) = start_token_server("id-x", "secret-y", "test-scope", "tok-abc").await;
        let (resource_url, rx, _resource_handle) = start_resource_server().await;

        let client = new_client_credentials_http_client(ClientCredentialsConfig {
            token_url: format!("{}/token", token_url),
            client_id: "id-x".to_owned(),
            client_secret: "secret-y".to_owned(),
            scope: "test-scope".to_owned(),
        }).await.expect("should build client");

        let _resp = client.get(format!("{}/resource", resource_url)).send().await.unwrap();
        let auth = tokio::time::timeout(std::time::Duration::from_secs(3), rx).await
            .expect("timeout").expect("channel closed");
        assert!(auth.starts_with("Bearer "), "expected Bearer header, got {:?}", auth);
        assert_eq!(auth, "Bearer tok-abc");
    }
}