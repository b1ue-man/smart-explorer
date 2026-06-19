use std::io;
use std::path::PathBuf;

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
    format!("'{}'", arg.replace('\'', "'\\''"))
}

pub fn is_enabled() -> bool {
    desktop_file_path().exists()
}

pub fn enable() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let dir = autostart_dir();
    std::fs::create_dir_all(&dir)?;
    let exec = format!("{} --sync-daemon", quote_exec_arg(&exe.to_string_lossy()));
    let contents = format!(
        "[Desktop Entry]\nType=Application\nName=Smart Explorer Sync Daemon\nComment=Start Smart Explorer background sync at login\nExec={exec}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    );
    std::fs::write(desktop_file_path(), contents)
}

pub fn disable() -> io::Result<()> {
    match std::fs::remove_file(desktop_file_path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn spawn_daemon_now() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--sync-daemon").spawn();
    }
}
