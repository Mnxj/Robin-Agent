use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::HashMap;
use std::sync::Arc;

use crate::session::store::Store;

/// Describes the kind of session entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    Message,
    ToolCall,
    ToolResult,
    Meta,
    Compaction,
}

/// A single node in the session DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: String,
    #[serde(rename = "parentId", skip_serializing_if = "String::is_empty", default)]
    pub parent_id: String,
    #[serde(rename = "type")]
    pub entry_type: EntryType,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub role: String,
    pub timestamp: i64,
    pub data: Box<RawValue>,
}

/// Holds a base64-encoded image for session persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageData {
    pub mime_type: String,
    /// base64-encoded image bytes
    pub data: String,
}

/// Holds text message content.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageData {
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub images: Vec<ImageData>,
}

/// Holds a tool call's details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallData {
    pub tool: String,
    pub id: String,
    pub input: Box<RawValue>,
}

/// Holds the result of a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultData {
    pub tool_call_id: String,
    #[serde(default)]
    pub output: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub error: String,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub is_error: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub aborted: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub images: Vec<ImageData>,
}

/// Holds an append-only summary of an older portion of the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionData {
    pub summary: String,
    #[serde(rename = "range_start_id", skip_serializing_if = "String::is_empty", default)]
    pub range_start_id: String,
    #[serde(rename = "range_end_id", skip_serializing_if = "String::is_empty", default)]
    pub range_end_id: String,
    pub model: String,
    pub tokens_before: i64,
    pub tokens_estimated_after: i64,
    pub turns_compacted: i64,
}

struct Inner {
    entries: Vec<SessionEntry>,
    entry_map: HashMap<String, usize>, // id → index in entries
    leaf_id: String,
}

/// Holds a conversation session with DAG-structured entries.
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub key: String,

    inner: RwLock<Inner>,
    store: parking_lot::Mutex<Option<Arc<Store>>>,
}

impl Session {
    /// Creates a new empty session.
    pub fn new(agent_id: &str, key: &str) -> Self {
        Self {
            id: generate_id("ses"),
            agent_id: agent_id.to_string(),
            key: key.to_string(),
            inner: RwLock::new(Inner {
                entries: Vec::new(),
                entry_map: HashMap::new(),
                leaf_id: String::new(),
            }),
            store: parking_lot::Mutex::new(None),
        }
    }

    /// Associates a Store for automatic persistence.
    /// Must be called once during session construction, before the session
    /// is shared with any goroutine.
    pub fn set_store(&self, store: Arc<Store>) {
        *self.store.lock() = Some(store);
    }

    /// Adds an entry to the session.
    pub fn append(&self, mut entry: SessionEntry) {
        let mut inner = self.inner.write();

        if entry.id.is_empty() {
            entry.id = generate_id("e");
        }
        if entry.timestamp == 0 {
            entry.timestamp = chrono::Utc::now().timestamp();
        }
        if !inner.leaf_id.is_empty() && entry.parent_id.is_empty() {
            entry.parent_id = inner.leaf_id.clone();
        }

        let idx = inner.entries.len();
        inner.leaf_id = entry.id.clone();
        inner.entries.push(entry);
        let id = inner.entries[idx].id.clone();
        inner.entry_map.insert(id, idx);

        // Persist if store is set.
        let store_opt = self.store.lock().clone();
        if let Some(store) = store_opt {
            // Release inner lock before calling store to avoid ordering issues.
            let entry_clone = inner.entries[idx].clone();
            drop(inner);
            store.append_entry(self, entry_clone);
            return;
        }
    }

    /// Walks the DAG from root to current leaf and returns the path.
    pub fn history(&self) -> Vec<SessionEntry> {
        let inner = self.inner.read();
        self.walk_from_leaf(&inner, false)
    }

    /// Returns the post-compaction message view for the LLM.
    /// Walks the current branch from leaf back to root; if a compaction entry
    /// is encountered it becomes the first emitted entry and everything before
    /// it is dropped.
    pub fn view(&self) -> Vec<SessionEntry> {
        let inner = self.inner.read();
        self.walk_from_leaf(&inner, true)
    }

    fn walk_from_leaf(&self, inner: &Inner, stop_at_compaction: bool) -> Vec<SessionEntry> {
        if inner.entries.is_empty() {
            return Vec::new();
        }

        let mut path: Vec<SessionEntry> = Vec::new();
        let mut current = inner.leaf_id.clone();

        loop {
            if current.is_empty() {
                break;
            }
            let Some(&idx) = inner.entry_map.get(&current) else {
                break;
            };
            let entry = inner.entries[idx].clone();
            let is_compaction = entry.entry_type == EntryType::Compaction;
            let parent = entry.parent_id.clone();
            path.push(entry);

            if stop_at_compaction && is_compaction {
                break;
            }
            current = parent;
        }

        path.reverse();
        path
    }

    /// Returns all entries in append order.
    pub fn entries(&self) -> Vec<SessionEntry> {
        let inner = self.inner.read();
        inner.entries.clone()
    }

    /// Appends an entry directly into the in-memory DAG **without** triggering
    /// store persistence. Used by `Store::load` when replaying entries from
    /// the JSONL file.
    pub(crate) fn load_entry(&self, entry: SessionEntry) {
        let mut inner = self.inner.write();
        let idx = inner.entries.len();
        inner.leaf_id = entry.id.clone();
        inner.entry_map.insert(entry.id.clone(), idx);
        inner.entries.push(entry);
    }

    /// Returns the current leaf entry ID.
    pub fn leaf_id(&self) -> String {
        self.inner.read().leaf_id.clone()
    }

    /// Moves the leaf pointer to the specified entry ID, creating a branch.
    pub fn branch(&self, entry_id: &str) -> Result<(), String> {
        let mut inner = self.inner.write();
        if !inner.entry_map.contains_key(entry_id) {
            return Err(format!("entry {:?} not found in session", entry_id));
        }
        inner.leaf_id = entry_id.to_string();
        Ok(())
    }

    /// Returns a rough token estimate for the current history (~4 chars/token).
    pub fn estimate_tokens(&self) -> usize {
        let history = self.history();
        let total_chars: usize = history
            .iter()
            .map(|e| e.data.get().len() + e.role.len())
            .sum();
        total_chars / 4
    }

    /// Replaces older history entries with a summary entry.
    /// Keeps the most recent `keep_entries` entries and replaces everything
    /// before them with a single summary meta entry.
    pub fn compact(&self, summary: &str, keep_entries: usize) {
        let history = self.history();
        if history.len() <= keep_entries {
            return; // nothing to compact
        }

        let cutoff = history.len() - keep_entries;
        let recent_entries = history[cutoff..].to_vec();

        // Create summary meta entry.
        let summary_data =
            serde_json::to_string(&MessageData { text: summary.to_string(), images: Vec::new() })
                .unwrap_or_else(|_| "{}".to_string());
        let summary_entry = SessionEntry {
            id: generate_id("compact"),
            parent_id: String::new(),
            entry_type: EntryType::Meta,
            role: "system".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            data: RawValue::from_string(summary_data).unwrap(),
        };

        // Rebuild the session with summary + recent entries.
        let mut inner = self.inner.write();
        inner.entries.clear();
        inner.entry_map.clear();
        inner.leaf_id = String::new();

        // Add summary entry.
        let summary_id = summary_entry.id.clone();
        inner.entries.push(summary_entry);
        inner.entry_map.insert(summary_id.clone(), 0);
        inner.leaf_id = summary_id.clone();

        // Add recent entries, re-parenting the first one to the summary.
        for (i, mut entry) in recent_entries.into_iter().enumerate() {
            if i == 0 {
                entry.parent_id = summary_id.clone();
            }
            // parent_id for subsequent entries is already correct from history walk.
            let idx = inner.entries.len();
            inner.leaf_id = entry.id.clone();
            inner.entry_map.insert(entry.id.clone(), idx);
            inner.entries.push(entry);
        }

        // Rewrite the session file if store is set.
        let store_opt = self.store.lock().clone();
        drop(inner);
        if let Some(store) = store_opt {
            store.rewrite(self);
        }
    }
}

// ── Entry constructor helpers ───────────────────────────────────────────────

/// Creates a user message entry.
pub fn user_message_entry(text: &str) -> SessionEntry {
    let data = serde_json::to_string(&MessageData {
        text: text.to_string(),
        images: Vec::new(),
    })
    .unwrap_or_else(|_| "{}".to_string());
    SessionEntry {
        id: String::new(),
        parent_id: String::new(),
        entry_type: EntryType::Message,
        role: "user".to_string(),
        timestamp: 0,
        data: RawValue::from_string(data).unwrap(),
    }
}

/// Creates a user message entry with image attachments.
pub fn user_message_with_images_entry(text: &str, images: Vec<ImageData>) -> SessionEntry {
    let data = serde_json::to_string(&MessageData {
        text: text.to_string(),
        images,
    })
    .unwrap_or_else(|_| "{}".to_string());
    SessionEntry {
        id: String::new(),
        parent_id: String::new(),
        entry_type: EntryType::Message,
        role: "user".to_string(),
        timestamp: 0,
        data: RawValue::from_string(data).unwrap(),
    }
}

/// Creates an assistant message entry.
pub fn assistant_message_entry(text: &str) -> SessionEntry {
    let data = serde_json::to_string(&MessageData {
        text: text.to_string(),
        images: Vec::new(),
    })
    .unwrap_or_else(|_| "{}".to_string());
    SessionEntry {
        id: String::new(),
        parent_id: String::new(),
        entry_type: EntryType::Message,
        role: "assistant".to_string(),
        timestamp: 0,
        data: RawValue::from_string(data).unwrap(),
    }
}

/// Creates a tool call entry.
///
/// `input` is sanitised to `{}` when empty or not valid JSON.
pub fn tool_call_entry(tool_call_id: &str, tool_name: &str, input: &[u8]) -> SessionEntry {
    let safe_input: Box<RawValue> = if input.is_empty() || !serde_json::from_slice::<serde_json::Value>(input).is_ok() {
        RawValue::from_string("{}".to_string()).unwrap()
    } else {
        RawValue::from_string(String::from_utf8_lossy(input).to_string()).unwrap()
    };

    let tc = ToolCallData {
        tool: tool_name.to_string(),
        id: tool_call_id.to_string(),
        input: safe_input,
    };
    let data = serde_json::to_string(&tc).unwrap_or_else(|_| "{}".to_string());
    SessionEntry {
        id: String::new(),
        parent_id: String::new(),
        entry_type: EntryType::ToolCall,
        role: String::new(),
        timestamp: 0,
        data: RawValue::from_string(data).unwrap(),
    }
}

/// Creates a tool result entry.
pub fn tool_result_entry(
    tool_call_id: &str,
    output: &str,
    err_msg: &str,
    images: Vec<ImageData>,
) -> SessionEntry {
    let tr = ToolResultData {
        tool_call_id: tool_call_id.to_string(),
        output: output.to_string(),
        error: err_msg.to_string(),
        is_error: !err_msg.is_empty(),
        aborted: false,
        images,
    };
    let data = serde_json::to_string(&tr).unwrap_or_else(|_| "{}".to_string());
    SessionEntry {
        id: String::new(),
        parent_id: String::new(),
        entry_type: EntryType::ToolResult,
        role: String::new(),
        timestamp: 0,
        data: RawValue::from_string(data).unwrap(),
    }
}

/// Creates a synthetic tool result for a tool call cancelled before completion.
pub fn aborted_tool_result_entry(tool_call_id: &str) -> SessionEntry {
    let tr = ToolResultData {
        tool_call_id: tool_call_id.to_string(),
        output: String::new(),
        error: "aborted by user".to_string(),
        is_error: true,
        aborted: true,
        images: Vec::new(),
    };
    let data = serde_json::to_string(&tr).unwrap_or_else(|_| "{}".to_string());
    SessionEntry {
        id: String::new(),
        parent_id: String::new(),
        entry_type: EntryType::ToolResult,
        role: String::new(),
        timestamp: 0,
        data: RawValue::from_string(data).unwrap(),
    }
}

/// Creates a new compaction entry summarizing an older range of session history.
pub fn compaction_entry(
    summary: &str,
    range_start_id: &str,
    range_end_id: &str,
    model: &str,
    tokens_before: i64,
    tokens_estimated_after: i64,
    turns_compacted: i64,
) -> SessionEntry {
    let cd = CompactionData {
        summary: summary.to_string(),
        range_start_id: range_start_id.to_string(),
        range_end_id: range_end_id.to_string(),
        model: model.to_string(),
        tokens_before,
        tokens_estimated_after,
        turns_compacted,
    };
    let data = serde_json::to_string(&cd).unwrap_or_else(|_| "{}".to_string());
    SessionEntry {
        id: String::new(),
        parent_id: String::new(),
        entry_type: EntryType::Compaction,
        role: "system".to_string(),
        timestamp: 0,
        data: RawValue::from_string(data).unwrap(),
    }
}

// ── ID generation ────────────────────────────────────────────────────────────

fn generate_id(prefix: &str) -> String {
    use rand::RngCore;
    let mut b = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut b);
    format!("{}_{}", prefix, hex::encode(b))
}

#[cfg(test)]
#[path = "session_test.rs"]
mod tests;