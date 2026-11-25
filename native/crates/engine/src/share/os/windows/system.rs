use std::io;
use std::path::Path;

pub use super::shared_system::lan_ips;

#[path = "firewall.rs"]
mod firewall;
#[path = "../shared/firewall_windows_plan.rs"]
mod firewall_windows_plan;

/// The elevated step `se rdp host-setup --system firewall-peer`.
pub const FIREWALL_STEP: &str = "firewall-peer";

/// At use (the Share worker's start, before peers connect): the program-bound
/// TCP and UDP rules of this installation in every network profile, set up or
/// repaired with the UAC confirmation when one is missing or outdated.
pub fn ensure_firewall_rule() -> io::Result<String> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| io::Error::other("Programmordner unbekannt"))?;
    firewall::ensure(dir, firewall::Consent::Ask)
}

/// The explicit repair (`request_lan_firewall_repair`, button „Firewall
/// freigeben / reparieren“): `exe` is the `se.exe` of the installation. Rules
/// that already fit raise no prompt.
pub fn request_firewall_rule_elevated(exe: &str) -> io::Result<()> {
    let dir = Path::new(exe)
        .parent()
        .ok_or_else(|| io::Error::other("Programmordner unbekannt"))?;
    firewall::ensure(dir, firewall::Consent::Ask).map(|_| ())
}

/// The installation and the update helper: the rules of the installation
/// folder `dir`. `ask` (an installation the person watches) asks for the UAC
/// confirmation; without it only an elevated helper changes rules and the
/// restarted programs ask at their start.
pub fn ensure_peer_firewall_for_installation(dir: &Path, ask: bool) -> io::Result<String> {
    let consent = if ask {
        firewall::Consent::Ask
    } else {
        firewall::Consent::ElevatedOnly
    };
    firewall::ensure(dir, consent)
}

/// `se rdp host-setup --system firewall-peer` in an elevated `se.exe`: the
/// rules of its own installation; `None` for another step.
pub fn firewall_system_step(step: &str) -> Option<(String, i32)> {
    if step != FIREWALL_STEP {
        return None;
    }
    Some(match ensure_firewall_rule() {
        Ok(status) => (format!("firewall_state=done:{status}"), 0),
        Err(error) => (format!("firewall_state=failed:{error}"), 70),
    })
}
