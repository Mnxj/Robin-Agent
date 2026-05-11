use std::time::Duration;

pub struct Gateway {
    pub(crate) inner: GatewayInner,
}

#[cfg(not(target_os = "windows"))]
pub(crate) use crate::process_unix::GatewayInner;

#[cfg(target_os = "windows")]
pub(crate) use crate::process_windows::GatewayInner;

impl Gateway {
    pub fn port(&self) -> u16 { self.inner.port }
    pub fn has_exited(&mut self) -> bool { self.inner.has_exited() }
    pub fn mark_detached(&mut self) { self.inner.mark_detached() }
    pub fn stop(&mut self) { self.inner.stop() }
}

pub fn start_or_attach_gateway(ready_timeout: Duration) -> anyhow::Result<Gateway> {
    #[cfg(not(target_os = "windows"))]
    { crate::process_unix::start_or_attach_gateway(ready_timeout).map(|inner| Gateway { inner }) }
    #[cfg(target_os = "windows")]
    { crate::process_windows::start_or_attach_gateway(ready_timeout).map(|inner| Gateway { inner }) }
}