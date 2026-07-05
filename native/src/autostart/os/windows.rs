const RUN_VALUE: &str = "SmartExplorerSync";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn daemon_exe_for(exe: std::path::PathBuf) -> std::path::PathBuf {
    let Some(name) = exe.file_name().and_then(|n| n.to_str()) else {
        return exe;
    };
    if !name.eq_ignore_ascii_case("se.exe") && !name.eq_ignore_ascii_case("se") {
        return exe;
    }
    for candidate in ["smart_explorer.exe", "Smart Explorer.exe"] {
        let sibling = exe.with_file_name(candidate);
        if sibling.exists() {
            return sibling;
        }
    }
    exe.with_file_name("smart_explorer.exe")
}

fn daemon_exe() -> std::io::Result<std::path::PathBuf> {
    std::env::current_exe().map(daemon_exe_for)
}

fn exe_path() -> String {
    daemon_exe()
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
    if let Ok(exe) = daemon_exe() {
        let _ = std::process::Command::new(exe).arg("--sync-daemon").spawn();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn se_uses_sibling_gui_executable_for_daemon() {
        let dir = std::env::temp_dir().join(format!(
            "smart_explorer_autostart_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let gui = dir.join("smart_explorer.exe");
        std::fs::write(&gui, b"").unwrap();

        let resolved = super::daemon_exe_for(dir.join("se.exe"));
        assert_eq!(resolved, gui);

        std::fs::remove_dir_all(dir).ok();
    }
}
