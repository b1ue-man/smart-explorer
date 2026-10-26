mod client_trust;
pub(crate) mod clients;
mod gnome;
mod inventory;
mod krdp;
mod xrdp;
use crate::rdp::core::{HostProbe, RDP_PROBE_VERSION};

pub(crate) fn probe() -> HostProbe {
    let mut report = HostProbe {
        version: RDP_PROBE_VERSION,
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
