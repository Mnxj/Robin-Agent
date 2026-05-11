#[cfg(test)]
mod tests {
    use crate::llm::anthropic::{AnthropicProvider, build_anthropic_messages_pub, build_anthropic_system_pub};
    use crate::llm::provider::{ChatRequest, EventType, LLMProvider, Message, ReasoningMode, SystemPromptPart, ToolCall};

    #[test]
    fn test_anthropic_consecutive_tool_results_coalesce() {
        let in_msgs = vec![
            Message { role: "user".into(), content: "do three things in parallel".into(), ..Default::default() },
            Message {
                role: "assistant".into(),
                content: "ok".into(),
                tool_calls: vec![
                    ToolCall { id: "A".into(), name: "search".into(), input: serde_json::json!({"q":"a"}) },
                    ToolCall { id: "B".into(), name: "search".into(), input: serde_json::json!({"q":"b"}) },
                    ToolCall { id: "C".into(), name: "search".into(), input: serde_json::json!({"q":"c"}) },
                ],
                ..Default::default()
            },
            Message { role: "user".into(), tool_call_id: "B".into(), content: "result B".into(), ..Default::default() },
            Message { role: "user".into(), tool_call_id: "A".into(), content: "result A".into(), ..Default::default() },
            Message { role: "user".into(), tool_call_id: "C".into(), content: "result C".into(), ..Default::default() },
            Message { role: "assistant".into(), content: "done".into(), ..Default::default() },
        ];

        let got = build_anthropic_messages_pub(&in_msgs, false);
        assert_eq!(got.len(), 4, "expected 4 messages, got {}", got.len());
        assert_eq!(got[0].role, "user");
        assert_eq!(got[3].role, "assistant");
        // The second message should be assistant with 3 tool_use blocks
        assert_eq!(got[1].role, "assistant");
        // The third message should be user with 3 tool_result blocks
        assert_eq!(got[2].role, "user");
        assert_eq!(got[2].content.len(), 3, "three tool_results must coalesce into one user message");
    }

    #[test]
    fn test_anthropic_single_tool_result_still_separate() {
        let in_msgs = vec![
            Message { role: "user".into(), content: "search".into(), ..Default::default() },
            Message {
                role: "assistant".into(),
                tool_calls: vec![ToolCall { id: "X".into(), name: "search".into(), input: serde_json::json!({"q":"x"}) }],
                ..Default::default()
            },
            Message { role: "user".into(), tool_call_id: "X".into(), content: "result X".into(), ..Default::default() },
            Message { role: "assistant".into(), content: "done".into(), ..Default::default() },
        ];
        let got = build_anthropic_messages_pub(&in_msgs, false);
        assert_eq!(got.len(), 4);
    }

    #[test]
    fn test_anthropic_system_prompt_parts_emit_cache_control() {
        let req = ChatRequest {
            system_prompt_parts: vec![
                SystemPromptPart { text: "static-cached".into(), cache: true },
                SystemPromptPart { text: "dynamic".into(), cache: false },
            ],
            ..Default::default()
        };
        let got = build_anthropic_system_pub(&req);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].text, "static-cached");
        assert!(got[0].cache_control.is_some(), "first part must be cache-marked");
        assert_eq!(got[0].cache_control.as_ref().unwrap().kind, "ephemeral");
        assert_eq!(got[1].text, "dynamic");
        assert!(got[1].cache_control.is_none(), "second part must not be cache-marked");
    }

    #[test]
    fn test_anthropic_system_prompt_string_fallback() {
        let req = ChatRequest { system_prompt: "legacy".into(), ..Default::default() };
        let got = build_anthropic_system_pub(&req);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, "legacy");
        assert!(got[0].cache_control.is_none());
    }

    #[test]
    fn test_anthropic_system_empty_when_both_empty() {
        let req = ChatRequest::default();
        let got = build_anthropic_system_pub(&req);
        assert!(got.is_empty());
    }

    #[test]
    fn test_anthropic_system_skips_empty_parts() {
        let req = ChatRequest {
            system_prompt_parts: vec![
                SystemPromptPart { text: "".into(), cache: false },
                SystemPromptPart { text: "real".into(), cache: true },
            ],
            ..Default::default()
        };
        let got = build_anthropic_system_pub(&req);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, "real");
    }

    #[test]
    fn test_build_thinking_config() {
        let p = AnthropicProvider::new("test-key", "");
        assert!(p.build_thinking_config("claude-sonnet-4-5", ReasoningMode::Off).is_none());
        assert_eq!(p.build_thinking_config("claude-sonnet-4-5", ReasoningMode::Low).unwrap().budget_tokens, 1024);
        assert_eq!(p.build_thinking_config("claude-sonnet-4-5", ReasoningMode::Medium).unwrap().budget_tokens, 4096);
        assert_eq!(p.build_thinking_config("claude-sonnet-4-5", ReasoningMode::High).unwrap().budget_tokens, 16384);
    }

    #[test]
    fn test_build_thinking_config_unsupported_model() {
        let p = AnthropicProvider::new("test-key", "");
        assert!(p.build_thinking_config("claude-3-haiku-20240307", ReasoningMode::High).is_none());
    }

    #[test]
    fn test_build_thinking_config_unknown_model_defaults_supported() {
        let p = AnthropicProvider::new("test-key", "");
        let cfg = p.build_thinking_config("claude-future-model-vNEW", ReasoningMode::Medium);
        assert!(cfg.is_some());
        assert_eq!(cfg.unwrap().budget_tokens, 4096);
    }

    // ── HTTP test server helper ───────────────────────────────────────────────

    struct TestServer { url: String, tx: tokio::sync::oneshot::Sender<()> }
    impl TestServer { async fn shutdown(self) { let _ = self.tx.send(()); } }

    async fn make_test_server(body: &'static str) -> TestServer {
        use axum::{Router, routing::post, response::Response, body::Body};
        use axum::http::{StatusCode, header};

        let app = Router::new().route("/v1/messages", post(move || async move {
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from(body))
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

    #[tokio::test]
    async fn test_anthropic_stream_surfaces_cache_tokens() {
        const SSE: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"x\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":0,\"cache_creation_input_tokens\":42,\"cache_read_input_tokens\":17}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":5}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

        let srv = make_test_server(SSE).await;
        let p = AnthropicProvider::new("key", &srv.url);
        let mut rx = p.chat_stream(ChatRequest { model: "x".into(), ..Default::default() }).await.unwrap();
        let mut done = None;
        while let Some(ev) = rx.recv().await {
            if ev.event_type == EventType::Done { done = Some(ev); }
        }
        let d = done.expect("expected Done");
        let u = d.usage.expect("expected Usage");
        assert_eq!(u.cache_creation_input_tokens, 42);
        assert_eq!(u.cache_read_input_tokens, 17);
        assert_eq!(u.output_tokens, 5);
        assert_eq!(u.input_tokens, 10);
        srv.shutdown().await;
    }

    #[tokio::test]
    async fn test_anthropic_stream_tool_use_input_on_content_block_start() {
        const SSE: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"x\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_abc\",\"name\":\"nl_search\",\"input\":{\"query\":\"latest\",\"limit\":5}}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

        let srv = make_test_server(SSE).await;
        let p = AnthropicProvider::new("key", &srv.url);
        let mut rx = p.chat_stream(ChatRequest::default()).await.unwrap();
        let mut tool_done = None;
        while let Some(ev) = rx.recv().await {
            if ev.event_type == EventType::ToolCallDone { tool_done = Some(ev); }
        }
        let td = tool_done.expect("expected ToolCallDone");
        let tc = td.tool_call.expect("expected tool_call");
        assert_eq!(tc.id, "call_abc");
        assert_eq!(tc.name, "nl_search");
        let s = serde_json::to_string(&tc.input).unwrap();
        assert!(s.contains("latest"), "input from content_block_start must be captured");
        srv.shutdown().await;
    }

    #[tokio::test]
    async fn test_anthropic_stream_empty_tool_input_defaults_to_object() {
        const SSE: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"x\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"e\",\"name\":\"ping\",\"input\":{}}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

        let srv = make_test_server(SSE).await;
        let p = AnthropicProvider::new("key", &srv.url);
        let mut rx = p.chat_stream(ChatRequest::default()).await.unwrap();
        let mut tool_done = None;
        while let Some(ev) = rx.recv().await {
            if ev.event_type == EventType::ToolCallDone { tool_done = Some(ev); }
        }
        let td = tool_done.expect("expected ToolCallDone");
        let tc = td.tool_call.expect("expected tool_call");
        let s = serde_json::to_string(&tc.input).unwrap();
        assert_eq!(s, "{}", "argument-less tool call must serialize as {{}}");
        srv.shutdown().await;
    }
}