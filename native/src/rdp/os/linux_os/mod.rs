mod client_trust;
pub(crate) mod clients;
mod gnome;
pub(crate) mod host_setup;
mod inventory;
mod krdp;
mod xrdp;
use crate::rdp::core::{HostProbe, RDP_PROBE_VERSION_V2};

pub(crate) fn probe() -> HostProbe {
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
