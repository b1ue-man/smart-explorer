use std::io;

pub use super::shared_system::lan_ips;

pub fn ensure_firewall_rule() -> io::Result<String> {
    Ok("Firewall-Regel: auf diesem System nicht erforderlich".to_string())
}
