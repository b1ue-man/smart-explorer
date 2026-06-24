use std::io;

const QUARANTINE: &str = "SmartExplorer-Empfangen";
const FIREWALL_RULE: &str = "Smart Explorer Share Peer Listener";

pub(crate) fn lan_ips() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            let ip = iface.ip();
            if ip.is_loopback() || ip.is_unspecified() {
                continue;
            }
            if let std::net::IpAddr::V4(v4) = ip {
                v.push(v4.to_string());
            }
        }
    }
    if let Ok(s) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:80").is_ok() {
            if let Ok(a) = s.local_addr() {
                v.push(a.ip().to_string());
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

pub(crate) fn quarantine_dir() -> io::Result<std::path::PathBuf> {
    let base = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join(QUARANTINE);
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

pub(crate) fn unique_in(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let stem = std::path::Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = std::path::Path::new(name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for i in 2..10_000 {
        let cand = dir.join(format!("{} ({}){}", stem, i, ext));
        if !cand.exists() {
            return cand;
        }
    }
    p
}

#[cfg(windows)]
pub(crate) fn ensure_firewall_rule() -> io::Result<String> {
    let exe = std::env::current_exe()?;
    ensure_firewall_rule_for(&exe)
}

#[cfg(windows)]
pub(crate) fn ensure_firewall_rule_for(exe: &std::path::Path) -> io::Result<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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
            "profile=any",
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
                "Firewall-Regel konnte nicht gesetzt werden".to_string()
            } else {
                msg
            },
        ))
    }
}

#[cfg(not(windows))]
pub(crate) fn ensure_firewall_rule() -> io::Result<String> {
    Ok("Firewall-Regel: auf diesem System nicht erforderlich".to_string())
}
