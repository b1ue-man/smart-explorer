use std::io;
use std::path::{Path, PathBuf};

const DESKTOP_FILE: &str = "smart-explorer-sync-daemon.desktop";

fn autostart_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        })
        .join("autostart")
}

fn desktop_file_path() -> PathBuf {
    autostart_dir().join(DESKTOP_FILE)
}

fn quote_exec_arg(arg: &str) -> String {
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    for character in arg.chars() {
        match character {
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(character);
            }
            '\\' => quoted.push_str("\\\\\\\\"),
            '%' => quoted.push_str("%%"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn quote_exec_path(exe: &Path) -> io::Result<String> {
    let executable = exe.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "autostart executable path is not valid UTF-8",
        )
    })?;
    if executable.chars().any(char::is_control) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "autostart executable path contains a control character",
        ));
    }
    Ok(quote_exec_arg(executable))
}

fn daemon_exe_for(exe: PathBuf) -> PathBuf {
    exe
}

fn daemon_exe() -> io::Result<PathBuf> {
    std::env::current_exe().map(daemon_exe_for)
}

pub fn is_enabled() -> bool {
    desktop_file_path().exists()
}

pub fn enable() -> io::Result<()> {
    let exe = daemon_exe()?;
    write_entry(&desktop_file_path(), &exe)
}

fn write_entry(path: &Path, exe: &Path) -> io::Result<()> {
    let exec = format!("{} --sync-daemon", quote_exec_path(exe)?);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = format!(
        "[Desktop Entry]\nType=Application\nName=Smart Explorer Sync Daemon\nComment=Start Smart Explorer background sync at login\nExec={exec}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    );
    std::fs::write(path, contents)
}

fn refresh_enabled_entry(path: &Path, exe: &Path) -> io::Result<()> {
    if path.exists() {
        write_entry(path, exe)?;
    }
    Ok(())
}

pub fn disable() -> io::Result<()> {
    match std::fs::remove_file(desktop_file_path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn spawn_daemon_now() {
    if let Ok(exe) = daemon_exe() {
        let _ = refresh_enabled_entry(&desktop_file_path(), &exe);
        let _ = std::process::Command::new(exe)
            .arg("--sync-daemon")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn desktop_entry_bytes_escape_exec_path_for_both_layers() {
        let dir = std::env::temp_dir().join(format!(
            "smart_explorer_autostart_escaping_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let entry = dir.join("daemon.desktop");
        let executable = std::path::Path::new("/tmp/Smart Explorer/a\"b`c$d\\e%f;g#h'i");

        super::write_entry(&entry, executable).unwrap();
        assert_eq!(
            std::fs::read(&entry).unwrap(),
            concat!(
                "[Desktop Entry]\n",
                "Type=Application\n",
                "Name=Smart Explorer Sync Daemon\n",
                "Comment=Start Smart Explorer background sync at login\n",
                "Exec=\"/tmp/Smart Explorer/a\\\\\"b\\\\`c\\\\$d\\\\\\\\e%%f;g#h'i\" --sync-daemon\n",
                "Terminal=false\n",
                "X-GNOME-Autostart-enabled=true\n",
            )
            .as_bytes()
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn control_character_executable_path_is_rejected() {
        let dir = std::env::temp_dir().join(format!(
            "smart_explorer_autostart_control_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let entry = dir.join("daemon.desktop");

        let error =
            super::write_entry(&entry, std::path::Path::new("/tmp/se\nExec=oops")).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!entry.exists());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn non_utf8_executable_path_is_rejected() {
        use std::os::unix::ffi::OsStringExt;

        let dir = std::env::temp_dir().join(format!(
            "smart_explorer_autostart_non_utf8_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let entry = dir.join("daemon.desktop");
        let executable =
            std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/se-\x80".to_vec()));

        let error = super::write_entry(&entry, &executable).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!entry.exists());

        std::fs::remove_dir_all(dir).ok();
    }

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
        let gui = dir.join("smart_explorer");
        std::fs::write(&gui, b"").unwrap();

        let se = dir.join("se");
        let resolved = super::daemon_exe_for(se.clone());
        assert_eq!(resolved, se);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn enabled_entry_is_refreshed_to_the_current_executable() {
        let dir = std::env::temp_dir().join(format!(
            "smart_explorer_autostart_missing_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("daemon.desktop");
        let se = dir.join("se");
        std::fs::write(&entry, "Exec=/stale/smart_explorer --sync-daemon\n").unwrap();

        super::refresh_enabled_entry(&entry, &se).unwrap();
        let refreshed = std::fs::read_to_string(&entry).unwrap();
        assert!(refreshed.contains(&format!(
            "Exec={} --sync-daemon",
            super::quote_exec_arg(&se.to_string_lossy())
        )));
        assert!(!refreshed.contains("/stale/smart_explorer"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn disabled_entry_is_not_created_during_spawn_refresh() {
        let dir = std::env::temp_dir().join(format!(
            "smart_explorer_autostart_disabled_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let entry = dir.join("daemon.desktop");
        let se = dir.join("se");

        super::refresh_enabled_entry(&entry, &se).unwrap();
        assert!(!entry.exists());

        std::fs::remove_dir_all(dir).ok();
    }
}
