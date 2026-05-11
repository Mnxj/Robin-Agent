#[cfg(test)]
mod tests {
    use crate::memory::embedder::Embedder;
    use crate::memory::memory::Manager;
    use crate::memory::probe::attach_with_probe;
    use anyhow::Result;
    use async_trait::async_trait;
    use std::sync::Arc;

    struct FakeEmbedder {
        fail: bool,
    }

    #[async_trait]
    impl Embedder for FakeEmbedder {
        async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
            if self.fail {
                anyhow::bail!("connection refused");
            }
            let out = texts.iter().map(|_| vec![0.1f32, 0.2f32]).collect();
            Ok(out)
        }
    }

    #[tokio::test]
    async fn test_attach_with_probe_attaches_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = Manager::new(dir.path());
        let emb: Arc<dyn Embedder> = Arc::new(FakeEmbedder { fail: false });

        attach_with_probe(&mgr, emb, "test-model").await;

        assert!(
            mgr.has_embedder(),
            "embedder should be attached on probe success"
        );
    }

    #[tokio::test]
    async fn test_attach_with_probe_skips_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = Manager::new(dir.path());
        let emb: Arc<dyn Embedder> = Arc::new(FakeEmbedder { fail: true });

        attach_with_probe(&mgr, emb, "test-model").await;

        assert!(
            !mgr.has_embedder(),
            "embedder must NOT be attached on probe failure"
        );
    }
}