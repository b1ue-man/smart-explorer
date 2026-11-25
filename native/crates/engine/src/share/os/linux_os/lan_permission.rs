//! The explicit firewall repair on Linux (button „Firewall freigeben /
//! reparieren“ of the LAN page): se's own ports in the active firewall, with
//! the polkit dialog when root rights are missing (user 2026-11-25: „SE MUSS
//! ES MACHEN! AUTOMATISCH! ELEVATION FEHLT? ERFRAGEN!“).

pub fn lan_firewall_repair_available() -> bool {
    true
}

/// Only a direct user action or the start at use asks; presence never does.
pub fn request_lan_firewall_repair() -> std::io::Result<()> {
    super::system::ensure_firewall_rule().map(|_| ())
}
