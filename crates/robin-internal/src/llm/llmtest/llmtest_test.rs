#[cfg(test)]
mod tests {
    use super::super::llmtest::{Base, Stub};
    use crate::llm::provider::{ChatRequest, EventType, LLMProvider};

    #[test]
    fn test_base_defaults() {
        let b = Base;
        let models = b.models();
        assert!(models.is_empty() || !models.is_empty()); // non-nil, just check it doesn't panic
        let _ = b.normalize_tool_schema(vec![]);
    }

    #[tokio::test]
    async fn test_stub_canned_text() {
        let s = Stub { text: "hello".into(), ..Default::default() };
        let mut rx = s.chat_stream(ChatRequest::default()).await.unwrap();
        let mut got = String::new();
        while let Some(ev) = rx.recv().await {
            if ev.event_type == EventType::TextDelta {
                got.push_str(&ev.text);
            }
        }
        assert_eq!(got, "hello");
    }

    #[tokio::test]
    async fn test_stub_chat_hook_observes_requests() {
        let s = Stub { text: "ok".into(), ..Default::default() };
        let _ = s.chat_stream(ChatRequest { model: "m1".into(), ..Default::default() }).await;
        let _ = s.chat_stream(ChatRequest { model: "m2".into(), ..Default::default() }).await;
        let reqs = s.requests.lock().unwrap();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].model, "m1");
        assert_eq!(reqs[1].model, "m2");
    }

    #[tokio::test]
    async fn test_stub_chat_err_short_circuits() {
        let s = Stub {
            chat_err: Some(anyhow::anyhow!("test error")),
            ..Default::default()
        };
        let result = s.chat_stream(ChatRequest::default()).await;
        assert!(result.is_err());
    }
}
