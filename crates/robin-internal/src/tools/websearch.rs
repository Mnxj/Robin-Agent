use parking_lot::Mutex;
use serde_json::Value;

use super::tool::{Tool, ToolResult};
use super::websearch_backends::{new_ddg_backend, WebSearchBackend};

/// A search result returned by a backend.
#[derive(Debug, Clone, Default)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Searches the web and returns results via a configurable backend.
pub struct WebSearchTool {
    backend: Mutex<Option<Box<dyn WebSearchBackend>>>,
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self { backend: Mutex::new(None) }
    }
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_backend(backend: Box<dyn WebSearchBackend>) -> Self {
        Self { backend: Mutex::new(Some(backend)) }
    }

    /// Swaps the active backend. Called from the config hot-reload path.
    pub fn set_backend(&self, backend: Box<dyn WebSearchBackend>) {
        *self.backend.lock() = Some(backend);
    }
}

impl Tool for WebSearchTool {
    fn name(&self) -> &str { "web_search" }

    fn description(&self) -> &str {
        "Search the web for information. Returns search results with titles, URLs, and snippets. Use this when you need current information, documentation, or to find web resources."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5, max: 10)"
                }
            },
            "required": ["query"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { true }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.is_empty() => q.to_owned(),
            _ => return Ok(ToolResult::err("query is required")),
        };

        let mut max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;
        if max_results == 0 {
            max_results = 5;
        }
        if max_results > 10 {
            max_results = 10;
        }

        // Use the configured backend or fall back to DDG
        let results = {
            let mut guard = self.backend.lock();
            let backend: &dyn WebSearchBackend = match guard.as_ref() {
                Some(b) => b.as_ref(),
                None => {
                    // Insert DDG as default
                    *guard = Some(new_ddg_backend());
                    guard.as_ref().unwrap().as_ref()
                }
            };
            backend.search(&query, max_results).map_err(|e| {
                anyhow::anyhow!("search failed ({}): {}", backend.name(), e)
            })?
        };

        if results.is_empty() {
            return Ok(ToolResult::ok(format!("No results found for: {}", query)));
        }

        let mut sb = String::new();
        for (i, r) in results.iter().enumerate() {
            sb.push_str(&format!(
                "{}. **{}**\n   {}\n   {}\n\n",
                i + 1,
                r.title,
                r.url,
                r.snippet
            ));
        }

        let mut meta = serde_json::Map::new();
        meta.insert("query".to_owned(), Value::String(query));
        meta.insert("num_results".to_owned(), Value::Number(results.len().into()));

        Ok(ToolResult { output: sb, metadata: Some(meta), ..Default::default() })
    }
}