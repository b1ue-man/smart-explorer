pub fn lan_firewall_repair_available() -> bool { false }
pub fn request_lan_firewall_repair() -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Keine Windows-Firewall auf dieser Plattform"))
}
