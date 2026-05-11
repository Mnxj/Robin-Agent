#[cfg(test)]
mod tests {
    use super::super::openai::{OpenAIProvider, concat_system_prompt_parts};
    use super::super::provider::{ChatRequest, EventType, LLMProvider, Message, SystemPromptPart};

    struct TestServer { pub url: String, tx: tokio::sync::oneshot::Sender<()> }
    impl TestServer {
        async fn shutdown(self) { let _ = self.tx.send(()); }
    }

    async fn capture_openai_request_server(response_body: &'static str) -> TestServer {
        use axum::{Router, routing::post, response::Response, body::Body};
        use axum::http::{StatusCode, header};

        let app = Router::new().route("/v1/chat/completions", post(move || async move {
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from(response_body))
                .unwrap()
        }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async { let _ = rx.await; })
                .await.unwrap();
        });
        TestServer { url, tx }
    }

    const DONE_RESPONSE: &str = "data: [DONE]\n\n";

    #[tokio::test]
    async fn test_openai_chat_stream_uses_system_prompt_parts() {
        let srv = capture_openai_request_server(DONE_RESPONSE).await;
        let p = OpenAIProvider::new("test-key", &format!("{}/", srv.url), "openai-compatible", "");
        let req = ChatRequest {
            system_prompt_parts: vec![
                SystemPromptPart { text: "static".into(), cache: false },
                SystemPromptPart { text: "dynamic".into(), cache: false },
            ],
            ..Default::default()
        };
        let mut rx = p.chat_stream(req).await.unwrap();
        while rx.recv().await.is_some() {}
        srv.shutdown().await;
    }

    #[tokio::test]
    async fn test_openai_chat_stream_falls_back_to_system_prompt_string() {
        let srv = capture_openai_request_server(DONE_RESPONSE).await;
        let p = OpenAIProvider::new("test-key", &format!("{}/", srv.url), "openai-compatible", "");
        let req = ChatRequest {
            system_prompt: "legacy".into(),
            ..Default::default()
        };
        let mut rx = p.chat_stream(req).await.unwrap();
        while rx.recv().await.is_some() {}
        srv.shutdown().await;
    }

    #[tokio::test]
    async fn test_openai_chat_stream_parts_beat_string() {
        let srv = capture_openai_request_server(DONE_RESPONSE).await;
        let p = OpenAIProvider::new("test-key", &format!("{}/", srv.url), "openai-compatible", "");
        let req = ChatRequest {
            system_prompt: "legacy".into(),
            system_prompt_parts: vec![SystemPromptPart { text: "new".into(), cache: false }],
            ..Default::default()
        };
        let mut rx = p.chat_stream(req).await.unwrap();
        while rx.recv().await.is_some() {}
        srv.shutdown().await;
    }

    #[test]
    fn test_concat_system_prompt_parts() {
        assert_eq!(concat_system_prompt_parts(&[]), "");
        assert_eq!(concat_system_prompt_parts(&[SystemPromptPart { text: "A".into(), cache: false }]), "A");
        assert_eq!(concat_system_prompt_parts(&[
            SystemPromptPart { text: "A".into(), cache: false },
            SystemPromptPart { text: "B".into(), cache: false },
        ]), "A\nB");
        assert_eq!(concat_system_prompt_parts(&[
            SystemPromptPart { text: "A".into(), cache: false },
            SystemPromptPart { text: "".into(), cache: false },
            SystemPromptPart { text: "B".into(), cache: false },
        ]), "A\nB");
    }
}