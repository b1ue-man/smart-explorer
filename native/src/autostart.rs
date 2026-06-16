//! Per-user logon autostart for the background sync daemon (#4). On Windows it
//! writes a single value under HKCU\…\Run that launches this exe with
//! `--sync-daemon` — no admin, fully reversible (delete the value). Because it
//! points at the *installed exe*, self-update keeps the daemon current.
//!
//! Non-Windows builds get no-op stubs so the GUI code stays platform-free.

#[cfg(windows)]
const RUN_VALUE: &str = "SmartExplorerSync";
#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
fn exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().replace('/', "\\"))
        .unwrap_or_default()
}

/// Is the daemon registered to start at logon?
#[cfg(windows)]
pub fn is_enabled() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(RUN_KEY)
        .and_then(|run| run.get_value::<String, _>(RUN_VALUE))
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Register the daemon to start at every logon.
#[cfg(windows)]
pub fn enable() -> std::io::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run, _) = hkcu.create_subkey(RUN_KEY)?;
    let cmd = format!("\"{}\" --sync-daemon", exe_path());
    run.set_value(RUN_VALUE, &cmd)
}

/// Remove the autostart entry (a running daemon keeps going until logoff; the
/// caller signals it to stop via `daemon::request_stop`).
#[cfg(windows)]
pub fn disable() -> std::io::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_ALL_ACCESS) {
        Ok(run) => match run.delete_value(RUN_VALUE) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Spawn the background daemon now (detached) so the user doesn't have to log
/// out and back in. The single-instance mutex makes a duplicate launch a no-op.
#[cfg(windows)]
pub fn spawn_daemon_now() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--sync-daemon").spawn();
    }
}

#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    false
}
#[cfg(not(windows))]
pub fn enable() -> std::io::Result<()> {
    Ok(())
}
#[cfg(not(windows))]
pub fn disable() -> std::io::Result<()> {
    Ok(())
}
#[cfg(not(windows))]
pub fn spawn_daemon_now() {}
