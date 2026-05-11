use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

const PERSISTED_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Persisted {
    v: u32,
    ratio: f64,
    count: usize,
}

pub struct CalibratorStore {
    base_dir: String,
    mu: Mutex<()>,
}

impl CalibratorStore {
    pub fn new(base_dir: &str) -> Self {
        Self { base_dir: base_dir.to_string(), mu: Mutex::new(()) }
    }

    fn path(&self, agent_id: &str, session_key: &str) -> Option<PathBuf> {
        if self.base_dir.is_empty() || agent_id.is_empty() || session_key.is_empty() {
            return None;
        }
        Some(PathBuf::from(&self.base_dir).join(format!("{}__{}.json", agent_id, session_key)))
    }

    pub fn load(&self, agent_id: &str, session_key: &str) -> (f64, usize) {
        let p = match self.path(agent_id, session_key) {
            Some(p) => p,
            None => return (1.0, 0),
        };
        let _guard = self.mu.lock();
        let data = match std::fs::read(&p) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (1.0, 0),
            Err(e) => { debug!("calibrator load error path={:?} error={}", p, e); return (1.0, 0); }
        };
        let rec: Persisted = match serde_json::from_slice(&data) {
            Ok(r) => r,
            Err(e) => { debug!("calibrator parse error path={:?} error={}", p, e); return (1.0, 0); }
        };
        if rec.v != PERSISTED_VERSION || rec.ratio <= 0.0 { return (1.0, 0); }
        (rec.ratio, rec.count)
    }

    pub fn save(&self, agent_id: &str, session_key: &str, ratio: f64, count: usize) {
        let p = match self.path(agent_id, session_key) { Some(p) => p, None => return };
        if ratio <= 0.0 || count == 0 { return; }
        let _guard = self.mu.lock();
        if let Some(parent) = p.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                warn!("calibrator dir create failed dir={:?} error={}", parent, e);
                return;
            }
        }
        let data = match serde_json::to_vec(&Persisted { v: PERSISTED_VERSION, ratio, count }) {
            Ok(d) => d,
            Err(_) => return,
        };
        let tmp = p.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, &data) {
            debug!("calibrator write failed path={:?} error={}", tmp, e);
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, &p) {
            let _ = std::fs::remove_file(&tmp);
            debug!("calibrator rename failed path={:?} error={}", p, e);
        }
    }

    pub fn forget(&self, agent_id: &str, session_key: &str) {
        let p = match self.path(agent_id, session_key) { Some(p) => p, None => return };
        let _guard = self.mu.lock();
        let _ = std::fs::remove_file(&p);
    }
}