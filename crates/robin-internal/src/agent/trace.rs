/// trace.rs — Per-request phase timer.
///
/// Mirrors the Go trace.go but without the OpenTelemetry integration (the
/// Rust OTel SDK surface is different; OTel can be wired separately). The
/// core contract — `Mark` records phase durations, `Summary` emits the
/// aggregate — is faithfully reproduced.
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;

/// A single phase record inside a Trace.
#[derive(Debug, Clone)]
struct PhaseRecord {
    name: String,
    dur_ms: i64,
    at_ms: i64,
}

/// A callback invoked on every `Mark` call (non-blocking).
pub type OnMarkFn = Arc<dyn Fn(&str, i64, i64, &[(String, Value)]) + Send + Sync>;

struct Inner {
    last: Instant,
    phases: Vec<PhaseRecord>,
    on_mark: Option<OnMarkFn>,
}

/// Records per-request phase timings.
///
/// All methods are no-ops on a `None` value (use `Option<Arc<Trace>>`).
pub struct Trace {
    pub id: String,
    pub agent_id: String,
    pub model: String,
    pub started: Instant,
    inner: Mutex<Inner>,
}

impl Trace {
    /// Creates a new Trace stamped at the current instant.
    pub fn new(agent_id: impl Into<String>, model: impl Into<String>) -> Arc<Self> {
        let now = Instant::now();
        let id = new_trace_id();
        Arc::new(Trace {
            id,
            agent_id: agent_id.into(),
            model: model.into(),
            started: now,
            inner: Mutex::new(Inner {
                last: now,
                phases: Vec::new(),
                on_mark: None,
            }),
        })
    }

    /// Registers a callback fired on every Mark. Thread-safe; replaces any
    /// prior callback.
    pub fn set_on_mark(&self, f: OnMarkFn) {
        if let Ok(mut g) = self.inner.lock() {
            g.on_mark = Some(f);
        }
    }

    /// Records a phase boundary. `extra_attrs` is a flat list of (key, value)
    /// pairs logged alongside the phase name.
    pub fn mark(&self, phase: &str, extra_attrs: &[(String, Value)]) {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let now = Instant::now();
        let dur_ms = now.duration_since(g.last).as_millis() as i64;
        let at_ms = now.duration_since(self.started).as_millis() as i64;
        g.phases.push(PhaseRecord { name: phase.to_owned(), dur_ms, at_ms });
        g.last = now;
        let cb = g.on_mark.clone();
        drop(g); // release before calling callback

        log::info!(
            "perf trace_id={} agent={} phase={} dur_ms={} at_ms={}",
            self.id, self.agent_id, phase, dur_ms, at_ms
        );

        if let Some(f) = cb {
            f(phase, dur_ms, at_ms, extra_attrs);
        }
    }

    /// Emits a summary log line with the top-3 slowest phases.
    pub fn summary(&self) {
        let g = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let total_ms = self.started.elapsed().as_millis() as i64;
        // Aggregate dur by phase name.
        let mut agg: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for p in &g.phases {
            *agg.entry(p.name.clone()).or_default() += p.dur_ms;
        }
        drop(g);

        let mut flat: Vec<(&str, i64)> = agg.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        // Sort desc
        flat.sort_by(|a, b| b.1.cmp(&a.1));
        flat.truncate(3);

        let mut summary = format!(
            "perf summary trace_id={} agent={} model={} total_ms={}",
            self.id, self.agent_id, self.model, total_ms
        );
        for (i, (name, dur)) in flat.iter().enumerate() {
            summary.push_str(&format!(" top{}_phase={} top{}_ms={}", i + 1, name, i + 1, dur));
        }
        log::info!("{}", summary);
    }
}

/// Key used to store a `Trace` in a request-scoped `HashMap<TypeId, Box<dyn Any>>`.
/// Because Rust doesn't have `context.Context`, we pass `Option<Arc<Trace>>`
/// explicitly through function arguments (same as the generated call-sites do).
///
/// This module provides a thin newtype so callers can write:
///   `let tr = TraceHandle::from(ctx_trace.as_deref());`
pub struct TraceHandle(pub Option<Arc<Trace>>);

impl TraceHandle {
    pub fn mark(&self, phase: &str) {
        if let Some(t) = &self.0 {
            t.mark(phase, &[]);
        }
    }

    pub fn mark_with(&self, phase: &str, attrs: &[(String, Value)]) {
        if let Some(t) = &self.0 {
            t.mark(phase, attrs);
        }
    }

    pub fn summary(&self) {
        if let Some(t) = &self.0 {
            t.summary();
        }
    }
}

impl From<Option<Arc<Trace>>> for TraceHandle {
    fn from(t: Option<Arc<Trace>>) -> Self {
        TraceHandle(t)
    }
}

pub fn new_trace_id() -> String {
    use rand::Rng;
    let mut buf = [0u8; 4];
    rand::thread_rng().fill(&mut buf);
    hex::encode(buf)
}