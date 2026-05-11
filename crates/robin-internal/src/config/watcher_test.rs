#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use super::super::watcher::Watcher;

    #[test]
    fn test_watcher_detects_changes() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("robin.json5");

        let initial_config = r#"{
            "gateway": {"host": "127.0.0.1", "port": 18789},
            "agents": {"list": [{"id": "default", "name": "Test", "model": "openai/gpt-4o"}]}
        }"#;
        std::fs::write(&cfg_path, initial_config).unwrap();

        let callback_fired = Arc::new(AtomicI32::new(0));
        let cb_clone = callback_fired.clone();

        let w = Watcher::new(cfg_path.to_str().unwrap(), move |_cfg| {
            cb_clone.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        w.start();

        std::thread::sleep(Duration::from_millis(100));

        let updated_config = r#"{
            "gateway": {"host": "127.0.0.1", "port": 19000},
            "agents": {"list": [{"id": "default", "name": "Updated", "model": "openai/gpt-4o"}]}
        }"#;
        std::fs::write(&cfg_path, updated_config).unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if callback_fired.load(Ordering::SeqCst) > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            callback_fired.load(Ordering::SeqCst) > 0,
            "callback should fire after file change"
        );

        w.stop();
    }

    #[test]
    fn test_watcher_stop_does_not_hang() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("robin.json5");

        let initial_config = r#"{
            "gateway": {"host": "127.0.0.1", "port": 18789},
            "agents": {"list": [{"id": "default", "name": "Test", "model": "openai/gpt-4o"}]}
        }"#;
        std::fs::write(&cfg_path, initial_config).unwrap();

        let w = Watcher::new(cfg_path.to_str().unwrap(), |_cfg| {}).unwrap();
        w.start();

        let start = std::time::Instant::now();
        w.stop();
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "Stop() should return within 3 seconds"
        );
    }
}