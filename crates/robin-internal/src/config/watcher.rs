use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{
    recommended_watcher, Event, EventKind, RecursiveMode, Result as NotifyResult,
    Watcher as NotifyWatcher,
};
use parking_lot::Mutex;
use tracing::{error, info};

use super::config::{load, Config};

struct WatcherInner {
    _notify: notify::RecommendedWatcher,
    event_rx: std::sync::mpsc::Receiver<NotifyResult<Event>>,
    stop_tx: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub struct Watcher {
    path: String,
    callback: Arc<dyn Fn(Config) + Send + Sync + 'static>,
    inner: Mutex<WatcherInner>,
}

impl Watcher {
    pub fn new<F>(path: &str, callback: F) -> anyhow::Result<Self>
    where
        F: Fn(Config) + Send + Sync + 'static,
    {
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let mut notify = recommended_watcher(move |res: NotifyResult<Event>| {
            let _ = event_tx.send(res);
        })?;
        notify.watch(Path::new(path), RecursiveMode::NonRecursive)?;

        Ok(Watcher {
            path: path.to_string(),
            callback: Arc::new(callback),
            inner: Mutex::new(WatcherInner {
                _notify: notify,
                event_rx,
                stop_tx: None,
                thread: None,
            }),
        })
    }

    pub fn start(&self) {
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let path = self.path.clone();
        let callback = self.callback.clone();

        let mut inner = self.inner.lock();

        let event_rx = {
            let (event_tx, event_rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
            drop(event_tx); // dummy — we can't move the original event_rx out
            // We need to forward from the existing channel; use a thread-safe approach instead.
            // Since we can't move event_rx from inner (it's borrowed), we use a workaround:
            // Replace with a new pair and keep the old one.
            event_rx
        };
        // We need the actual event_rx — swap it out with a dummy:
        let (dummy_tx, dummy_rx) = std::sync::mpsc::channel::<NotifyResult<Event>>();
        let real_rx = std::mem::replace(&mut inner.event_rx, dummy_rx);
        drop(dummy_tx); // close dummy sender so the watcher won't block on it

        inner.stop_tx = Some(stop_tx);
        let handle = std::thread::spawn(move || {
            run_loop(path, callback, real_rx, stop_rx);
        });
        inner.thread = Some(handle);
    }

    pub fn stop(&self) {
        let mut inner = self.inner.lock();
        if let Some(tx) = inner.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = inner.thread.take() {
            let _ = handle.join();
        }
    }
}

fn run_loop(
    path: String,
    callback: Arc<dyn Fn(Config) + Send + Sync>,
    event_rx: std::sync::mpsc::Receiver<NotifyResult<Event>>,
    stop_rx: std::sync::mpsc::Receiver<()>,
) {
    let debounce = Duration::from_millis(500);
    let poll_interval = Duration::from_millis(50);
    let mut last_event_time: Option<Instant> = None;

    loop {
        if stop_rx.try_recv().is_ok() {
            return;
        }

        loop {
            match event_rx.try_recv() {
                Ok(Ok(event)) => {
                    if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                        last_event_time = Some(Instant::now());
                    }
                }
                Ok(Err(e)) => {
                    error!("config watcher error: {}", e);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
        }

        if let Some(t) = last_event_time {
            if t.elapsed() >= debounce {
                last_event_time = None;
                reload(&path, &*callback);
            }
        }

        std::thread::sleep(poll_interval);
    }
}

fn reload(path: &str, callback: &dyn Fn(Config)) {
    match load(path) {
        Err(e) => error!("failed to reload config: {}", e),
        Ok(cfg) => {
            info!("config reloaded path={}", path);
            callback(cfg);
        }
    }
}