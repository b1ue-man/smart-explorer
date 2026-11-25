use std::io;

pub use super::shared_system::lan_ips;

#[cfg(target_os = "linux")]
#[path = "firewall.rs"]
mod firewall;
#[cfg(target_os = "linux")]
#[path = "../shared/firewall_linux_plan.rs"]
mod firewall_linux_plan;
// The Windows rule plan is pure; its tests also run on the Linux host, where
// only they use it.
#[cfg(all(test, target_os = "linux"))]
#[allow(dead_code)]
#[path = "../shared/firewall_windows_plan.rs"]
mod firewall_windows_plan;

/// At use (the Share worker's start, before peers connect): se's fixed ports
/// admitted in an active ufw, firewalld, nftables or iptables firewall, with
/// the polkit dialog when root rights are missing.
#[cfg(target_os = "linux")]
pub fn ensure_firewall_rule() -> io::Result<String> {
    firewall::ensure_at_use()
}

/// Android has no host firewall that filters an app's sockets.
#[cfg(not(target_os = "linux"))]
pub fn ensure_firewall_rule() -> io::Result<String> {
    Ok("Firewall-Regel: auf diesem System nicht erforderlich".to_string())
}

/// The installation and the update helper. `ask` (an installation the person
/// watches) may raise the polkit dialog; without it only a root helper
/// changes the firewall and the restarted programs ask at their start.
#[cfg(target_os = "linux")]
pub fn ensure_peer_firewall_for_installation(
    _dir: &std::path::Path,
    ask: bool,
) -> io::Result<String> {
    if ask || crate::login::os::platform::privileged() {
        firewall::ensure_at_use()
    } else {
        Ok(firewall::state_without_prompt())
    }
}

#[cfg(not(target_os = "linux"))]
pub fn ensure_peer_firewall_for_installation(
    _dir: &std::path::Path,
    _ask: bool,
) -> io::Result<String> {
    ensure_firewall_rule()
}

/// `se rdp host-setup --system firewall-peer` as root; `None` for another step.
#[cfg(target_os = "linux")]
pub fn firewall_system_step(step: &str) -> Option<(String, i32)> {
    firewall::system_step(step)
}

#[cfg(not(target_os = "linux"))]
pub fn firewall_system_step(_step: &str) -> Option<(String, i32)> {
    None
}
