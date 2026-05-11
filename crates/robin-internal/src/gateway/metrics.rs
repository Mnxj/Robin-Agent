use axum::{
    http::header,
    response::Response,
};
use parking_lot::RwLock;
use std::{
    collections::HashMap,
    fmt::Write as FmtWrite,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Instant,
};

/// Metrics collects gateway operational metrics in Prometheus text format.
pub struct Metrics {
    requests_total: AtomicI64,
    ws_connections: AtomicI64,
    ws_messages_total: AtomicI64,
    tool_calls_total: AtomicI64,
    llm_calls_total: AtomicI64,
    errors_total: AtomicI64,
    start_time: Instant,

    tool_counts: RwLock<HashMap<String, AtomicI64>>,
}

impl Metrics {
    /// Creates a new metrics collector.
    pub fn new() -> Self {
        Metrics {
            requests_total: AtomicI64::new(0),
            ws_connections: AtomicI64::new(0),
            ws_messages_total: AtomicI64::new(0),
            tool_calls_total: AtomicI64::new(0),
            llm_calls_total: AtomicI64::new(0),
            errors_total: AtomicI64::new(0),
            start_time: Instant::now(),
            tool_counts: RwLock::new(HashMap::new()),
        }
    }

    /// Increments the HTTP request counter.
    pub fn inc_requests(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the active WebSocket connection counter.
    pub fn inc_ws_connections(&self) {
        self.ws_connections.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrements the active WebSocket connection counter.
    pub fn dec_ws_connections(&self) {
        self.ws_connections.fetch_sub(1, Ordering::Relaxed);
    }

    /// Increments the WebSocket message counter.
    pub fn inc_ws_messages(&self) {
        self.ws_messages_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the tool call counter for a specific tool.
    pub fn inc_tool_calls(&self, tool_name: &str) {
        self.tool_calls_total.fetch_add(1, Ordering::Relaxed);

        // Check if counter exists under read lock
        {
            let counts = self.tool_counts.read();
            if let Some(counter) = counts.get(tool_name) {
                counter.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }

        // Insert under write lock
        let mut counts = self.tool_counts.write();
        // Double-check after acquiring write lock
        if let Some(counter) = counts.get(tool_name) {
            counter.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let counter = AtomicI64::new(1);
        counts.insert(tool_name.to_string(), counter);
    }

    /// Increments the LLM call counter.
    pub fn inc_llm_calls(&self) {
        self.llm_calls_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the error counter.
    pub fn inc_errors(&self) {
        self.errors_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns an axum handler that serves Prometheus-compatible metrics.
    pub fn handler(self: Arc<Self>) -> impl Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>> + Clone + Send + 'static {
        move || {
            let m = self.clone();
            Box::pin(async move {
                let mut b = String::new();

                let uptime = m.start_time.elapsed().as_secs_f64();
                let _ = writeln!(b, "# HELP robin_uptime_seconds Time since gateway started.");
                let _ = writeln!(b, "# TYPE robin_uptime_seconds gauge");
                let _ = writeln!(b, "robin_uptime_seconds {:.1}\n", uptime);

                let _ = writeln!(b, "# HELP robin_http_requests_total Total HTTP requests.");
                let _ = writeln!(b, "# TYPE robin_http_requests_total counter");
                let _ = writeln!(b, "robin_http_requests_total {}\n", m.requests_total.load(Ordering::Relaxed));

                let _ = writeln!(b, "# HELP robin_ws_connections_active Active WebSocket connections.");
                let _ = writeln!(b, "# TYPE robin_ws_connections_active gauge");
                let _ = writeln!(b, "robin_ws_connections_active {}\n", m.ws_connections.load(Ordering::Relaxed));

                let _ = writeln!(b, "# HELP robin_ws_messages_total Total WebSocket messages received.");
                let _ = writeln!(b, "# TYPE robin_ws_messages_total counter");
                let _ = writeln!(b, "robin_ws_messages_total {}\n", m.ws_messages_total.load(Ordering::Relaxed));

                let _ = writeln!(b, "# HELP robin_tool_calls_total Total tool calls.");
                let _ = writeln!(b, "# TYPE robin_tool_calls_total counter");
                let _ = writeln!(b, "robin_tool_calls_total {}\n", m.tool_calls_total.load(Ordering::Relaxed));

                let _ = writeln!(b, "# HELP robin_llm_calls_total Total LLM API calls.");
                let _ = writeln!(b, "# TYPE robin_llm_calls_total counter");
                let _ = writeln!(b, "robin_llm_calls_total {}\n", m.llm_calls_total.load(Ordering::Relaxed));

                let _ = writeln!(b, "# HELP robin_errors_total Total errors.");
                let _ = writeln!(b, "# TYPE robin_errors_total counter");
                let _ = writeln!(b, "robin_errors_total {}\n", m.errors_total.load(Ordering::Relaxed));

                // Per-tool breakdown (sorted for deterministic output)
                let counts = m.tool_counts.read();
                if !counts.is_empty() {
                    let _ = writeln!(b, "# HELP robin_tool_calls_by_tool Tool calls by tool name.");
                    let _ = writeln!(b, "# TYPE robin_tool_calls_by_tool counter");

                    let mut names: Vec<&String> = counts.keys().collect();
                    names.sort();
                    for name in names {
                        let count = counts[name].load(Ordering::Relaxed);
                        let _ = writeln!(b, "robin_tool_calls_by_tool{{tool=\"{}\"}} {}", name, count);
                    }
                }

                axum::response::Response::builder()
                    .status(200)
                    .header(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")
                    .body(axum::body::Body::from(b))
                    .unwrap()
            })
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "metrics_test.rs"]
mod metrics_test;