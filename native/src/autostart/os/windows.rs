const RUN_VALUE: &str = "SmartExplorerSync";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().replace('/', "\\"))
        .unwrap_or_default()
}

/// Is the daemon registered to start at logon?
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
pub fn spawn_daemon_now() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--sync-daemon").spawn();
    }
}
