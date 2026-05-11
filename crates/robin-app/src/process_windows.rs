#![cfg(target_os = "windows")]

use std::time::Duration;
use tracing::{info, warn};

pub struct GatewayInner {
    child: Option<std::process::Child>,
    pub port: u16,
    owned: bool,
    exited: bool,
}

impl GatewayInner {
    pub fn has_exited(&mut self) -> bool {
        if !self.owned || self.exited { return false; }
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(_)) => { self.exited = true; true }
                _ => false,
            }
        } else { false }
    }

    pub fn mark_detached(&mut self) { self.owned = false; }

    pub fn stop(&mut self) {
        if !self.owned { return; }
        let Some(child) = &mut self.child else { return };
        if let Err(e) = child.kill() {
            warn!("kill gateway subprocess: {e}");
        }
        let _ = child.wait();
        info!("gateway subprocess exited");
    }
}

fn find_robin_binary() -> anyhow::Result<std::path::PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or_else(|| anyhow::anyhow!("cannot determine exe dir"))?;
    for candidate in &[dir.join("robin.exe"), dir.join("robin")] {
        if let Ok(abs) = candidate.canonicalize() {
            if abs.is_file() { return Ok(abs); }
        }
    }
    which::which("robin").map_err(|_| anyhow::anyhow!("robin binary not found"))
}

const GATEWAY_PORT: u16 = 18789;

pub fn start_or_attach_gateway(ready_timeout: Duration) -> anyhow::Result<GatewayInner> {
    if probe_health(GATEWAY_PORT) {
        info!("attaching to existing gateway port={}", GATEWAY_PORT);
        return Ok(GatewayInner { child: None, port: GATEWAY_PORT, owned: false, exited: false });
    }
    let bin = find_robin_binary()?;
    let child = std::process::Command::new(&bin)
        .arg("start").envs(std::env::vars()).spawn()
        .map_err(|e| anyhow::anyhow!("spawn {:?} start: {}", bin, e))?;
    info!("spawned gateway subprocess pid={} binary={:?}", child.id(), bin);
    let mut gw = GatewayInner { child: Some(child), port: GATEWAY_PORT, owned: true, exited: false };
    wait_for_ready(GATEWAY_PORT, ready_timeout)
        .map_err(|e| { gw.stop(); anyhow::anyhow!("gateway did not become ready: {e}") })?;
    Ok(gw)
}

fn probe_health(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/health");
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(500))
        .build().ok()
        .and_then(|c| c.get(&url).send().ok())
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

fn wait_for_ready(port: u16, timeout: Duration) -> anyhow::Result<()> {
    let url = format!("http://127.0.0.1:{port}/health");
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(1)).build()?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(r) = client.get(&url).send() {
            if r.status().is_success() { return Ok(()); }
        }
        if std::time::Instant::now() > deadline {
            anyhow::bail!("/health did not return 200 within {:?}", timeout);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}