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
    pub fn probe() -> HostProbe {
        HostProbe::unavailable(
            std::env::consts::OS,
            "This platform has no supported RDP host; Android is a viewer, not an RDP host",
        )
    }
    /// Android and every other platform host no RDP service (design 3.4).
    pub mod host_setup {
        use crate::rdp::core::{HostProbe, HostService, SetupState};
        pub fn impossible() -> Option<SetupState> {
            Some(SetupState::Impossible(if cfg!(target_os = "android") {
                "android".into()
            } else {
                format!("platform_{}", std::env::consts::OS)
            }))
        }
        pub fn ensure(_: &HostProbe, _: Option<&str>) -> SetupState {
            impossible().unwrap_or_else(|| SetupState::Failed("platform".into()))
        }
        pub fn issue_credential(_: &HostService, _: &str) -> Result<(String, String), String> {
            Err("This platform hosts no RDP service".into())
        }
        pub fn rotate_credentials(_: &str) -> Result<(), String> {
            Ok(())
        }
        pub fn withdraw_managed() -> Result<bool, String> {
            Ok(false)
        }
        pub fn session_facts() -> String {
            String::new()
        }
        pub fn run_system_step(_: &str, _: Option<&str>) -> (String, i32) {
            ("state=impossible:platform".into(), 1)
        }
    }
    /// No RDP host here, so no sign-in with the device's account (P4).
    pub mod host_login {
        use crate::rdp::core::{HostAccount, LoginInfo};
        use crate::rdp::os::shared::host_login::{Automatic, Viewer};
        use crate::share::access_grant::AccountProfile;
        const NONE: &str = "Dieses Gerät ist kein RDP-Host";
        pub fn supports(_: &str) -> Result<(), String> {
            Err(NONE.into())
        }
        pub fn account(_: &AccountProfile) -> Result<HostAccount, String> {
            Err(NONE.into())
        }
        pub fn automatic(
            _: &str,
            _: &AccountProfile,
            _: &HostAccount,
            _: &Viewer,
            _: bool,
        ) -> Automatic {
            Automatic::No
        }
        pub fn verify(_: &HostAccount, _: &str) -> Result<bool, String> {
            Err(NONE.into())
        }
        pub fn withdraw(_: &Viewer, _: &LoginInfo) {}
        pub fn coverage_ended(_: &dyn Fn(&Viewer) -> bool, _: bool) {}
    }
    pub mod clients {
        use crate::rdp::core::{ClientCapability, HostService, StartRequest};
        use std::{net::SocketAddr, process::Command};
        pub fn inventory() -> Vec<ClientCapability> {
            Vec::new()
        }
        pub fn hide_probe_console(_: &mut Command) {}
        pub fn launch(
            _: &StartRequest,
            _: &HostService,
            _: SocketAddr,
        ) -> Result<super::super::shared::clients::ClientProcess, String> {
            Err("No installed RDP client has a documented certificate-verification adapter on this platform".into())
        }
    }
}
pub use platform::probe;

/// `se --rdp-xrdp-login` (pam_exec of xrdp-sesman, P4): 0 accepts the
/// login with a se token; other platforms host no xrdp.
pub fn xrdp_login_check() -> i32 {
    #[cfg(target_os = "linux")]
    {
        platform::xrdp_token::pam_check()
    }
    #[cfg(not(target_os = "linux"))]
    {
        1
    }
}
