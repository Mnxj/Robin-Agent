#![cfg(not(target_os = "windows"))]

/// On non-Windows platforms, write the error to the log.
/// Mirrors Go's showError on !windows.
pub fn show_error(msg: &str) {
    tracing::error!("{msg}");
}