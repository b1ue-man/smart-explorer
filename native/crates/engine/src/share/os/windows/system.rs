use std::io;

pub use super::shared_system::lan_ips;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const FIREWALL_RULE: &str = "Smart Explorer Share Peer Listener";

pub fn ensure_firewall_rule() -> io::Result<String> {
    let exe = std::env::current_exe()?;
    let message = ensure_firewall_rule_for(&exe)?;
    // R8 (plan 2026-11-06): the rule covers private and domain networks
    // only, as Windows intends; on a network classified „Öffentlich“ the
    // firewall still drops incoming UDP and every device reaches this one
    // over the relay only. That is reported, never changed silently.
    Ok(match public_networks() {
        names if names.is_empty() => message,
        names => format!("{message}. {}", public_network_notice(&names)),
    })
}

/// Marker the desktop recognizes in the status line (`share_drain.rs`).
pub const PUBLIC_NETWORK_NOTICE: &str = "Netzwerk ist als „Öffentlich“ eingestuft";

fn public_network_notice(names: &[String]) -> String {
    format!(
        "{PUBLIC_NETWORK_NOTICE} ({}): eingehende Direkt- und LAN-Verbindungen sperrt die Windows-Firewall, andere Geräte erreichen dieses Gerät nur über das Relay (langsamer, Zeitüberschreitungen möglich). Abhilfe: in den Windows-Einstellungen unter Netzwerk und Internet dieses Netz auf „Privates Netzwerk“ stellen",
        names.join(", ")
    )
}

/// Names of the connected networks Windows classifies as public; empty when
/// none is or the query fails (no claim without evidence).
fn public_networks() -> Vec<String> {
    use std::os::windows::process::CommandExt;

    let Ok(engine) = crate::net::system_powershell() else {
        return Vec::new();
    };
    let script = "Get-NetConnectionProfile -ErrorAction SilentlyContinue \
        | Where-Object { $_.NetworkCategory -eq 'Public' } \
        | ForEach-Object { $_.Name }";
    let Ok(output) = std::process::Command::new(engine)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| name.chars().take(80).collect())
        .take(4)
        .collect()
}

pub fn ensure_firewall_rule_for(exe: &std::path::Path) -> io::Result<String> {
    use std::os::windows::process::CommandExt;

    let exe = exe.to_string_lossy().to_string();
    // Deleting and re-adding needs administrator rights; a rule that already
    // allows this program (set once through the UAC repair) is kept as is.
    if rule_allows(&exe) == Some(true) {
        return Ok(format!("Firewall-Regel aktiv: {FIREWALL_RULE}"));
    }
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

/// Whether the rule named `FIREWALL_RULE` allows `exe`: the `Program` of its
/// application filter (Get-NetFirewallRule | Get-NetFirewallApplicationFilter,
/// NetSecurity module). `None` when the query itself fails.
fn rule_allows(exe: &str) -> Option<bool> {
    use std::os::windows::process::CommandExt;

    let engine = crate::net::system_powershell().ok()?;
    let script = format!(
        "(Get-NetFirewallRule -DisplayName '{FIREWALL_RULE}' -ErrorAction SilentlyContinue \
         | Get-NetFirewallApplicationFilter).Program"
    );
    let output = std::process::Command::new(engine)
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case(exe)),
    )
}

pub fn request_firewall_rule_elevated(exe: &str) -> io::Result<()> {
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
