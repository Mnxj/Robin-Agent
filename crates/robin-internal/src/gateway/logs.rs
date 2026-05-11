use axum::{
    response::{Html, IntoResponse, Response, Sse},
    response::sse::{Event, KeepAlive},
};
use parking_lot::RwLock;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;
use tracing::Level;

/// A single captured log record.
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub time: SystemTime,
    pub level: String,
    pub message: String,
    pub attrs: String, // pre-formatted key=value pairs
}

impl LogEntry {
    /// Format as a text line: "HH:MM:SS.mmm LEVEL message attrs"
    pub fn format(&self) -> String {
        let duration = self
            .time
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = duration.as_secs();
        let millis = duration.subsec_millis();
        let h = (secs / 3600) % 24;
        let m = (secs / 60) % 60;
        let s = secs % 60;

        let ts = format!("{:02}:{:02}:{:02}.{:03}", h, m, s, millis);
        let mut line = format!("{} {} {}", ts, self.level, self.message);
        if !self.attrs.is_empty() {
            line.push(' ');
            line.push_str(&self.attrs);
        }
        line
    }
}

/// LogBuffer captures log records into a ring buffer and supports
/// streaming new entries to subscribers via Server-Sent Events.
#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<LogBufferInner>,
}

struct LogBufferInner {
    entries: RwLock<Vec<Option<LogEntry>>>,
    head: AtomicUsize,
    count: AtomicUsize,
    capacity: usize,
    sender: broadcast::Sender<LogEntry>,
}

impl LogBuffer {
    /// Creates a log buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(256);
        LogBuffer {
            inner: Arc::new(LogBufferInner {
                entries: RwLock::new(vec![None; capacity]),
                head: AtomicUsize::new(0),
                count: AtomicUsize::new(0),
                capacity,
                sender,
            }),
        }
    }

    /// Add a log entry to the ring buffer.
    pub fn add(&self, entry: LogEntry) {
        let inner = &self.inner;
        let mut entries = inner.entries.write();
        let head = inner.head.load(Ordering::Relaxed);
        entries[head] = Some(entry.clone());
        inner.head.store((head + 1) % inner.capacity, Ordering::Relaxed);
        let count = inner.count.load(Ordering::Relaxed);
        if count < inner.capacity {
            inner.count.store(count + 1, Ordering::Relaxed);
        }
        drop(entries);

        // Notify subscribers (broadcast — if no receivers, ignore the error)
        let _ = inner.sender.send(entry);
    }

    /// Returns all current entries in chronological order.
    pub fn snapshot(&self) -> Vec<LogEntry> {
        let inner = &self.inner;
        let entries = inner.entries.read();
        let count = inner.count.load(Ordering::Relaxed);
        let head = inner.head.load(Ordering::Relaxed);
        let start = (head + inner.capacity - count) % inner.capacity;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            if let Some(e) = &entries[(start + i) % inner.capacity] {
                out.push(e.clone());
            }
        }
        out
    }

    /// Subscribe to new log entries. Returns a broadcast receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.inner.sender.subscribe()
    }
}

/// A tracing layer that forwards records to a LogBuffer.
pub struct LogBufferLayer {
    buf: LogBuffer,
}

impl LogBufferLayer {
    pub fn new(buf: LogBuffer) -> Self {
        LogBufferLayer { buf }
    }
}

impl<S> tracing_subscriber::Layer<S> for LogBufferLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        use tracing::field::{Field, Visit};

        struct AttrCollector {
            message: String,
            attrs: Vec<String>,
        }

        impl Visit for AttrCollector {
            fn record_str(&mut self, field: &Field, value: &str) {
                if field.name() == "message" {
                    self.message = value.to_string();
                } else {
                    self.attrs.push(format!("{}={}", field.name(), value));
                }
            }
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.message = format!("{:?}", value);
                } else {
                    self.attrs.push(format!("{}={:?}", field.name(), value));
                }
            }
        }

        let level = match *event.metadata().level() {
            Level::ERROR => "ERROR",
            Level::WARN => "WARN",
            Level::INFO => "INFO",
            Level::DEBUG => "DEBUG",
            Level::TRACE => "TRACE",
        };

        let mut collector = AttrCollector {
            message: String::new(),
            attrs: Vec::new(),
        };
        event.record(&mut collector);

        let entry = LogEntry {
            time: SystemTime::now(),
            level: level.to_string(),
            message: collector.message,
            attrs: collector.attrs.join(" "),
        };
        self.buf.add(entry);
    }
}

/// Returns an axum handler for the /logs page (HTML).
pub async fn logs_page_handler() -> Html<&'static str> {
    Html(LOGS_HTML)
}

/// Returns an SSE stream of log entries from the buffer.
pub async fn logs_stream_handler(
    axum::extract::State(buf): axum::extract::State<LogBuffer>,
) -> Response {
    use tokio_stream::wrappers::BroadcastStream;
    use tokio_stream::StreamExt;

    // Build the initial snapshot + live stream
    let snapshot = buf.snapshot();
    let receiver = buf.subscribe();

    let initial = futures::stream::iter(snapshot.into_iter().map(|e| {
        Ok::<Event, std::convert::Infallible>(
            Event::default().data(e.format()),
        )
    }));

    let live = BroadcastStream::new(receiver).filter_map(|r| {
        r.ok().map(|e| Ok::<Event, std::convert::Infallible>(Event::default().data(e.format())))
    });

    let combined = initial.chain(live);

    Sse::new(combined)
        .keep_alive(KeepAlive::default())
        .into_response()
}

const LOGS_HTML: &str = include_str!("logs_template.html");