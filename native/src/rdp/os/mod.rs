#[cfg(windows)]
#[path = "windows/mod.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "linux_os/mod.rs"]
mod platform;
pub(super) mod shared;
#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use crate::rdp::core::HostProbe;
    pub(crate) fn probe() -> HostProbe {
        HostProbe::unavailable(
            std::env::consts::OS,
            "This platform has no supported RDP host; Android is a viewer, not an RDP host",
        )
    }
    /// Android and every other platform host no RDP service (design 3.4).
    pub(crate) mod host_setup {
        use crate::rdp::core::{HostProbe, HostService, SetupState};
        pub(crate) fn impossible() -> Option<SetupState> {
            Some(SetupState::Impossible(if cfg!(target_os = "android") {
                "android".into()
            } else {
                format!("platform_{}", std::env::consts::OS)
            }))
        }
        pub(crate) fn ensure(_: &HostProbe, _: Option<&str>) -> SetupState {
            impossible().unwrap_or_else(|| SetupState::Failed("platform".into()))
        }
        pub(crate) fn issue_credential(
            _: &HostService,
            _: &str,
        ) -> Result<(String, String), String> {
            Err("This platform hosts no RDP service".into())
        }
        pub(crate) fn rotate_credentials(_: &str) -> Result<(), String> {
            Ok(())
        }
        pub(crate) fn withdraw_managed() -> Result<bool, String> {
            Ok(false)
        }
        pub(crate) fn session_facts() -> String {
            String::new()
        }
        pub(crate) fn run_system_step(_: &str, _: Option<&str>) -> (String, i32) {
            ("state=impossible:platform".into(), 1)
        }
    }
    pub(crate) mod clients {
        use crate::rdp::core::{ClientCapability, HostService, StartRequest};
        use std::{net::SocketAddr, process::Command};
        pub(crate) fn inventory() -> Vec<ClientCapability> {
            Vec::new()
        }
        pub(crate) fn hide_probe_console(_: &mut Command) {}
        pub(crate) fn launch(
            _: &StartRequest,
            _: &HostService,
            _: SocketAddr,
        ) -> Result<super::super::shared::clients::ClientProcess, String> {
            Err("No installed RDP client has a documented certificate-verification adapter on this platform".into())
        }
    }
}
pub(crate) use platform::probe;
