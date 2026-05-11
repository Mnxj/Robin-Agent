#[cfg(test)]
mod tests {
    use crate::memory::embedder::{Embedder, OpenAiEmbedder};

    /// Verifies that `OpenAiEmbedder` passes an unrecognised model name verbatim
    /// to the wire payload rather than silently downgrading to ada-002.
    ///
    /// We spin up a tiny HTTP server with `wiremock` that captures the request
    /// body and returns a minimal valid embedding response.
    #[tokio::test]
    async fn test_new_open_ai_embedder_passes_nomic_model_verbatim() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data": [{"embedding": [0.1, 0.2, 0.3], "index": 0, "object": "embedding"}],
                    "model": "nomic-embed-text",
                    "object": "list",
                    "usage": {"prompt_tokens": 1, "total_tokens": 1}
                })),
            )
            .mount(&mock_server)
            .await;

        let emb = OpenAiEmbedder::new("dummy-key", &mock_server.uri(), "nomic-embed-text");

        let result = emb.embed(vec!["hello".to_string()]).await;
        assert!(result.is_ok(), "Embed returned error: {:?}", result.err());

        // Verify the request body contained the model name verbatim.
        // wiremock records requests; we check the received request.
        let received = mock_server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let body_str = String::from_utf8_lossy(&received[0].body);
        assert!(
            body_str.contains("nomic-embed-text"),
            "request payload should pass model verbatim; body={}",
            body_str
        );
        assert!(
            !body_str.contains("text-embedding-ada-002"),
            "model was silently rewritten to ada-002; body={}",
            body_str
        );
    }
}