use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tracing::{debug, info, warn};

use crate::config::config::{CortexConfig, MemoryConfig, ProviderConfig};

/// Mirrors `conversation.Message` from `github.com/sausheong/cortex`.
#[derive(Clone, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// Result entry returned by cortex Recall.
#[derive(Clone, Debug)]
pub struct CortexResult {
    pub r#type: String,
    pub content: String,
    pub source: String,
}

/// Opaque cortex client. In Go this is `*cortex.Cortex`.
/// This stub would be replaced by an actual cortex integration or FFI.
pub struct Cortex {
    pub db_path: String,
}

impl Cortex {
    pub fn open(db_path: &str) -> anyhow::Result<Self> {
        Ok(Cortex { db_path: db_path.to_string() })
    }

    pub async fn recall(&self, _query: &str) -> anyhow::Result<Vec<CortexResult>> {
        Ok(Vec::new())
    }

    pub async fn ingest(&self, _thread: &[Message]) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

const MIN_INGEST_LEN: usize = 300;
const MIN_RECALL_LEN: usize = 12;
const MAX_INGEST_LEN: usize = 28000;
const INGEST_TIMEOUT: Duration = Duration::from_secs(90);

fn trivial_phrases() -> HashSet<&'static str> {
    [
        "ok", "okay", "thanks", "thank you", "yes", "no", "sure", "got it",
        "understood", "hi", "hello", "hey", "bye", "goodbye", "good morning", "good night",
    ]
    .into_iter()
    .collect()
}

/// Returns true if the message is substantial enough to recall from cortex.
pub fn should_recall(user_msg: &str) -> bool {
    let trimmed = user_msg.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trivial_phrases().contains(trimmed.to_lowercase().as_str()) {
        return false;
    }
    trimmed.len() >= MIN_RECALL_LEN
}

/// Returns true if the thread contains enough substance to ingest.
pub fn should_ingest(thread: &[Message]) -> bool {
    if thread.is_empty() {
        return false;
    }
    let phrases = trivial_phrases();
    if thread[0].role == "user" && phrases.contains(thread[0].content.trim().to_lowercase().as_str()) {
        return false;
    }
    let mut total = 0usize;
    let mut has_assistant = false;
    for m in thread {
        total += m.content.trim().len();
        if m.role == "assistant" {
            has_assistant = true;
        }
    }
    if !has_assistant || total < MIN_INGEST_LEN {
        return false;
    }
    if total > MAX_INGEST_LEN {
        debug!(chars = total, cap = MAX_INGEST_LEN, "cortex: skipping oversized thread");
        return false;
    }
    true
}

/// Resolve (provider, model) for cortex. Mirrors Go's resolveCortexModel.
fn resolve_cortex_model(cfg: &CortexConfig, agent_model: &str) -> (String, String) {
    if !cfg.provider.is_empty() && !cfg.llm_model.is_empty() {
        return (cfg.provider.clone(), cfg.llm_model.clone());
    }
    let (p, m) = crate::llm::parse_provider_model(agent_model);
    (p.to_string(), m.to_string())
}

/// Ingest a thread synchronously with a timeout. Mirrors Go's IngestThread.
pub async fn ingest_thread(cx: &Cortex, thread: &[Message]) {
    if !should_ingest(thread) {
        debug!("cortex: skipping ingest (trivial, too small, or too large)");
        return;
    }
    let result = tokio::time::timeout(INGEST_TIMEOUT, cx.ingest(thread)).await;
    match result {
        Err(_) => warn!("cortex: thread ingest timed out"),
        Ok(Err(e)) => warn!("cortex: thread ingest failed: {}", e),
        Ok(Ok(())) => {}
    }
}

/// Queue a thread for background ingestion. Mirrors Go's IngestThreadAsync.
pub fn ingest_thread_async(cx: Arc<Cortex>, thread: Vec<Message>) {
    if !should_ingest(&thread) {
        return;
    }
    tokio::spawn(async move {
        ingest_thread(&cx, &thread).await;
    });
}

/// Wait for all background ingests to complete. Mirrors Go's Drain.
/// The Rust version is best-effort: callers should use structured concurrency
/// (e.g. a `JoinSet`) rather than a global WaitGroup.
pub async fn drain() {}

/// Build cortex recall results for injection into the agent system prompt.
pub fn format_results(results: &[CortexResult]) -> String {
    if results.is_empty() {
        return String::new();
    }
    let mut b = String::from(
        "\n\n## Cortex Knowledge Graph\n\nThe following knowledge was retrieved from your knowledge graph and is relevant to the current message:\n\n",
    );
    for r in results {
        match r.r#type.as_str() {
            "entity" => b.push_str("- [entity] "),
            "memory" => b.push_str("- [memory] "),
            "chunk" => b.push_str("- [context] "),
            other => b.push_str(&format!("- [{}] ", other)),
        }
        let content = if r.content.len() > 500 {
            format!("{}...", &r.content[..500])
        } else {
            r.content.clone()
        };
        b.push_str(&content);
        if !r.source.is_empty() {
            b.push_str(" (source: ");
            b.push_str(&r.source);
            b.push(')');
        }
        b.push('\n');
    }
    b
}

pub const CORTEX_HINT: &str = r#"

You have access to Cortex, a persistent knowledge graph that automatically stores and retrieves knowledge across conversations. Cortex extracts entities (people, organizations, places, concepts), relationships between them, and factual memories from every conversation.

How Cortex works for you:
- AUTOMATIC STORAGE: After each conversation turn, entities, relationships, and facts are automatically extracted and stored. You do not need to do anything to save knowledge.
- AUTOMATIC RETRIEVAL: Before each response, Cortex searches its knowledge graph for information relevant to the user's message. Results appear below under "Cortex Knowledge Graph".
- CORTEX FIRST — ALWAYS: Before using any tool (web_fetch, web_search, bash, read_file, or any other), check whether the "Cortex Knowledge Graph" section below already contains the answer. Only reach for a tool if Cortex does not have sufficient information.
- USE THE CONTEXT: When Cortex results appear, incorporate that knowledge naturally into your response. Reference what you know about people, organizations, past conversations, and relationships.
- CONNECT THE DOTS: If a user mentions a person or topic that Cortex has data on, proactively surface relevant connections and context — don't wait to be asked.
- ACKNOWLEDGE MEMORY: When you use Cortex knowledge, you can say things like "From our previous conversations..." or "I recall that..." to indicate you remember."#;

/// Caches per-(provider, model) Cortex clients. Mirrors Go's Provider.
pub struct Provider {
    db_path: String,
    cfg: CortexConfig,
    mem_cfg: MemoryConfig,
    get_provider: Arc<dyn Fn(&str) -> ProviderConfig + Send + Sync>,
    clients: Mutex<HashMap<String, Arc<Cortex>>>,
}

impl Provider {
    pub fn new(
        cfg: CortexConfig,
        mem_cfg: MemoryConfig,
        get_provider: impl Fn(&str) -> ProviderConfig + Send + Sync + 'static,
    ) -> Self {
        let db_path = if cfg.db_path.is_empty() {
            format!("{}/brain.db", crate::config::config::default_data_dir())
        } else {
            cfg.db_path.clone()
        };
        Self {
            db_path,
            cfg,
            mem_cfg,
            get_provider: Arc::new(get_provider),
            clients: Mutex::new(HashMap::new()),
        }
    }

    pub fn for_agent(&self, agent_model: &str) -> anyhow::Result<Arc<Cortex>> {
        let (provider, model) = resolve_cortex_model(&self.cfg, agent_model);
        let key = format!("{}/{}", provider, model);

        let mut clients = self.clients.lock();
        if let Some(cx) = clients.get(&key) {
            return Ok(Arc::clone(cx));
        }
        let cx = Arc::new(Cortex::open(&self.db_path)?);
        clients.insert(key.clone(), Arc::clone(&cx));
        info!(
            agent_model = agent_model,
            resolved_provider = provider,
            resolved_model = model,
            db = %self.db_path,
            "cortex client built"
        );
        Ok(cx)
    }

    pub fn close(&self) -> anyhow::Result<()> {
        let mut clients = self.clients.lock();
        let mut first_err: Option<anyhow::Error> = None;
        for (_, cx) in clients.drain() {
            if let Err(e) = cx.close() {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        if let Some(e) = first_err { Err(e) } else { Ok(()) }
    }
}