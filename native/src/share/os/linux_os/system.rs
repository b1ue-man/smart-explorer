use std::io;

pub(crate) use super::shared_system::lan_ips;

pub(crate) fn ensure_firewall_rule() -> io::Result<String> {
    Ok("Firewall-Regel: auf diesem System nicht erforderlich".to_string())
}
