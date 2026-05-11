use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::Value;

use crate::llm::{ImageContent, ToolDef};


// ── Path helpers ──────────────────────────────────────────────────────────────

/// Rewrites a leading "~" or "~/" in `p` to the user's home directory.
pub fn expand_home(p: &str) -> String {
    if p == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().into_owned();
        }
        return p.to_owned();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    p.to_owned()
}

/// Returns true if `s` contains any Unicode whitespace or invisible characters
/// that `sanitize_llm_text` would normalize away.
pub fn has_unicode_whitespace(s: &str) -> bool {
    for c in s.chars() {
        match c {
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}'
            | '\u{2028}' | '\u{2029}' => return true,
            _ => {}
        }
        if c != ' ' && c != '\t' && c != '\n' && c != '\r' && c.is_whitespace() {
            return true;
        }
    }
    false
}

/// Normalizes Unicode whitespace lookalikes (NBSP, narrow NBSP, ideographic
/// space, en/em space, etc.) to ASCII space, and strips zero-width characters.
pub fn sanitize_llm_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            // Zero-width / BOM: drop
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' => {}
            // Line/paragraph separator → newline
            '\u{2028}' | '\u{2029}' => out.push('\n'),
            _ => {
                if c != ' ' && c != '\t' && c != '\n' && c != '\r' && c.is_whitespace() {
                    out.push(' ');
                } else {
                    out.push(c);
                }
            }
        }
    }
    out
}

/// Resolves a path that may not exist on disk by recovering from
/// Unicode-whitespace mismatches. Resolution order:
/// 1. The path as given.
/// 2. The Unicode-sanitized variant.
/// 3. A directory scan for an entry whose sanitized name matches.
///    (only if exactly one match, to avoid ambiguity)
pub fn resolve_existing_path(p: &str) -> String {
    if Path::new(p).exists() {
        return p.to_owned();
    }
    let alt = sanitize_llm_text(p);
    if alt != p && Path::new(&alt).exists() {
        return alt;
    }
    // Try directory scan
    let path = Path::new(p);
    let (dir, base) = match (path.parent(), path.file_name()) {
        (Some(d), Some(b)) => (d, b.to_string_lossy().into_owned()),
        _ => return p.to_owned(),
    };
    let dir = if dir == Path::new("") {
        Path::new(".")
    } else {
        dir
    };
    let target = sanitize_llm_text(&base);
    if let Ok(entries) = std::fs::read_dir(dir) {
        let matches: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| sanitize_llm_text(&e.file_name().to_string_lossy()) == target)
            .collect();
        if matches.len() == 1 {
            return dir.join(matches[0].file_name()).to_string_lossy().into_owned();
        }
    }
    p.to_owned()
}

/// Like `resolve_existing_path` but the dir-scan fallback only fires when the
/// matched on-disk entry actually contains non-ASCII whitespace. Used by the
/// bash tool.
pub fn resolve_existing_path_strict(p: &str) -> String {
    if Path::new(p).exists() {
        return p.to_owned();
    }
    let alt = sanitize_llm_text(p);
    if alt != p && Path::new(&alt).exists() {
        return alt;
    }
    let path = Path::new(p);
    let (dir, base) = match (path.parent(), path.file_name()) {
        (Some(d), Some(b)) => (d, b.to_string_lossy().into_owned()),
        _ => return p.to_owned(),
    };
    let dir = if dir == Path::new("") {
        Path::new(".")
    } else {
        dir
    };
    let target = sanitize_llm_text(&base);
    if let Ok(entries) = std::fs::read_dir(dir) {
        let matches: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                sanitize_llm_text(&name) == target && has_unicode_whitespace(&name)
            })
            .collect();
        if matches.len() == 1 {
            return dir.join(matches[0].file_name()).to_string_lossy().into_owned();
        }
    }
    p.to_owned()
}

/// Ensures the resolved path is within the workspace directory.
/// Resolves symlinks to prevent traversal attacks.
pub fn validate_path_in_work_dir(path: &str, work_dir: &str) -> anyhow::Result<()> {
    if work_dir.is_empty() {
        return Ok(());
    }
    let abs_work = std::fs::canonicalize(work_dir)
        .unwrap_or_else(|_| PathBuf::from(work_dir).canonicalize().unwrap_or_else(|_| PathBuf::from(work_dir)));

    let abs_path = {
        let p = PathBuf::from(path);
        if p.is_absolute() {
            p
        } else {
            std::env::current_dir().unwrap_or_default().join(p)
        }
    };

    let real_work = std::fs::canonicalize(&abs_work).unwrap_or(abs_work.clone());

    // For the target path, resolve the parent directory (file may not exist yet)
    let parent_dir = abs_path.parent().unwrap_or(&abs_path);
    let real_parent = std::fs::canonicalize(parent_dir).unwrap_or_else(|_| parent_dir.to_path_buf());
    let real_path = real_parent.join(abs_path.file_name().unwrap_or_default());

    let real_work_str = real_work.to_string_lossy();
    let real_path_str = real_path.to_string_lossy();

    let separator = std::path::MAIN_SEPARATOR.to_string();
    if !real_path_str.starts_with(&format!("{}{}", real_work_str, separator))
        && real_path_str != real_work_str
    {
        anyhow::bail!("path {:?} is outside workspace {:?}", path, work_dir);
    }
    Ok(())
}

// ── Tool trait ────────────────────────────────────────────────────────────────

/// The result of executing a tool.
#[derive(Debug, Clone, Default)]
pub struct ToolResult {
    pub output: String,
    pub error: String,
    pub metadata: Option<serde_json::Map<String, Value>>,
    /// Image attachments (not JSON-serialized in standard output).
    pub images: Vec<ImageContent>,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self { output: output.into(), ..Default::default() }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self { error: error.into(), ..Default::default() }
    }

    pub fn with_metadata(mut self, meta: serde_json::Map<String, Value>) -> Self {
        self.metadata = Some(meta);
        self
    }
}

/// The Tool trait — all Robin tools implement this.
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// Returns the JSON Schema for this tool's parameters.
    fn parameters(&self) -> Value;
    /// Execute the tool with the given JSON input.
    fn execute(&self, input: Value) -> anyhow::Result<ToolResult>;
    /// Whether this tool can be invoked in parallel with other concurrency-safe
    /// tools. Pure-read tools return true; anything that mutates state returns false.
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }
}

// ── Executor trait ────────────────────────────────────────────────────────────

/// Used by the agent runtime for tool operations.
pub trait Executor: Send + Sync {
    fn execute(&self, name: &str, input: Value) -> anyhow::Result<ToolResult>;
    fn tool_defs(&self) -> Vec<ToolDef>;
    fn names(&self) -> Vec<String>;
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>>;
}

// ── Registry ─────────────────────────────────────────────────────────────────

/// Manages a collection of available tools.
pub struct Registry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
}

impl Registry {
    pub fn new() -> Self {
        Self { tools: RwLock::new(HashMap::new()) }
    }

    /// Register a tool.
    pub fn register(&self, tool: impl Tool + 'static) {
        let name = tool.name().to_owned();
        self.tools.write().insert(name, Arc::new(tool));
    }

    /// Register an Arc<dyn Tool> directly.
    pub fn register_arc(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_owned();
        self.tools.write().insert(name, tool);
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.read().get(name).cloned()
    }

    /// Execute a tool by name.
    pub fn execute(&self, name: &str, input: Value) -> anyhow::Result<ToolResult> {
        let tool = self
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown tool: {:?}", name))?;
        tool.execute(input)
    }

    /// Returns sorted tool definitions for the LLM API.
    /// Output is sorted by name for prompt-cache stability.
    pub fn tool_defs(&self) -> Vec<ToolDef> {
        let guard = self.tools.read();
        let mut names: Vec<&str> = guard.keys().map(|s| s.as_str()).collect();
        names.sort_unstable();
        names
            .into_iter()
            .map(|name| {
                let t = &guard[name];
                ToolDef {
                    name: t.name().to_owned(),
                    description: t.description().to_owned(),
                    parameters: t.parameters(),
                }
            })
            .collect()
    }

    /// Returns sorted tool names.
    pub fn names(&self) -> Vec<String> {
        let guard = self.tools.read();
        let mut names: Vec<String> = guard.keys().cloned().collect();
        names.sort_unstable();
        names
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor for Registry {
    fn execute(&self, name: &str, input: Value) -> anyhow::Result<ToolResult> {
        self.execute(name, input)
    }

    fn tool_defs(&self) -> Vec<ToolDef> {
        self.tool_defs()
    }

    fn names(&self) -> Vec<String> {
        self.names()
    }

    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.get(name)
    }
}

// ── NoopExecutor ──────────────────────────────────────────────────────────────

/// A do-nothing Executor used as a placeholder when no tools are configured
/// (e.g., in unit tests that don't exercise tool dispatch).
pub struct NoopExecutor;

impl Executor for NoopExecutor {
    fn execute(&self, name: &str, _input: Value) -> anyhow::Result<ToolResult> {
        anyhow::bail!("NoopExecutor: no tools registered (attempted: {:?})", name)
    }

    fn tool_defs(&self) -> Vec<ToolDef> {
        vec![]
    }

    fn names(&self) -> Vec<String> {
        vec![]
    }

    fn get(&self, _name: &str) -> Option<Arc<dyn Tool>> {
        None
    }
}