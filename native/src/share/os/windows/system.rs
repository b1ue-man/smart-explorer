use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) use super::shared_system::lan_ips;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const FIREWALL_RULE: &str = "Smart Explorer Share Peer Listener";
static ELEVATED_FIREWALL_ATTEMPTED: AtomicBool = AtomicBool::new(false);

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
            "profile=any",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if output.status.success() {
        Ok(format!("Firewall-Regel aktiv: {FIREWALL_RULE}"))
    } else {
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !ELEVATED_FIREWALL_ATTEMPTED.swap(true, Ordering::Relaxed) {
            request_firewall_rule_elevated(&exe)?;
            Ok(format!(
                "Firewall-Freigabe angefragt: Windows-UAC bestaetigen ({FIREWALL_RULE})"
            ))
        } else {
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
}

fn request_firewall_rule_elevated(exe: &str) -> io::Result<()> {
    use std::os::windows::process::CommandExt;

    let escaped_exe = exe.replace('\'', "''");
    let escaped_rule = FIREWALL_RULE.replace('\'', "''");
    let script = format!(
        "netsh advfirewall firewall delete rule name='{escaped_rule}'; \
         netsh advfirewall firewall add rule name='{escaped_rule}' dir=in action=allow program='{escaped_exe}' enable=yes profile=any"
    );
    let arg_list = format!("-NoProfile -ExecutionPolicy Bypass -Command \"{script}\"");
    let status = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "Start-Process",
            "powershell",
            "-Verb",
            "RunAs",
            "-WindowStyle",
            "Hidden",
            "-ArgumentList",
            &arg_list,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Firewall-UAC-Anfrage konnte nicht gestartet werden",
        ))
    }
}
