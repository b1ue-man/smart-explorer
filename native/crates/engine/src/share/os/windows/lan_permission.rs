pub fn lan_firewall_repair_available() -> bool {
    true
}

/// Only a direct user action calls the UAC repair, never presence startup.
pub fn request_lan_firewall_repair() -> std::io::Result<()> {
    let current = std::env::current_exe()?;
    let exe = current
        .parent()
        .ok_or_else(|| std::io::Error::other("Worker directory unavailable"))?
        .join("se.exe");
    let metadata = std::fs::symlink_metadata(&exe)?;
    use std::os::windows::fs::MetadataExt;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err(std::io::Error::other(
            "Share worker is not a regular executable",
        ));
    }
    let exe = exe
        .to_str()
        .ok_or_else(|| std::io::Error::other("Firewall path is not Unicode"))?;
    super::system::request_firewall_rule_elevated(exe)
}
