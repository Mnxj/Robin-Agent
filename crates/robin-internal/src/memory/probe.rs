use crate::memory::embedder::Embedder;
use crate::memory::memory::Manager;
use std::sync::Arc;
use std::time::Duration;

/// Attaches an embedder to `mgr` only if a single probe embed call succeeds
/// within 5 seconds. Logs a warning and leaves the embedder unset on failure
/// (memory falls back to BM25-only).
pub async fn attach_with_probe(mgr: &Manager, embedder: Arc<dyn Embedder>, model_name: &str) {
    let probe_result = tokio::time::timeout(
        Duration::from_secs(5),
        embedder.embed(vec!["probe".to_string()]),
    )
    .await;

    match probe_result {
        Ok(Ok(_)) => {
            mgr.set_embedder(embedder);
            mgr.set_embedder_model(model_name);
            tracing::info!("memory vector search enabled: model={}", model_name);
        }
        Ok(Err(e)) => {
            tracing::warn!(
                "memory: embedder unavailable, BM25-only: model={} reason={}",
                model_name,
                e
            );
        }
        Err(_) => {
            tracing::warn!(
                "memory: embedder probe timed out, BM25-only: model={}",
                model_name
            );
        }
    }
}

#[cfg(test)]
#[path = "probe_test.rs"]
mod tests;