pub(super) mod shared;
#[cfg(windows)]
#[path = "windows/mod.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "linux_os/mod.rs"]
mod platform;
#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use crate::rdp::core::HostProbe;
    pub(crate) fn probe() -> HostProbe {
        HostProbe::unavailable(std::env::consts::OS,
            "This platform has no supported RDP host; Android is a viewer, not an RDP host")
    }
    pub(crate) mod clients {
        use crate::rdp::core::{ClientCapability, HostService, StartRequest};
        use std::{net::SocketAddr, process::Command};
        pub(crate) fn inventory() -> Vec<ClientCapability> { Vec::new() }
        pub(crate) fn hide_probe_console(_: &mut Command) {}
        pub(crate) fn launch(_: &StartRequest, _: &HostService, _: SocketAddr) -> Result<super::super::shared::clients::ClientProcess, String> {
            Err("No installed RDP client has a documented certificate-verification adapter on this platform".into())
        }
    }
}
pub(crate) use platform::probe;
