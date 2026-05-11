use std::time::Duration;

use serde_json::Value;

use super::websearch::SearchResult;

/// URL-encode a query string (percent-encoding for query parameters).
fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

const SEARCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Strategy interface used by `WebSearchTool`.
pub trait WebSearchBackend: Send + Sync {
    fn name(&self) -> &str;
    fn search(&self, query: &str, max_results: usize) -> anyhow::Result<Vec<SearchResult>>;
}

// ── DuckDuckGo backend ────────────────────────────────────────────────────────

pub struct DdgBackend;

pub fn new_ddg_backend() -> Box<dyn WebSearchBackend> {
    Box::new(DdgBackend)
}

impl WebSearchBackend for DdgBackend {
    fn name(&self) -> &str { "duckduckgo" }

    fn search(&self, query: &str, max_results: usize) -> anyhow::Result<Vec<SearchResult>> {
        duck_duck_go_search(query, max_results)
    }
}

fn duck_duck_go_search(query: &str, max_results: usize) -> anyhow::Result<Vec<SearchResult>> {
    let search_url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        url_encode(query)
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(SEARCH_TIMEOUT)
        .user_agent("Robin/1.0 (AI Agent Gateway)")
        .build()?;
    let resp = client.get(&search_url).send()?;
    if !resp.status().is_success() {
        anyhow::bail!("search returned HTTP {}", resp.status().as_u16());
    }
    let body = resp.text()?;
    Ok(parse_ddg_results(&body, max_results))
}

fn parse_ddg_results(html: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut remaining = html;

    while results.len() < max_results {
        let link_idx = match remaining.find(r#"class="result__a""#) {
            Some(i) => i,
            None => break,
        };
        remaining = &remaining[link_idx..];

        let href = extract_attr(remaining, "href");
        let title = extract_tag_text(remaining, "a");

        let snippet = remaining.find(r#"class="result__snippet""#).map(|idx| {
            extract_tag_text(&remaining[idx..], "a")
        }).unwrap_or_default();

        let clean_url = clean_ddg_url(&href);

        if !clean_url.is_empty() && !title.is_empty() {
            results.push(SearchResult {
                title: clean_html_text(&title),
                url: clean_url,
                snippet: clean_html_text(&snippet),
            });
        }

        let next_idx = match remaining[1..].find(r#"class="result__a""#) {
            Some(i) => i + 1,
            None => break,
        };
        remaining = &remaining[next_idx..];
    }

    results
}

fn extract_attr(html: &str, attr: &str) -> String {
    let needle = format!("{}=\"", attr);
    let idx = match html.find(&needle) {
        Some(i) => i + needle.len(),
        None => return String::new(),
    };
    let end = match html[idx..].find('"') {
        Some(i) => idx + i,
        None => return String::new(),
    };
    html[idx..end].to_owned()
}

fn extract_tag_text(html: &str, tag: &str) -> String {
    let start = match html.find('>') {
        Some(i) => i + 1,
        None => return String::new(),
    };
    let end_tag = format!("</{}>", tag);
    let end = match html[start..].find(&end_tag) {
        Some(i) => start + i,
        None => return String::new(),
    };
    html[start..end].to_owned()
}

fn clean_ddg_url(raw_url: &str) -> String {
    if raw_url.contains("uddg=") {
        if let Ok(u) = url::Url::parse(raw_url) {
            if let Some(uddg) = u.query_pairs().find(|(k, _)| k == "uddg").map(|(_, v)| v.into_owned()) {
                if !uddg.is_empty() {
                    return uddg;
                }
            }
        }
    }
    if raw_url.starts_with("//") {
        return format!("https:{}", raw_url);
    }
    raw_url.to_owned()
}

pub fn clean_html_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out = out.replace("&amp;", "&");
    out = out.replace("&lt;", "<");
    out = out.replace("&gt;", ">");
    out = out.replace("&quot;", "\"");
    out = out.replace("&#x27;", "'");
    out = out.replace("&nbsp;", " ");
    out.trim().to_owned()
}

// ── Brave backend ─────────────────────────────────────────────────────────────

pub struct BraveBackend {
    api_key: String,
}

pub fn new_brave_backend(api_key: String) -> Box<dyn WebSearchBackend> {
    Box::new(BraveBackend { api_key })
}

impl WebSearchBackend for BraveBackend {
    fn name(&self) -> &str { "brave" }

    fn search(&self, query: &str, max_results: usize) -> anyhow::Result<Vec<SearchResult>> {
        if self.api_key.is_empty() {
            anyhow::bail!("brave backend: no API key configured");
        }
        let n = max_results.min(20);
        let u = format!(
            "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
            url_encode(query),
            n
        );
        let client = reqwest::blocking::Client::builder()
            .timeout(SEARCH_TIMEOUT)
            .build()?;
        let resp = client
            .get(&u)
            .header("Accept", "application/json")
            .header("X-Subscription-Token", &self.api_key)
            .send()?;
        if !resp.status().is_success() {
            let status_code = resp.status().as_u16();
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("brave returned HTTP {}: {}", status_code, body.trim());
        }
        let parsed: serde_json::Value = resp.json()?;
        let results = parsed["web"]["results"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .take(n)
                    .map(|r| SearchResult {
                        title: clean_html_text(r["title"].as_str().unwrap_or("")),
                        url: r["url"].as_str().unwrap_or("").to_owned(),
                        snippet: clean_html_text(r["description"].as_str().unwrap_or("")),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(results)
    }
}

// ── Tavily backend ────────────────────────────────────────────────────────────

pub struct TavilyBackend {
    api_key: String,
}

pub fn new_tavily_backend(api_key: String) -> Box<dyn WebSearchBackend> {
    Box::new(TavilyBackend { api_key })
}

impl WebSearchBackend for TavilyBackend {
    fn name(&self) -> &str { "tavily" }

    fn search(&self, query: &str, max_results: usize) -> anyhow::Result<Vec<SearchResult>> {
        if self.api_key.is_empty() {
            anyhow::bail!("tavily backend: no API key configured");
        }
        let n = max_results.min(20);
        let body = serde_json::json!({
            "api_key": self.api_key,
            "query": query,
            "max_results": n
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(SEARCH_TIMEOUT)
            .build()?;
        let resp = client
            .post("https://api.tavily.com/search")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()?;
        if !resp.status().is_success() {
            let text = resp.text().unwrap_or_default();
            anyhow::bail!("tavily returned HTTP: {}", text.trim());
        }
        let parsed: serde_json::Value = resp.json()?;
        let results = parsed["results"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|r| SearchResult {
                        title: r["title"].as_str().unwrap_or("").to_owned(),
                        url: r["url"].as_str().unwrap_or("").to_owned(),
                        snippet: r["content"].as_str().unwrap_or("").to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(results)
    }
}

// ── SearXNG backend ───────────────────────────────────────────────────────────

pub struct SearxngBackend {
    base_url: String,
}

pub fn new_searxng_backend(base_url: String) -> Box<dyn WebSearchBackend> {
    Box::new(SearxngBackend {
        base_url: base_url.trim_end_matches('/').to_owned(),
    })
}

impl WebSearchBackend for SearxngBackend {
    fn name(&self) -> &str { "searxng" }

    fn search(&self, query: &str, max_results: usize) -> anyhow::Result<Vec<SearchResult>> {
        if self.base_url.is_empty() {
            anyhow::bail!("searxng backend: no base URL configured");
        }
        let u = format!(
            "{}/search?format=json&q={}",
            self.base_url,
            url_encode(query)
        );
        let client = reqwest::blocking::Client::builder()
            .timeout(SEARCH_TIMEOUT)
            .build()?;
        let resp = client.get(&u).send()?;
        if !resp.status().is_success() {
            anyhow::bail!("searxng returned HTTP {}", resp.status().as_u16());
        }
        let parsed: serde_json::Value = resp.json()?;
        let results = parsed["results"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .take(max_results)
                    .map(|r| SearchResult {
                        title: r["title"].as_str().unwrap_or("").to_owned(),
                        url: r["url"].as_str().unwrap_or("").to_owned(),
                        snippet: r["content"].as_str().unwrap_or("").to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(results)
    }
}

// ── Factory ───────────────────────────────────────────────────────────────────

/// Configuration for selecting a web search backend.
#[derive(Debug, Clone, Default)]
pub struct WebSearchConfig {
    /// "duckduckgo" (default) | "brave" | "tavily" | "searxng"
    pub backend: String,
    /// API key for Brave or Tavily.
    pub api_key: String,
    /// Base URL for SearXNG.
    pub base_url: String,
}

/// Resolves a `WebSearchConfig` into a concrete backend.
/// Returns the DDG fallback when the config is empty or names an unknown backend.
pub fn new_web_search_backend(cfg: WebSearchConfig) -> Box<dyn WebSearchBackend> {
    match cfg.backend.to_lowercase().trim() {
        "" | "duckduckgo" | "ddg" => new_ddg_backend(),
        "brave" => new_brave_backend(cfg.api_key),
        "tavily" => new_tavily_backend(cfg.api_key),
        "searxng" => new_searxng_backend(cfg.base_url),
        _ => new_ddg_backend(),
    }
}