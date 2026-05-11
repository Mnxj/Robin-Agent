use std::sync::Arc;

use parking_lot::Mutex;

use crate::llm::{Message, ToolDef};

const PER_MESSAGE_OVERHEAD: usize = 3;
const DEFAULT_UNKNOWN_WINDOW: usize = 128000;

/// Estimate token count for an LLM payload using the chars/4 heuristic.
pub fn estimate(msgs: &[Message], system_prompt: &str, tools: &[ToolDef]) -> usize {
    let mut total = system_prompt.len();
    for m in msgs {
        total += m.role.len() + m.content.len() + m.tool_call_id.len() + PER_MESSAGE_OVERHEAD;
        for tc in &m.tool_calls {
            total += tc.id.len() + tc.name.len() + tc.input.to_string().len();
        }
    }
    for t in tools {
        total += t.name.len() + t.description.len() + t.parameters.to_string().len();
    }
    total / 4
}

/// Returns the context window size for a model string ("provider/model").
pub fn context_window(model: &str) -> usize {
    if model.is_empty() {
        return DEFAULT_UNKNOWN_WINDOW;
    }
    let (_, model_id) = split_provider_model(model);

    if let Some(w) = window_by_model_family(model_id) {
        return w;
    }

    DEFAULT_UNKNOWN_WINDOW
}

/// Returns the effective context window, honoring a per-agent override when > 0.
pub fn context_window_for(model: &str, override_val: i64) -> usize {
    if override_val > 0 {
        return override_val as usize;
    }
    context_window(model)
}

fn window_by_model_family(model_id: &str) -> Option<usize> {
    let id = model_id.to_lowercase();
    let leaf = id.rfind('/').map(|i| &id[i + 1..]).unwrap_or(&id);
    if id.contains("claude") { return Some(200000); }
    if leaf.starts_with("gpt-4o") || leaf.starts_with("gpt-4-turbo") { return Some(128000); }
    if leaf.starts_with("gpt-4") { return Some(8192); }
    if leaf.starts_with("gpt-3.5") { return Some(16385); }
    if id.contains("gemini-1.5-pro") { return Some(2000000); }
    if id.contains("gemini-1.5-flash") || id.contains("gemini-2") { return Some(1000000); }
    None
}

fn split_provider_model(s: &str) -> (&str, &str) {
    match s.find('/') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => ("", s),
    }
}


/// Self-calibrating token estimator. Learns the actual/estimated ratio per session.
pub struct Calibrator {
    mu: Mutex<CalibratorState>,
}

struct CalibratorState {
    ratio: f64,
    count: usize,
}

impl Calibrator {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            mu: Mutex::new(CalibratorState { ratio: 1.0, count: 0 }),
        })
    }

    pub fn update(&self, actual: usize, estimated: usize) {
        if actual == 0 || estimated == 0 { return; }
        let mut s = self.mu.lock();
        let sample = actual as f64 / estimated as f64;
        s.count += 1;
        s.ratio += (sample - s.ratio) / s.count as f64;
    }

    pub fn adjust(&self, estimated: usize) -> usize {
        let s = self.mu.lock();
        (estimated as f64 * s.ratio) as usize
    }

    pub fn snapshot(&self) -> (f64, usize) {
        let s = self.mu.lock();
        (s.ratio, s.count)
    }

    pub fn restore(&self, ratio: f64, count: usize) {
        if ratio <= 0.0 { return; }
        let mut s = self.mu.lock();
        s.ratio = ratio;
        s.count = count;
    }
}

impl Default for Calibrator {
    fn default() -> Self {
        Self { mu: Mutex::new(CalibratorState { ratio: 1.0, count: 0 }) }
    }
}