use crate::session::session::{Session, SessionEntry};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Describes a session without loading its full contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub key: String,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "lastActivity")]
    pub last_activity: DateTime<Utc>,
    #[serde(rename = "entryCount")]
    pub entry_count: usize,
}

/// Handles JSONL file I/O for sessions.
pub struct Store {
    base_dir: PathBuf,
    mu: Mutex<()>,
}

impl Store {
    /// Creates a new session store rooted at `base_dir`.
    pub fn new(base_dir: impl AsRef<Path>) -> Arc<Self> {
        Arc::new(Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            mu: Mutex::new(()),
        })
    }

    fn session_dir(&self, agent_id: &str) -> PathBuf {
        self.base_dir.join(agent_id)
    }

    fn session_path(&self, agent_id: &str, key: &str) -> PathBuf {
        self.session_dir(agent_id).join(format!("{}.jsonl", key))
    }

    /// Reads a session from its JSONL file.
    /// Returns an empty session (associated with the store) if the file does not exist.
    pub fn load(self: &Arc<Self>, agent_id: &str, key: &str) -> anyhow::Result<Arc<Session>> {
        let path = self.session_path(agent_id, key);

        let sess = Arc::new(Session::new(agent_id, key));
        sess.set_store(self.clone());

        let f = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(sess);
            }
            Err(e) => {
                return Err(anyhow::anyhow!("open session file: {}", e));
            }
        };

        let reader = std::io::BufReader::new(f);
        for line in reader.lines() {
            let line = line.map_err(|e| anyhow::anyhow!("read session file: {}", e))?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let entry: SessionEntry = match serde_json::from_str(line) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("skipping malformed session entry: error={}", e);
                    continue;
                }
            };

            sess.load_entry(entry);
        }

        Ok(sess)
    }

    /// Writes a single entry to the session's JSONL file.
    pub fn append_entry(&self, sess: &Session, entry: SessionEntry) {
        let _guard = self.mu.lock();

        let dir = self.session_dir(&sess.agent_id);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::error!("failed to create session dir: error={}", e);
            return;
        }

        let path = self.session_path(&sess.agent_id, &sess.key);
        let mut f = match std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("failed to open session file: error={}", e);
                return;
            }
        };

        let mut data = match serde_json::to_vec(&entry) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("failed to marshal session entry: error={}", e);
                return;
            }
        };
        data.push(b'\n');

        if let Err(e) = f.write_all(&data) {
            tracing::error!("failed to write session entry: error={}", e);
        }
    }

    /// Creates an empty session file on disk so it shows up in `list`.
    pub fn create(&self, agent_id: &str, key: &str) -> anyhow::Result<()> {
        let _guard = self.mu.lock();

        let dir = self.session_dir(agent_id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("create session dir: {}", e))?;

        let path = self.session_path(agent_id, key);
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|e| anyhow::anyhow!("create session file: {}", e))?;

        Ok(())
    }

    /// Returns metadata for all sessions belonging to the given agent.
    pub fn list(&self, agent_id: &str) -> anyhow::Result<Vec<SessionInfo>> {
        let dir = self.session_dir(agent_id);
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(anyhow::anyhow!("read session dir: {}", e)),
        };

        let mut sessions: Vec<SessionInfo> = Vec::new();

        for de in read_dir {
            let de = de?;
            let name = de.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.ends_with(".jsonl") {
                continue;
            }

            let key = name_str.trim_end_matches(".jsonl").to_string();
            let path = dir.join(&*name_str);

            let f = match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let reader = std::io::BufReader::new(f);
            let mut first_ts: i64 = 0;
            let mut last_ts: i64 = 0;
            let mut line_count: usize = 0;

            for line in reader.lines() {
                let Ok(line) = line else { continue };
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                line_count += 1;

                #[derive(Deserialize)]
                struct Partial {
                    timestamp: Option<i64>,
                }
                if let Ok(p) = serde_json::from_str::<Partial>(line) {
                    if let Some(ts) = p.timestamp.filter(|&t| t > 0) {
                        if first_ts == 0 {
                            first_ts = ts;
                        }
                        last_ts = ts;
                    }
                }
            }

            let created_at = if first_ts > 0 {
                DateTime::from_timestamp(first_ts, 0).unwrap_or_else(Utc::now)
            } else {
                de.metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .map(DateTime::from)
                    .unwrap_or_else(Utc::now)
            };

            let last_activity = if last_ts > 0 {
                DateTime::from_timestamp(last_ts, 0).unwrap_or(created_at)
            } else {
                created_at
            };

            sessions.push(SessionInfo {
                key,
                created_at,
                last_activity,
                entry_count: line_count,
            });
        }

        Ok(sessions)
    }

    /// Checks whether a session file exists for the given agent and key.
    pub fn exists(&self, agent_id: &str, key: &str) -> bool {
        self.session_path(agent_id, key).exists()
    }

    /// Renames a session file from `old_key` to `new_key`.
    pub fn rename(&self, agent_id: &str, old_key: &str, new_key: &str) -> anyhow::Result<()> {
        let _guard = self.mu.lock();

        let old_path = self.session_path(agent_id, old_key);
        let new_path = self.session_path(agent_id, new_key);

        if !old_path.exists() {
            return Err(anyhow::anyhow!("session {:?} does not exist", old_key));
        }
        if new_path.exists() {
            return Err(anyhow::anyhow!("session {:?} already exists", new_key));
        }

        std::fs::rename(&old_path, &new_path)
            .map_err(|e| anyhow::anyhow!("rename session file: {}", e))
    }

    /// Removes a session's JSONL file.
    pub fn delete(&self, agent_id: &str, key: &str) -> anyhow::Result<()> {
        let _guard = self.mu.lock();

        let path = self.session_path(agent_id, key);
        match std::fs::remove_file(&path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(anyhow::anyhow!("remove session file: {}", e)),
        }
    }

    /// Replaces the entire session JSONL file with the current entries.
    /// Used after compaction to replace the old file.
    pub fn rewrite(&self, sess: &Session) {
        let _guard = self.mu.lock();

        let dir = self.session_dir(&sess.agent_id);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::error!("failed to create session dir: error={}", e);
            return;
        }

        let path = self.session_path(&sess.agent_id, &sess.key);
        let f = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("failed to create session file for rewrite: error={}", e);
                return;
            }
        };

        let mut w = std::io::BufWriter::new(f);
        for entry in sess.entries() {
            let data = match serde_json::to_vec(&entry) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("failed to marshal session entry: error={}", e);
                    continue;
                }
            };
            if let Err(e) = w.write_all(&data) {
                tracing::error!("failed to write session entry: error={}", e);
                return;
            }
            if let Err(e) = w.write_all(b"\n") {
                tracing::error!("failed to write newline: error={}", e);
                return;
            }
        }

        if let Err(e) = w.flush() {
            tracing::error!("failed to flush session file: error={}", e);
        }
    }
}