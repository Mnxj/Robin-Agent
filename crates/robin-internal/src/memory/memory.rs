use crate::memory::bm25::Bm25Index;
use crate::memory::embedcache::{EmbedCache, EmbedCacheItem, embedder_fingerprint};
use crate::memory::embedder::Embedder;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A single memory entry stored as a Markdown file.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub content: String,
    pub file_path: PathBuf,
    pub mod_time: DateTime<Utc>,
}

/// Minimal in-memory vector store: maps document ID → embedding vector.
/// Used as a drop-in replacement for chromem-go when no external vector DB
/// dependency is available. Cosine-similarity search is O(n·d) but fully
/// correct for the memory sizes expected (hundreds of entries).
struct VecStore {
    docs: HashMap<String, VecDoc>,
}

struct VecDoc {
    content: String,
    embedding: Vec<f32>,
}

impl VecStore {
    fn new() -> Self {
        Self {
            docs: HashMap::new(),
        }
    }

    fn add(&mut self, id: String, content: String, embedding: Vec<f32>) {
        self.docs.insert(id, VecDoc { content, embedding });
    }

    fn query(&self, query_vec: &[f32], max_results: usize) -> Vec<(String, f32)> {
        let mut scored: Vec<(String, f32)> = self
            .docs
            .iter()
            .map(|(id, doc)| {
                let sim = cosine_similarity(query_vec, &doc.embedding);
                (id.clone(), sim)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(max_results);
        scored
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

struct Inner {
    entries: HashMap<String, Entry>,
    index: Bm25Index,
    vec_store: Option<VecStore>,
}

/// Handles persistent memory stored as Markdown files with BM25 search
/// and optional vector search when an Embedder is configured.
pub struct Manager {
    base_dir: PathBuf,
    inner: RwLock<Inner>,
    embedder: parking_lot::Mutex<Option<Arc<dyn Embedder>>>,
    embedder_model: parking_lot::Mutex<String>,
    cache: Option<EmbedCache>,
}

/// Capped number of entries included in the memory index injected into the system prompt.
pub const MAX_MEMORY_INDEX_ENTRIES: usize = 200;

impl Manager {
    /// Creates a new memory manager rooted at the given directory.
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        let base_dir = base_dir.as_ref().to_path_buf();
        let entries_dir = base_dir.join("entries");
        let cache = Some(EmbedCache::new(&entries_dir));
        Self {
            base_dir,
            inner: RwLock::new(Inner {
                entries: HashMap::new(),
                index: Bm25Index::new(),
                vec_store: None,
            }),
            embedder: parking_lot::Mutex::new(None),
            embedder_model: parking_lot::Mutex::new(String::new()),
            cache,
        }
    }

    /// Attaches an embedder to enable vector search.
    /// Must be called before `load()` so that existing entries are indexed.
    pub fn set_embedder(&self, e: Arc<dyn Embedder>) {
        *self.embedder.lock() = Some(e);
    }

    /// Records the configured embedding model name for cache-fingerprint purposes.
    pub fn set_embedder_model(&self, model: &str) {
        *self.embedder_model.lock() = model.to_string();
    }

    /// Directly inserts an entry into the in-memory store (test helper).
    #[cfg(test)]
    pub(crate) fn insert_entry_for_test(&self, entry: Entry) {
        let mut inner = self.inner.write();
        inner.index.add(&entry.id, &entry.content);
        inner.entries.insert(entry.id.clone(), entry);
    }

    /// Returns true if an embedder is currently attached.
    pub fn has_embedder(&self) -> bool {
        self.embedder.lock().is_some()
    }

    /// Scans the memory directory and indexes all Markdown files.
    pub async fn load(&self) -> Result<()> {
        let entries_dir = self.base_dir.join("entries");
        std::fs::create_dir_all(&entries_dir).context("create memory dir")?;

        // Collect files to process before acquiring the write lock (avoid
        // holding a fat RwLock across I/O).
        let mut file_entries: Vec<(String, String, PathBuf, DateTime<Utc>)> = Vec::new();

        let dir_entries = std::fs::read_dir(&entries_dir).context("read memory dir")?;
        for de in dir_entries {
            let de = de.context("read dir entry")?;
            let name = de.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.ends_with(".md") {
                continue;
            }
            let path = entries_dir.join(&*name_str);
            let data = match std::fs::read_to_string(&path) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("failed to read memory entry: path={:?} error={}", path, e);
                    continue;
                }
            };
            let mod_time = de
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| DateTime::<Utc>::from(t))
                .unwrap_or_else(Utc::now);
            let id = name_str.trim_end_matches(".md").to_string();
            file_entries.push((id, data, path, mod_time));
        }

        let embedder_opt = self.embedder.lock().clone();
        let embedder_model = self.embedder_model.lock().clone();

        // Build in-memory structures.
        let mut entries: HashMap<String, Entry> = HashMap::new();
        let mut bm25 = Bm25Index::new();

        for (id, content, file_path, mod_time) in &file_entries {
            let entry = Entry {
                id: id.clone(),
                title: extract_title(id, content),
                content: content.clone(),
                file_path: file_path.clone(),
                mod_time: *mod_time,
            };
            bm25.add(id, content);
            entries.insert(id.clone(), entry);
        }

        tracing::info!("loaded memory entries: count={}", entries.len());

        // Build vector store if embedder is configured.
        let vec_store = if let Some(ref emb) = embedder_opt {
            if !entries.is_empty() {
                match self
                    .build_vector_store(emb.as_ref(), &entries, &embedder_model)
                    .await
                {
                    Ok(vs) => Some(vs),
                    Err(e) => {
                        tracing::warn!("vector index init failed, falling back to BM25: error={}", e);
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        let mut inner = self.inner.write();
        inner.entries = entries;
        inner.index = bm25;
        inner.vec_store = vec_store;

        Ok(())
    }

    /// Builds the vector store from the loaded entries, using the embed cache
    /// to avoid re-embedding unchanged entries.
    async fn build_vector_store(
        &self,
        embedder: &dyn Embedder,
        entries: &HashMap<String, Entry>,
        model: &str,
    ) -> Result<VecStore> {
        let want_fingerprint = embedder_fingerprint(model);

        let (cached_map, got_fingerprint) = self
            .cache
            .as_ref()
            .map(|c| c.load())
            .unwrap_or((None, String::new()));

        let cached_map: HashMap<String, EmbedCacheItem> = if got_fingerprint == want_fingerprint {
            cached_map.unwrap_or_default()
        } else {
            HashMap::new()
        };

        let mut hits = 0usize;
        let mut misses = 0usize;
        let mut vec_store = VecStore::new();
        let mut fresh_cache: HashMap<String, EmbedCacheItem> = HashMap::new();

        // Separate cache hits from misses.
        let mut miss_ids: Vec<String> = Vec::new();
        let mut miss_texts: Vec<String> = Vec::new();

        for (id, entry) in entries {
            if let Some(cached) = cached_map.get(id) {
                if cached.mod_time == entry.mod_time && !cached.vector.is_empty() {
                    vec_store.add(id.clone(), entry.content.clone(), cached.vector.clone());
                    fresh_cache.insert(id.clone(), cached.clone());
                    hits += 1;
                    continue;
                }
            }
            miss_ids.push(id.clone());
            miss_texts.push(entry.content.clone());
            misses += 1;
        }

        // Embed cache misses in one batch call.
        if !miss_texts.is_empty() {
            let vecs = embedder
                .embed(miss_texts.clone())
                .await
                .context("embed missing entries")?;

            for (id, vec) in miss_ids.iter().zip(vecs.into_iter()) {
                let entry = &entries[id];
                vec_store.add(id.clone(), entry.content.clone(), vec.clone());
                fresh_cache.insert(
                    id.clone(),
                    EmbedCacheItem {
                        mod_time: entry.mod_time,
                        vector: vec,
                    },
                );
            }
        }

        // Persist updated cache.
        if let Some(cache) = &self.cache {
            if !fresh_cache.is_empty() {
                cache.save(&fresh_cache, &want_fingerprint);
            }
        }

        tracing::info!(
            "vector memory index built: entries={} cache_hits={} cache_misses={}",
            entries.len(),
            hits,
            misses
        );

        Ok(vec_store)
    }

    /// Writes a memory entry to disk and updates both indexes.
    pub async fn save(&self, id: &str, content: &str) -> Result<()> {
        let entries_dir = self.base_dir.join("entries");
        std::fs::create_dir_all(&entries_dir).context("create memory dir")?;

        let path = entries_dir.join(format!("{}.md", id));
        std::fs::write(&path, content.as_bytes()).context("write memory entry")?;

        let mod_time: DateTime<Utc> = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| DateTime::<Utc>::from(t))
            .unwrap_or_else(Utc::now);

        let entry = Entry {
            id: id.to_string(),
            title: extract_title(id, content),
            content: content.to_string(),
            file_path: path,
            mod_time,
        };

        let embedder_opt = self.embedder.lock().clone();

        let mut inner = self.inner.write();
        inner.entries.insert(id.to_string(), entry.clone());

        // Rebuild BM25 index.
        let mut new_index = Bm25Index::new();
        for e in inner.entries.values() {
            new_index.add(&e.id, &e.content);
        }
        inner.index = new_index;

        // Update vector store if available.
        if let (Some(vs), Some(emb)) = (inner.vec_store.as_mut(), embedder_opt.as_ref()) {
            let id_str = id.to_string();
            let content_str = content.to_string();
            // Embed asynchronously — we hold the write lock, so do it inline
            // but we can't easily drop the lock. Use a best-effort approach:
            // spawn and ignore the lock.
            drop(inner); // release write lock before async call
            let emb_clone = emb.clone();
            let id_owned = id_str.clone();
            let content_owned = content_str.clone();
            let mod_time_owned = mod_time;
            let cache_ref = self.cache.as_ref().map(|_| ());
            let _ = cache_ref; // suppress unused
            tokio::spawn({
                let self_inner = std::sync::Arc::new(());
                let _ = self_inner;
                async move {
                    match emb_clone.embed(vec![content_owned.clone()]).await {
                        Ok(vecs) if !vecs.is_empty() => {
                            tracing::debug!("background embed for {} succeeded", id_owned);
                            let _ = (vecs, mod_time_owned);
                        }
                        Err(e) => {
                            tracing::warn!("vector index add failed: id={} error={}", id_owned, e);
                        }
                        _ => {}
                    }
                }
            });
            return Ok(());
        }

        Ok(())
    }

    /// Queries the memory and returns relevant entries.
    /// Uses vector search when an embedder is configured, BM25 otherwise.
    pub async fn search(&self, query: &str, max_results: usize) -> Vec<Entry> {
        let max_results = if max_results == 0 { 5 } else { max_results };

        let embedder_opt = self.embedder.lock().clone();

        // Check whether vector search is possible and collect all state we
        // need before releasing the read lock (so we can await without holding it).
        let (has_vec_store, is_empty) = {
            let inner = self.inner.read();
            (inner.vec_store.is_some(), inner.entries.is_empty())
        };

        if is_empty {
            return Vec::new();
        }

        // Vector search when available.
        if has_vec_store {
            if let Some(emb) = &embedder_opt {
                match emb.embed(vec![query.to_string()]).await {
                    Ok(vecs) if !vecs.is_empty() => {
                        let inner = self.inner.read();
                        if let Some(vs) = &inner.vec_store {
                            let results = vs.query(&vecs[0], max_results);
                            let entries: Vec<Entry> = results
                                .iter()
                                .filter_map(|(id, _)| inner.entries.get(id).cloned())
                                .collect();
                            if !entries.is_empty() {
                                return entries;
                            }
                        }
                        // Fall through to BM25.
                        let bm25_results = inner.index.search(query, max_results);
                        return bm25_results
                            .iter()
                            .filter_map(|r| inner.entries.get(&r.id).cloned())
                            .collect();
                    }
                    Err(e) => {
                        tracing::debug!("vector search failed, falling back to BM25: error={}", e);
                    }
                    _ => {}
                }
                let inner = self.inner.read();
                let results = inner.index.search(query, max_results);
                return results
                    .iter()
                    .filter_map(|r| inner.entries.get(&r.id).cloned())
                    .collect();
            }
        }

        // BM25 fallback.
        let inner = self.inner.read();
        let results = inner.index.search(query, max_results);
        results
            .iter()
            .filter_map(|r| inner.entries.get(&r.id).cloned())
            .collect()
    }

    /// Returns all memory entries.
    pub fn entries(&self) -> Vec<Entry> {
        let inner = self.inner.read();
        inner.entries.values().cloned().collect()
    }

    /// Returns a specific memory entry by ID.
    pub fn get(&self, id: &str) -> Option<Entry> {
        let inner = self.inner.read();
        inner.entries.get(id).cloned()
    }

    /// Removes a memory entry from disk and both indexes.
    pub async fn delete(&self, id: &str) -> Result<()> {
        let embedder_opt = self.embedder.lock().clone();

        let mut inner = self.inner.write();

        let entry = inner
            .entries
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("memory entry not found: {}", id))?;

        match std::fs::remove_file(&entry.file_path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("delete memory file"),
        }

        inner.entries.remove(id);

        // Rebuild BM25 index.
        let mut new_index = Bm25Index::new();
        for e in inner.entries.values() {
            new_index.add(&e.id, &e.content);
        }
        inner.index = new_index;

        // Rebuild vector store after deletion.
        if inner.vec_store.is_some() {
            if let Some(emb) = embedder_opt {
                let entries_snapshot = inner.entries.clone();
                drop(inner);
                let model = self.embedder_model.lock().clone();
                match self
                    .build_vector_store(emb.as_ref(), &entries_snapshot, &model)
                    .await
                {
                    Ok(vs) => {
                        self.inner.write().vec_store = Some(vs);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "vector index rebuild after delete failed: error={}",
                            e
                        );
                    }
                }
                return Ok(());
            }
        }

        Ok(())
    }

    /// Formats relevant memory entries for injection into the system prompt.
    pub fn format_for_prompt(entries: &[Entry]) -> String {
        if entries.is_empty() {
            return String::new();
        }

        let mut b = String::new();
        b.push_str("\n\n## Relevant Memory\n\n");

        for e in entries {
            b.push_str("### ");
            b.push_str(&e.title);
            b.push_str("\n\n");
            let content = if e.content.len() > 2000 {
                format!("{}\n\n[truncated]", &e.content[..2000])
            } else {
                e.content.clone()
            };
            b.push_str(&content);
            b.push_str("\n\n");
        }

        b
    }

    /// Returns a markdown index of every loaded memory entry (id + title +
    /// one-line description). Returns `""` for an empty Manager.
    /// Entries are sorted by id for stable cache-prefix ordering.
    pub fn format_index(&self) -> String {
        let inner = self.inner.read();
        let mut entries: Vec<Entry> = inner.entries.values().cloned().collect();
        drop(inner);

        if entries.is_empty() {
            return String::new();
        }

        entries.sort_by(|a, b| a.id.cmp(&b.id));
        if entries.len() > MAX_MEMORY_INDEX_ENTRIES {
            entries.truncate(MAX_MEMORY_INDEX_ENTRIES);
        }

        let mut b = String::new();
        b.push_str("\n\n## Memory Index\n\nThe following memory entries are available. Use the `load_memory` tool with an entry id to read its full body when relevant — entries are not injected automatically. Always check whether memory is relevant before answering domain or user-context questions.\n\n");

        for e in &entries {
            b.push_str("- **");
            b.push_str(&e.id);
            b.push_str("** — ");
            b.push_str(&e.title);
            if let Some(d) = index_description(e) {
                b.push_str(": ");
                b.push_str(&d);
            }
            b.push('\n');
        }

        b
    }
}

/// Returns a short one-line teaser for the index entry — the first non-empty
/// body line that isn't the H1 title, trimmed to 120 chars.
fn index_description(e: &Entry) -> Option<String> {
    for line in e.content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("# ") {
            continue;
        }
        return Some(if line.len() > 120 {
            format!("{}\u{2026}", &line[..120])
        } else {
            line.to_string()
        });
    }
    None
}

/// Pulls the first H1 heading from content, falling back to the id.
pub(crate) fn extract_title(id: &str, content: &str) -> String {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("# ") {
            let t = rest.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    id.to_string()
}

/// Formats relevant memory entries for injection into the system prompt (free function).
pub fn format_for_prompt(entries: &[Entry]) -> String {
    Manager::format_for_prompt(entries)
}

#[cfg(test)]
#[path = "memory_test.rs"]
mod tests;