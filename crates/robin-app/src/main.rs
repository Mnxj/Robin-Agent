mod error_display;
mod error_other;
mod error_windows;
mod icon;
mod icon_other;
mod icon_windows;
mod process;
mod process_unix;
mod process_windows;

use std::time::Duration;

static VERSION: &str = env!("CARGO_PKG_VERSION");
const COMMIT: &str = match option_env!("GIT_COMMIT") {
    Some(s) => s,
    None => "none",
};

static ICON_BYTES: &[u8] = include_bytes!("../icon.png");

fn main() {
    init_log_file();
    load_shell_env();

    tracing::info!("robin-app starting version={} commit={} pid={}", VERSION, COMMIT, std::process::id());

    let icon_data = icon::tray_icon(ICON_BYTES);
    let _ = icon_data;

    let mut gw = match process::start_or_attach_gateway(Duration::from_secs(90)) {
        Ok(g) => g,
        Err(e) => {
            error_display::show_error(&format!("Robin failed to start the gateway:\n\n{e}"));
            return;
        }
    };

    open_url(&format!("http://localhost:{}/chat", gw.port()));

    loop {
        if gw.has_exited() {
            tracing::error!("gateway subprocess exited unexpectedly");
            error_display::show_error(
                "Robin's gateway process stopped unexpectedly. Relaunch Robin to restart it.",
            );
            gw.mark_detached();
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(url).spawn(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg(url).spawn(); }
    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn(); }
    let _ = url;
}

fn init_log_file() {
    // TODO: initialize file logger to ~/.robin/robin-app.log
}

fn load_shell_env() {
    #[cfg(target_os = "macos")]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        if let Ok(out) = std::process::Command::new(&shell).args(["-ilc", "env"]).output() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Some((k, v)) = line.split_once('=') {
                    if k.is_empty() { continue; }
                    if k == "PATH" || std::env::var(k).is_err() {
                        unsafe { std::env::set_var(k, v); }
                    }
                }
            }
        }
    }
}