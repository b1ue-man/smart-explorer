use std::io;

pub(crate) use super::shared_system::lan_ips;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const FIREWALL_RULE: &str = "Smart Explorer Share Peer Listener";

pub(crate) fn ensure_firewall_rule() -> io::Result<String> {
    let exe = std::env::current_exe()?;
    ensure_firewall_rule_for(&exe)
}

pub(crate) fn ensure_firewall_rule_for(exe: &std::path::Path) -> io::Result<String> {
    use std::os::windows::process::CommandExt;

    let exe = exe.to_string_lossy().to_string();
    let delete = std::process::Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "delete",
            "rule",
            &format!("name={FIREWALL_RULE}"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = delete;

    let output = std::process::Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "add",
            "rule",
            &format!("name={FIREWALL_RULE}"),
            "dir=in",
            "action=allow",
            &format!("program={exe}"),
            "enable=yes",
            "profile=private,domain",
            "protocol=UDP",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if output.status.success() {
        Ok(format!("Firewall-Regel aktiv: {FIREWALL_RULE}"))
    } else {
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            if msg.is_empty() {
                "UDP-Firewall-Regel fuer private/Domaenen-Netze braucht eine ausdrueckliche Administratorfreigabe".to_string()
            } else {
                msg
            },
        ))
    }
}

pub(crate) fn request_firewall_rule_elevated(exe: &str) -> io::Result<()> {
    let engine = crate::net::system_powershell()?;
    let netsh = engine
        .parent()
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
        .ok_or_else(|| io::Error::other("System directory unavailable"))?
        .join("netsh.exe");
    let netsh = netsh.to_string_lossy().replace('\'', "''");
    let escaped_exe = exe.replace('\'', "''");
    let script = format!("$ErrorActionPreference='Stop'; & '{netsh}' advfirewall firewall delete rule name='{FIREWALL_RULE}'; \
        & '{netsh}' advfirewall firewall add rule name='{FIREWALL_RULE}' dir=in action=allow program='{escaped_exe}' enable=yes protocol=UDP profile=private,domain; \
        if ($LASTEXITCODE -ne 0) {{ throw 'Firewall rule repair failed' }}");
    crate::net::run_elevated_system_powershell(&script)
}
