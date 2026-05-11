/// spill.rs — Per-session tool-result spill directory helpers.
///
/// Mirrors Go's spill.go. Provides path computation, cleanup, and orphan-janitor
/// functions for the workspace-local spill directory.
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

/// Returns the absolute path of the spill subdirectory for `(workspace,
/// session_key)`. Returns `None` if either argument is empty — we never want
/// to accidentally walk a non-spill location.
///
/// Layout:  `<workspace>/.robin/spill/<session_key>/`
pub fn spill_dir_for_session(workspace: &str, session_key: &str) -> Option<PathBuf> {
    if workspace.is_empty() || session_key.is_empty() {
        return None;
    }
    Some(
        Path::new(workspace)
            .join(".robin")
            .join("spill")
            .join(session_key),
    )
}

/// Removes the per-session spill directory if it exists. Safe to call when the
/// directory was never created. Logs at warn on unexpected I/O errors but never
/// returns one.
pub fn remove_session_spill(workspace: &str, session_key: &str) {
    if let Some(dir) = spill_dir_for_session(workspace, session_key) {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            if e.kind() != io::ErrorKind::NotFound {
                log::warn!("spill cleanup failed: dir={} error={}", dir.display(), e);
            }
        }
    }
}

/// Returns the parent directory under which all per-session spill directories
/// live for the given workspace. Used by the startup janitor.
pub fn spill_root(workspace: &str) -> Option<PathBuf> {
    if workspace.is_empty() {
        return None;
    }
    Some(Path::new(workspace).join(".robin").join("spill"))
}

/// Callback that returns the set of currently-live session keys for the agent
/// that owns the given workspace. The startup janitor uses this to decide which
/// spill dirs are orphans.
pub type LiveSessionKeysFn = Box<dyn Fn() -> io::Result<HashSet<String>> + Send + Sync>;

/// Walks `<workspace>/.robin/spill/` and removes any per-session subdirectory
/// whose key is not in the set returned by `live_keys`. Returns the number of
/// directories removed and the first error encountered (if any). A missing root
/// is not an error.
pub fn cleanup_orphaned_spills(
    workspace: &str,
    live_keys: &dyn Fn() -> io::Result<HashSet<String>>,
) -> io::Result<usize> {
    let root = match spill_root(workspace) {
        Some(r) => r,
        None => return Ok(0),
    };

    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };

    let live = live_keys()?;
    let mut removed = 0usize;
    let mut first_err: Option<io::Error> = None;

    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        if live.contains(&name) {
            continue;
        }
        let full = root.join(&name);
        if let Err(e) = std::fs::remove_dir_all(&full) {
            log::warn!("orphan spill cleanup failed: dir={} error={}", full.display(), e);
            if first_err.is_none() {
                first_err = Some(e);
            }
            continue;
        }
        removed += 1;
    }

    if removed > 0 {
        log::info!("removed orphan spill directories: workspace={} count={}", workspace, removed);
    }

    if let Some(e) = first_err {
        return Err(e);
    }
    Ok(removed)
}