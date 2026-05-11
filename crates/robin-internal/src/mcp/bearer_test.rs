#[cfg(test)]
mod tests {
    use super::super::bearer::new_bearer_http_client;

    /// Helper: start a tiny HTTP server that records the Authorization header
    /// and responds 200, using wiremock.
    async fn bearer_server_response(
        mock_server: &wiremock::MockServer,
    ) -> String {
        use wiremock::matchers::method;
        use wiremock::{Mock, ResponseTemplate};
        use std::sync::{Arc, Mutex};

        // We use a manual approach: spin a simple tokio HTTP server via axum.
        // Actually, since we can't easily extract headers from wiremock responses,
        // use a simpler approach with a channel.
        // For these tests we use the actual flow: start an httptest-like server
        // using axum on a random port.
        String::new() // placeholder; see actual tests below
    }

    /// Spin up a simple HTTP echo server using tokio + a oneshot channel.
    async fn start_header_capture_server() -> (String, tokio::sync::oneshot::Receiver<String>) {
        use axum::{Router, routing::get, extract::Request};
        use axum::response::IntoResponse;
        use std::net::SocketAddr;

        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));

        let tx_clone = tx.clone();
        let app = Router::new().route("/", get(move |req: Request| {
            let tx = tx_clone.clone();
            async move {
                let auth = req.headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_owned();
                // Send the first request's auth header.
                let mut guard = tx.lock().await;
                if let Some(sender) = guard.take() {
                    let _ = sender.send(auth);
                }
                axum::http::StatusCode::OK
            }
        }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (format!("http://{}", addr), rx)
    }

    #[tokio::test]
    async fn test_bearer_http_client_injects_authorization() {
        let (url, rx) = start_header_capture_server().await;
        let client = new_bearer_http_client("tok-123".to_owned());
        let _resp = client.get(&url).send().await.unwrap();
        let auth = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await
            .expect("timeout").expect("channel closed");
        assert_eq!(auth, "Bearer tok-123");
    }

    #[tokio::test]
    async fn test_bearer_http_client_empty_token_no_header() {
        let (url, rx) = start_header_capture_server().await;
        let client = new_bearer_http_client(String::new());
        let _resp = client.get(&url).send().await.unwrap();
        let auth = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await
            .expect("timeout").expect("channel closed");
        assert!(auth.is_empty(), "no Authorization header should be set when token is empty");
    }
}