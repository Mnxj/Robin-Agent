use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

const EMBED_CACHE_VERSION: u32 = 1;

/// Per-entry cache item: the mtime at embedding time plus the vector itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedCacheItem {
    #[serde(rename = "mtime")]
    pub mod_time: DateTime<Utc>,
    #[serde(rename = "vec")]
    pub vector: Vec<f32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EmbedCacheFile {
    #[serde(rename = "v")]
    version: u32,
    /// Fingerprint: "model=<name>" or "default"
    #[serde(rename = "embedder")]
    embedder: String,
    #[serde(rename = "embeddings")]
    embeddings: HashMap<String, EmbedCacheItem>,
}

/// Persists per-entry embeddings to disk so Manager::load doesn't
/// re-call the embedder on every startup.
///
/// Cache layout: a single JSON file at `<base_dir>/.embeddings-cache.json`
/// mapping entry id → record. Keyed by (id, mod_time). When the embedder
/// model fingerprint changes the whole cache is invalidated.
///
/// Failure modes are non-fatal — a corrupt cache is logged and ignored;
/// Load falls back to full re-embed.
pub struct EmbedCache {
    path: PathBuf,
    mu: Mutex<()>,
}

impl EmbedCache {
    pub fn new(base_dir: &std::path::Path) -> Self {
        Self {
            path: base_dir.join(".embeddings-cache.json"),
            mu: Mutex::new(()),
        }
    }

    /// Reads the cache file. Returns `(None, "")` for missing or corrupt files.
    /// The returned fingerprint is used to detect embedder swaps.
    pub fn load(&self) -> (Option<HashMap<String, EmbedCacheItem>>, String) {
        let _guard = self.mu.lock();
        let data = match std::fs::read(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, String::new()),
            Err(e) => {
                tracing::debug!("embedding cache read error; ignoring: path={:?} error={}", self.path, e);
                return (None, String::new());
            }
        };

        let f: EmbedCacheFile = match serde_json::from_slice(&data) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("embedding cache parse error; ignoring: path={:?} error={}", self.path, e);
                return (None, String::new());
            }
        };

        if f.version != EMBED_CACHE_VERSION {
            return (None, String::new());
        }

        (Some(f.embeddings), f.embedder)
    }

    /// Writes the cache atomically via write-then-rename.
    /// No-op on any I/O error — cache misses cost an embed call, not correctness.
    pub fn save(&self, cache: &HashMap<String, EmbedCacheItem>, fingerprint: &str) {
        let _guard = self.mu.lock();

        let f = EmbedCacheFile {
            version: EMBED_CACHE_VERSION,
            embedder: fingerprint.to_string(),
            embeddings: cache.clone(),
        };

        let data = match serde_json::to_vec(&f) {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!("embedding cache marshal failed: error={}", e);
                return;
            }
        };

        let tmp = self.path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, &data) {
            tracing::debug!("embedding cache write failed: path={:?} error={}", tmp, e);
            return;
        }

        if let Err(e) = std::fs::rename(&tmp, &self.path) {
            let _ = std::fs::remove_file(&tmp);
            tracing::debug!("embedding cache rename failed: path={:?} error={}", self.path, e);
        }
    }
}

/// Returns a stable identifier for the embedder so model swaps invalidate the cache.
pub fn embedder_fingerprint(model: &str) -> String {
    if model.is_empty() {
        "default".to_string()
    } else {
        format!("model={}", model)
    }
}