#![cfg(not(target_os = "windows"))]

/// Return PNG bytes as-is; tray-icon accepts PNG on macOS and Linux.
/// Mirrors Go's trayIcon on !windows.
pub fn tray_icon(png_data: &[u8]) -> Vec<u8> {
    png_data.to_vec()
}