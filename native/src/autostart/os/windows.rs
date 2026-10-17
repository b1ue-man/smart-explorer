const RUN_VALUE: &str = "SmartExplorerSync";
/// Run value of the drive restore (DRV-M3, strand S2 C2).
const DRIVES_RUN_VALUE: &str = "SmartExplorerDrives";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn daemon_exe_for(exe: std::path::PathBuf) -> std::path::PathBuf {
    exe
}

fn daemon_exe() -> std::io::Result<std::path::PathBuf> {
    std::env::current_exe().map(daemon_exe_for)
}

fn logon_exe_for(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let name = exe.file_name().and_then(|name| name.to_str())?;
    if !name.eq_ignore_ascii_case("se.exe") && !name.eq_ignore_ascii_case("se") {
        return Some(exe.to_path_buf());
    }
    ["smart_explorer.exe", "Smart Explorer.exe"]
        .into_iter()
        .map(|name| exe.with_file_name(name))
        .find(|candidate| candidate.is_file())
}

fn run_command_for(exe: &std::path::Path) -> String {
    format!(
        "\"{}\" --sync-daemon",
        exe.to_string_lossy().replace('/', "\\")
    )
}

fn drive_run_command_for(exe: &std::path::Path) -> String {
    format!(
        "\"{}\" --sync-daemon --drive-restore",
        exe.to_string_lossy().replace('/', "\\")
    )
}

fn write_run_entry(exe: &std::path::Path) -> std::io::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let exe = logon_exe_for(exe).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Windows logon autostart requires the GUI executable beside se.exe",
        )
    })?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run, _) = hkcu.create_subkey(RUN_KEY)?;
    run.set_value(RUN_VALUE, &run_command_for(&exe))
}

fn refresh_enabled_entry(exe: &std::path::Path) -> std::io::Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    use winreg::RegKey;
    let Some(exe) = logon_exe_for(exe) else {
        // `se.exe` is a console binary. Keeping a prior GUI Run entry is safer
        // than replacing it with a command that opens a persistent console at
        // logon when no matching GUI executable is available.
        return Ok(());
    };
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = match hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_SET_VALUE) {
        Ok(run) => run,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    match run.get_value::<String, _>(RUN_VALUE) {
        Ok(value) if !value.is_empty() => run.set_value(RUN_VALUE, &run_command_for(&exe)),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn daemon_command(
    exe: &std::path::Path,
    handoff_generation: Option<&str>,
    retiring_generation: Option<&str>,
) -> std::process::Command {
    use std::os::windows::process::CommandExt;

    let mut command = std::process::Command::new(exe);
    command
        .arg("--sync-daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    if let Some(generation) = handoff_generation {
        command.env(super::DAEMON_HANDOFF_ENV, generation);
    } else {
        command.env_remove(super::DAEMON_HANDOFF_ENV);
    }
    if let Some(generation) = retiring_generation {
        command.env(super::DAEMON_RETIRING_GENERATION_ENV, generation);
    } else {
        command.env_remove(super::DAEMON_RETIRING_GENERATION_ENV);
    }
    command
}

fn spawn_daemon(
    exe: &std::path::Path,
    handoff_generation: Option<&str>,
    retiring_generation: Option<&str>,
) -> std::io::Result<()> {
    clear_standard_handle_inheritance()?;
    daemon_command(exe, handoff_generation, retiring_generation)
        .spawn()
        .map(|_| ())
}

/// A terminal caller may itself have inherited stdout/stderr pipe handles from
/// an automation host. Rust's Windows process launcher inherits every handle
/// whose `HANDLE_FLAG_INHERIT` bit remains set, even when the child's selected
/// standard streams are redirected to NUL. Clear that bit before launching the
/// long-lived daemon so an exited `se` process cannot leave its caller waiting
/// forever for pipe EOF.
fn clear_standard_handle_inheritance() -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    for handle in [
        std::io::stdin().as_raw_handle(),
        std::io::stdout().as_raw_handle(),
        std::io::stderr().as_raw_handle(),
    ] {
        clear_handle_inheritance(handle)?;
    }
    Ok(())
}

fn clear_handle_inheritance(handle: std::os::windows::io::RawHandle) -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::{
        SetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    };

    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Ok(());
    }
    let changed = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
    if changed == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
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

/// The logon entry exists, but Windows was told to skip it (Task Manager or
/// Settings "Autostart", `StartupApproved\Run`); the settings show it.
pub fn disabled_by_system() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, REG_BINARY};
    let key = winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");
    let Ok(value) = key.and_then(|key| key.get_raw_value(RUN_VALUE)) else {
        return false;
    };
    if value.vtype != REG_BINARY || value.bytes.len() < 4 {
        return false;
    }
    let state = u32::from_le_bytes([
        value.bytes[0],
        value.bytes[1],
        value.bytes[2],
        value.bytes[3],
    ]);
    // Known Task Manager disable states only; do not guess newer encodings,
    // or modify Windows' approval metadata when refreshing our Run value.
    matches!(state, 3 | 7)
}

/// Register the daemon to start at every logon.
pub fn enable() -> std::io::Result<()> {
    write_run_entry(&daemon_exe()?)
}

/// Remove the autostart entry. A running Share session worker may remain alive,
/// but the daemon observes this setting and cancels scheduled sync work.
pub fn disable() -> std::io::Result<()> {
    delete_run_value(RUN_VALUE)
}

fn delete_run_value(name: &str) -> std::io::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_ALL_ACCESS) {
        Ok(run) => match run.delete_value(name) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// The Run value of the drive restore exists (DRV-M3).
pub fn drive_restore_registered() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|run| run.get_value::<String, _>(DRIVES_RUN_VALUE))
        .map(|value| !value.is_empty())
        .unwrap_or(false)
}

/// Creates the Run value of the drive restore or brings its program path up
/// to date. Windows keeps its own switch apart (`StartupApproved\Run`), so a
/// value switched off in the Task Manager stays off.
pub fn register_drive_restore() -> std::io::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let exe = logon_exe_for(&daemon_exe()?).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Windows logon autostart requires the GUI executable beside se.exe",
        )
    })?;
    if !exe.is_file() {
        // An update renamed the running program; the worker of the new
        // program writes the value at its start.
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "the running program was replaced; the drive Run value stays unchanged",
        ));
    }
    let command = drive_run_command_for(&exe);
    let (run, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if run
        .get_value::<String, _>(DRIVES_RUN_VALUE)
        .is_ok_and(|current| current == command)
    {
        return Ok(());
    }
    run.set_value(DRIVES_RUN_VALUE, &command)
}

pub fn unregister_drive_restore() -> std::io::Result<()> {
    delete_run_value(DRIVES_RUN_VALUE)
}

/// Spawn the background daemon now (detached) so the user doesn't have to log
/// out and back in. The single-instance mutex makes a duplicate launch a no-op.
pub fn spawn_daemon_now() {
    if let Ok(exe) = daemon_exe() {
        let _ = refresh_enabled_entry(&exe);
        let _ = spawn_daemon(&exe, None, None);
    }
}

/// Spawn a replacement daemon and report executable-resolution or launch
/// failures. The handoff marker tells the child to wait for the retiring
/// instance instead of treating the singleton as an ordinary duplicate launch.
pub fn spawn_daemon_handoff_checked(
    generation: &str,
    retiring_generation: Option<&str>,
) -> std::io::Result<()> {
    let exe = daemon_exe()?;
    // A stale/unwritable optional login entry must not prevent an explicit
    // terminal or GUI request from restoring the live worker.
    let _ = refresh_enabled_entry(&exe);
    spawn_daemon(&exe, Some(generation), retiring_generation)
}

#[cfg(test)]
mod tests {
    use std::os::windows::io::AsRawHandle;

    #[test]
    fn se_uses_its_own_executable_even_with_a_gui_sibling() {
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

        let se = dir.join("se.exe");
        let resolved = super::daemon_exe_for(se.clone());
        assert_eq!(resolved, se);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn handoff_command_marks_only_replacement_children() {
        let exe = std::path::Path::new(r"C:\Program Files\Smart Explorer\se.exe");
        let handoff = super::daemon_command(
            exe,
            Some("0123456789abcdef0123456789abcdef"),
            Some("fedcba9876543210fedcba9876543210"),
        );
        assert_eq!(handoff.get_program(), exe.as_os_str());
        assert_eq!(
            handoff.get_args().collect::<Vec<_>>(),
            vec![std::ffi::OsStr::new("--sync-daemon")]
        );
        assert_eq!(
            handoff
                .get_envs()
                .find(|(name, _)| *name == std::ffi::OsStr::new(super::super::DAEMON_HANDOFF_ENV))
                .map(|(_, value)| value),
            Some(Some(std::ffi::OsStr::new(
                "0123456789abcdef0123456789abcdef"
            )))
        );
        assert_eq!(
            handoff
                .get_envs()
                .find(|(name, _)| {
                    *name == std::ffi::OsStr::new(super::super::DAEMON_RETIRING_GENERATION_ENV)
                })
                .map(|(_, value)| value),
            Some(Some(std::ffi::OsStr::new(
                "fedcba9876543210fedcba9876543210"
            )))
        );

        let ordinary = super::daemon_command(exe, None, None);
        assert_eq!(
            ordinary
                .get_envs()
                .find(|(name, _)| *name == std::ffi::OsStr::new(super::super::DAEMON_HANDOFF_ENV))
                .map(|(_, value)| value),
            Some(None)
        );
    }

    #[test]
    fn daemon_spawn_guard_clears_handle_inheritance() {
        use windows_sys::Win32::Foundation::{
            GetHandleInformation, SetHandleInformation, HANDLE_FLAG_INHERIT,
        };

        let file = tempfile::tempfile().unwrap();
        let handle = file.as_raw_handle();
        assert_ne!(
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) },
            0
        );

        super::clear_handle_inheritance(handle).unwrap();
        let mut flags = 0;
        assert_ne!(unsafe { GetHandleInformation(handle, &mut flags) }, 0);
        assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
    }

    #[test]
    fn logon_entry_uses_gui_sibling_instead_of_console_se() {
        let dir = std::env::temp_dir().join(format!(
            "smart_explorer_logon_exe_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let gui = dir.join("smart_explorer.exe");
        std::fs::write(&gui, b"").unwrap();

        assert_eq!(super::logon_exe_for(&dir.join("se.exe")), Some(gui));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn standalone_console_se_is_not_a_logon_target() {
        assert_eq!(
            super::logon_exe_for(std::path::Path::new(r"C:\Tools\se.exe")),
            None
        );
    }

    #[test]
    fn run_entry_quotes_the_gui_executable() {
        assert_eq!(
            super::run_command_for(std::path::Path::new(
                r"C:\Program Files\Smart Explorer\smart_explorer.exe"
            )),
            r#""C:\Program Files\Smart Explorer\smart_explorer.exe" --sync-daemon"#
        );
    }

    #[test]
    fn drive_run_entry_starts_the_restore_with_the_gui_executable() {
        assert_eq!(
            super::drive_run_command_for(std::path::Path::new(
                r"C:\Program Files\Smart Explorer\Smart Explorer.exe"
            )),
            r#""C:\Program Files\Smart Explorer\Smart Explorer.exe" --sync-daemon --drive-restore"#
        );
    }
}
