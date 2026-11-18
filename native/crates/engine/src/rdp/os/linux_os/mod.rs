mod client_trust;
pub mod clients;
mod gnome;
/// Sign-in with the grant's account: xrdp with a one-time se token (P4).
pub mod host_login;
pub mod host_setup;
mod inventory;
mod krdp;
/// KRdp sign-in with se's own KRdp user (RDP-ZIELKONTO, second step).
mod krdp_login;
/// The account at the desktop for a root worker (RDP-ZIELKONTO, point 7).
mod session_account;
mod xrdp;
/// Token list, PAM line and PAM helper of the xrdp sign-in (P4).
pub mod xrdp_token;
use crate::rdp::core::{HostProbe, RDP_PROBE_VERSION_V2};

pub fn probe() -> HostProbe {
    let mut report = HostProbe {
        version: RDP_PROBE_VERSION_V2,
        platform: "linux".into(),
        services: Vec::new(),
        unavailable_reason: None,
        diagnostics: Vec::new(),
    };
    gnome::probe(&mut report);
    krdp::probe(&mut report);
    xrdp::probe(&mut report);
    if report.services.is_empty() {
        report.unavailable_reason = Some("No installed GNOME Remote Desktop, KRdp or xrdp service was found in the current host inventory".into());
    }
    report
}
